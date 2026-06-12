//! Live spectrum analyzer around the current playhead position.

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

use crate::audio::WavReader;
use crate::ui::theme;

pub struct SpectrumPanel {
    pub visible: bool,
    pub use_log_scale: bool,
    pub use_db_scale: bool,
    pub show_peak: bool,
    /// (buffer address, buffer length, window start) of the cached spectrum.
    /// The FFT (and its planner) only rerun when the analysis window moves —
    /// not on every repaint, which during playback happens at frame rate.
    spectrum_key: Option<(usize, usize, usize)>,
    spectrum: Vec<(f64, f64)>,
}

impl Default for SpectrumPanel {
    fn default() -> Self {
        Self {
            visible: true,
            use_log_scale: true,
            use_db_scale: true,
            show_peak: true,
            spectrum_key: None,
            spectrum: Vec::new(),
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
    /// Drop the cached spectrum. Must be called when a new file is loaded:
    /// the cache key uses buffer address + length, which a new same-length
    /// allocation can collide with (ABA).
    pub fn invalidate(&mut self) {
        self.spectrum_key = None;
        self.spectrum.clear();
    }

    /// Draw the analyzer plot and scale controls. Returns the dominant
    /// `(frequency_hz, scaled_magnitude)` when peak tracking is enabled and
    /// audio is loaded, so the caller can surface it in the Signal box.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        wav_reader: &Option<WavReader>,
        current_position_samples: usize,
        selected_channel: crate::audio::WaveformChannel,
    ) -> Option<(f64, f64)> {
        // Scale controls
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.use_log_scale, "Log");
            ui.checkbox(&mut self.use_db_scale, "dB");
            ui.checkbox(&mut self.show_peak, "Peak");
        });

        let Some(reader) = wav_reader else {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("No audio loaded").color(theme::TEXT_MUTED));
            return None;
        };

        let samples = reader.get_samples(selected_channel);

        // Use a window around current position
        let window_size = 4096;
        let start = current_position_samples;
        let end = (start + window_size).min(samples.len());

        if start >= end {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("No samples at current position").color(theme::TEXT_MUTED));
            return None;
        }

        let key = (samples.as_ptr() as usize, samples.len(), start);
        if self.spectrum_key != Some(key) {
            let window_samples = &samples[start..end];
            self.spectrum = crate::analysis::compute_spectrum(window_samples, reader.sample_rate);
            self.spectrum_key = Some(key);
        }

        // Generate points for plotting and track peak in a single pass
        let mut peak: Option<(f64, f64)> = None;
        let points: PlotPoints = self
            .spectrum
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

                // Track peak during iteration (reuse already-scaled mag).
                // Skip the DC bin (0 Hz) in both scale modes, matching
                // analysis::stats dominant-frequency selection; otherwise a DC
                // offset can report "0 Hz" as dominant when log scale is off.
                if self.show_peak && *f > 0.0 {
                    match peak {
                        None => peak = Some((*f, mag)),
                        Some((_, max_mag)) => {
                            if mag > max_mag {
                                peak = Some((*f, mag));
                            }
                        }
                    }
                }

                Some([freq, mag])
            })
            .collect();

        let line = Line::new("Magnitude", points).color(theme::CYAN);

        let mut plot = Plot::new("spectrum_plot")
            .height(190.0)
            .show_x(true)
            .show_y(true)
            .y_axis_label(if self.use_db_scale { "dB" } else { "mag" });

        if self.use_log_scale {
            plot = plot.x_axis_formatter(|mark, _range| format_log_freq(mark.value));
        }

        plot.show(ui, |plot_ui| {
            plot_ui.line(line);
        });

        peak
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
