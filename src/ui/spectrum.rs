use crate::audio::WavReader;
use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

pub struct SpectrumPanel {
    pub visible: bool,
    pub use_log_scale: bool,
    pub use_db_scale: bool,
    pub show_peak: bool,
}

impl Default for SpectrumPanel {
    fn default() -> Self {
        Self {
            visible: false,
            use_log_scale: true,
            use_db_scale: true,
            show_peak: true,
        }
    }
}

fn to_db(magnitude: f64) -> f64 {
    if magnitude > 0.00001 {
        20.0 * magnitude.log10()
    } else {
        -100.0
    }
}

impl SpectrumPanel {
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        wav_reader: &Option<WavReader>,
        current_position_samples: usize,
        selected_channel: crate::audio::WaveformChannel,
    ) {
        ui.heading("Signal Analysis (Spectrum)");

        // Controls
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.use_log_scale, "Log Freq");
            ui.checkbox(&mut self.use_db_scale, "dB Scale");
            ui.checkbox(&mut self.show_peak, "Show Peak");
        });

        ui.separator();

        if let Some(reader) = wav_reader {
            let samples = reader.get_samples(selected_channel);

            // Use a window around current position
            let window_size = 4096;
            let start = current_position_samples;
            let end = (start + window_size).min(samples.len());

            if start < end {
                let window_samples = &samples[start..end];
                let spectrum =
                    crate::analysis::compute_spectrum(window_samples, reader.sample_rate);

                // Find peak if enabled
                let mut peak: Option<(f64, f64)> = None;

                // First pass to find peak (on linear/dB data before log transform)
                for (f, m) in &spectrum {
                    let mag = if self.use_db_scale { to_db(*m) } else { *m };

                    match peak {
                        None => peak = Some((*f, mag)),
                        Some((_, max_mag)) => {
                            if mag > max_mag {
                                peak = Some((*f, mag));
                            }
                        }
                    }
                }

                if self.show_peak {
                    if let Some((peak_freq, peak_mag)) = peak {
                        ui.label(format!(
                            "Peak: {:.1} Hz ({:.1} {})",
                            peak_freq,
                            peak_mag,
                            if self.use_db_scale { "dB" } else { "mag" }
                        ));
                    } else {
                        ui.label("Peak: N/A");
                    }
                }

                // Generate points for plotting
                let points: PlotPoints = spectrum
                    .iter()
                    .filter_map(|(f, m)| {
                        let mut freq = *f;
                        let mut mag = *m;

                        if self.use_log_scale {
                            if freq <= 0.0 {
                                return None;
                            }
                            freq = freq.log10();
                        }

                        if self.use_db_scale {
                            mag = to_db(mag);
                        }

                        Some([freq, mag])
                    })
                    .collect();

                let line =
                    Line::new("Magnitude", points).color(egui::Color32::from_rgb(100, 255, 100));

                let mut plot = Plot::new("spectrum_plot")
                    .view_aspect(2.0)
                    .x_axis_label(if self.use_log_scale {
                        "Frequency (Log Hz)"
                    } else {
                        "Frequency (Hz)"
                    })
                    .y_axis_label(if self.use_db_scale {
                        "Magnitude (dB)"
                    } else {
                        "Magnitude"
                    });

                if self.use_log_scale {
                    plot = plot.x_axis_formatter(|mark, _range| format_log_freq(mark.value));
                }

                plot.show(ui, |plot_ui| {
                    plot_ui.line(line);
                });
            } else {
                ui.label("No samples available at current position");
            }
        } else {
            ui.label("No audio loaded");
        }
    }
}

fn format_log_freq(mark_value: f64) -> String {
    let value = 10.0_f64.powf(mark_value);
    if value >= 1000.0 {
        format!("{:.1}k", value / 1000.0)
    } else {
        format!("{:.0}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_log_freq() {
        // 100 Hz -> log10(100) = 2.0
        assert_eq!(format_log_freq(2.0), "100");

        // 1000 Hz -> log10(1000) = 3.0
        assert_eq!(format_log_freq(3.0), "1.0k");

        // 1500 Hz -> log10(1500) = 3.176...
        // 10^3.176 = 1500. 1.5k
        let val = 1500.0f64.log10();
        assert_eq!(format_log_freq(val), "1.5k");

        // 10 kHz -> 4.0
        assert_eq!(format_log_freq(4.0), "10.0k");
    }
}
