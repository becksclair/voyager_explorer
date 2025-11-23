use crate::audio::{WavReader, WaveformChannel};
use crate::utils::format_duration;
use eframe::egui;

pub struct WaveformPanel;

impl WaveformPanel {
    pub fn draw(
        ui: &mut egui::Ui,
        wav_reader: &Option<WavReader>,
        selected_channel: WaveformChannel,
        current_position_samples: usize,
        hover_position: &mut Option<f32>,
    ) -> Option<usize> {
        let mut seek_to = None;

        ui.heading("Audio Waveform");
        ui.separator();

        // Position display
        if let Some(reader) = wav_reader {
            ui.horizontal(|ui| {
                let duration_secs = reader.left_channel.len() as f32 / reader.sample_rate as f32;
                let current_secs = current_position_samples as f32 / reader.sample_rate as f32;
                ui.label(format!(
                    "Position: {} / {}",
                    format_duration(current_secs),
                    format_duration(duration_secs)
                ));
            });
        }

        ui.separator();

        // Waveform visualization
        if let Some(reader) = wav_reader {
            let samples = reader.get_samples(selected_channel);
            let available_width = ui.available_width();
            let available_height = ui.available_height().min(150.0);

            let response = ui.allocate_response(
                egui::Vec2::new(available_width, available_height),
                egui::Sense::click_and_drag(),
            );
            let rect = response.rect;

            if response.clicked() || response.dragged() {
                let click_pos = response.interact_pointer_pos().unwrap_or_default();
                let relative_x = (click_pos.x - rect.min.x) / rect.width();
                let samples_len = samples.len();
                let new_position = (relative_x * samples_len as f32) as usize;
                seek_to = Some(new_position.clamp(0, samples_len));
            }

            if response.hovered() {
                if let Some(hover_pos) = response.hover_pos() {
                    let relative_x = (hover_pos.x - rect.min.x) / rect.width();
                    *hover_position = Some(relative_x.clamp(0.0, 1.0));
                }
            } else {
                *hover_position = None;
            }

            Self::draw_internal(
                ui,
                &rect,
                samples,
                current_position_samples,
                *hover_position,
            );
        } else {
            ui.label("No audio loaded");
        }

        seek_to
    }

    fn draw_internal(
        ui: &mut egui::Ui,
        rect: &egui::Rect,
        samples: &[f32],
        current_position: usize,
        hover_position: Option<f32>,
    ) {
        if ui.is_rect_visible(*rect) {
            let painter = ui.painter();

            // Background
            painter.rect_filled(*rect, 0.0, egui::Color32::from_gray(20));

            // Handle empty samples gracefully
            if samples.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No audio data",
                    egui::FontId::default(),
                    egui::Color32::GRAY,
                );
                return;
            }

            // Draw waveform
            let samples_per_pixel = samples.len().max(1) as f32 / rect.width();

            for pixel_x in 0..rect.width() as i32 {
                let start_sample = (pixel_x as f32 * samples_per_pixel) as usize;
                let end_sample =
                    (((pixel_x + 1) as f32 * samples_per_pixel) as usize).min(samples.len());

                if start_sample < samples.len() {
                    // Find min/max in this pixel range for better visualization
                    let mut min_val = 1.0f32;
                    let mut max_val = -1.0f32;

                    for sample_idx in start_sample..end_sample {
                        if sample_idx < samples.len() {
                            let sample = samples[sample_idx];
                            // Clamp sample values to prevent rendering issues
                            let clamped_sample = sample.clamp(-1.0, 1.0);
                            min_val = min_val.min(clamped_sample);
                            max_val = max_val.max(clamped_sample);
                        }
                    }

                    let center_y = rect.center().y;
                    let amplitude_scale = rect.height() * 0.4; // Use 40% of height for amplitude

                    let min_y = center_y - min_val * amplitude_scale;
                    let max_y = center_y - max_val * amplitude_scale;

                    let x = rect.min.x + pixel_x as f32;

                    // Draw vertical line from min to max
                    painter.line_segment(
                        [egui::Pos2::new(x, min_y), egui::Pos2::new(x, max_y)],
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)),
                    );
                }
            }

            // Draw current position indicator (only if position is valid)
            if current_position < samples.len() {
                let position_x =
                    rect.min.x + (current_position as f32 / samples.len() as f32) * rect.width();
                painter.line_segment(
                    [
                        egui::Pos2::new(position_x, rect.min.y),
                        egui::Pos2::new(position_x, rect.max.y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::RED),
                );
            }

            // Draw hover line
            if let Some(hover_x) = hover_position {
                let hover_pixel_x = rect.min.x + hover_x * rect.width();
                painter.line_segment(
                    [
                        egui::Pos2::new(hover_pixel_x, rect.min.y),
                        egui::Pos2::new(hover_pixel_x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::YELLOW),
                );
            }
        }
    }
}
