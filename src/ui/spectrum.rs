use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use crate::audio::WavReader;

#[derive(Default)]
pub struct SpectrumPanel {
    pub visible: bool,
}

impl SpectrumPanel {
    pub fn draw(&self, ui: &mut egui::Ui, wav_reader: &Option<WavReader>, current_position_samples: usize, selected_channel: crate::audio::WaveformChannel) {
        ui.heading("Signal Analysis (Spectrum)");
        ui.separator();

        if let Some(reader) = wav_reader {
            let samples = reader.get_samples(selected_channel);

            // Use a window around current position
            let window_size = 4096;
            let start = current_position_samples;
            let end = (start + window_size).min(samples.len());

            if start < end {
                let window_samples = &samples[start..end];
                let spectrum = crate::analysis::compute_spectrum(window_samples, reader.sample_rate);

                let points = PlotPoints::from_iter(spectrum.iter().map(|(f, m)| [*f, *m]));
                let line = Line::new("Magnitude", points)
                    .color(egui::Color32::from_rgb(100, 255, 100));

                Plot::new("spectrum_plot")
                    .view_aspect(2.0)
                    .x_axis_label("Frequency (Hz)")
                    .y_axis_label("Magnitude")
                    .show(ui, |plot_ui| {
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
