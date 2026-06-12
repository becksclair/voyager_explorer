//! Full-width waveform strip with click-to-seek, playhead, hover cursor,
//! cached sync-position markers, and a time axis.

use eframe::egui;

use crate::audio::{WavReader, WaveformChannel};
use crate::ui::theme;
use crate::utils::format_duration;

/// Waveform strip with a cached min/max envelope. The envelope scan walks the
/// entire channel buffer, which for the record rips is hundreds of millions
/// of samples — recomputing it per frame during playback (continuous
/// repaints) would freeze the UI. The cache is keyed by channel buffer
/// identity and pixel width and rebuilt only when those change.
#[derive(Default)]
pub struct WaveformPanel {
    /// (channel buffer address, buffer length, pixel width) of the cached envelope
    envelope_key: Option<(usize, usize, u32)>,
    /// Per-pixel-column (min, max) amplitude
    envelope: Vec<(f32, f32)>,
}

impl WaveformPanel {
    /// Drop the cached envelope. Must be called when a new file is loaded:
    /// the cache key uses buffer address + length, and the allocator can hand
    /// a new same-length file the same address (ABA), which would silently
    /// display the previous file's envelope.
    pub fn invalidate(&mut self) {
        self.envelope_key = None;
        self.envelope.clear();
    }

    /// Draw the waveform strip. Returns a new sample position when the user
    /// clicks or drags to seek. `sync_positions` are precomputed on file
    /// load (never per frame) and rendered as amber tick markers.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        wav_reader: &Option<WavReader>,
        selected_channel: WaveformChannel,
        current_position_samples: usize,
        hover_position: &mut Option<f32>,
        sync_positions: &[usize],
    ) -> Option<usize> {
        let mut seek_to = None;

        let Some(reader) = wav_reader else {
            let (rect, _) = ui.allocate_exact_size(
                egui::Vec2::new(ui.available_width(), ui.available_height().max(60.0)),
                egui::Sense::hover(),
            );
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, theme::WELL);
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, theme::PANEL_BORDER),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No audio loaded — open a WAV file to begin",
                egui::FontId::proportional(13.0),
                theme::TEXT_MUTED,
            );
            return None;
        };

        let samples = reader.get_samples(selected_channel);
        let available_width = ui.available_width();
        let available_height = ui.available_height().max(60.0);

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
            seek_to = Some(new_position.clamp(0, samples_len.saturating_sub(1)));
        }

        if response.hovered() {
            if let Some(hover_pos) = response.hover_pos() {
                let relative_x = (hover_pos.x - rect.min.x) / rect.width();
                *hover_position = Some(relative_x.clamp(0.0, 1.0));
            }
        } else {
            *hover_position = None;
        }

        self.draw_internal(
            ui,
            &rect,
            samples,
            reader.sample_rate,
            current_position_samples,
            *hover_position,
            sync_positions,
        );

        seek_to
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_internal(
        &mut self,
        ui: &mut egui::Ui,
        rect: &egui::Rect,
        samples: &[f32],
        sample_rate: u32,
        current_position: usize,
        hover_position: Option<f32>,
        sync_positions: &[usize],
    ) {
        if !ui.is_rect_visible(*rect) {
            return;
        }
        let painter = ui.painter();

        // Recessed well background with subtle border
        painter.rect_filled(*rect, 4.0, theme::WELL);
        painter.rect_stroke(
            *rect,
            4.0,
            egui::Stroke::new(1.0, theme::PANEL_BORDER),
            egui::StrokeKind::Inside,
        );

        // Handle empty samples gracefully
        if samples.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No audio data",
                egui::FontId::proportional(13.0),
                theme::TEXT_MUTED,
            );
            return;
        }

        // Reserve a band at the bottom for the time axis labels
        let axis_height = 16.0;
        let wave_rect = egui::Rect::from_min_max(
            rect.min,
            egui::Pos2::new(rect.max.x, (rect.max.y - axis_height).max(rect.min.y)),
        );

        // Faint zero-amplitude center line
        let center_y = wave_rect.center().y;
        painter.line_segment(
            [
                egui::Pos2::new(wave_rect.min.x, center_y),
                egui::Pos2::new(wave_rect.max.x, center_y),
            ],
            egui::Stroke::new(1.0, theme::PANEL_BORDER),
        );

        // Min/max envelope per pixel column, drawn in cyan. Rebuilt only when
        // the channel buffer or pixel width changes — never per frame.
        let width_px = wave_rect.width() as u32;
        let key = (samples.as_ptr() as usize, samples.len(), width_px);
        if self.envelope_key != Some(key) {
            let samples_per_pixel = samples.len().max(1) as f32 / width_px.max(1) as f32;
            self.envelope.clear();
            self.envelope.reserve(width_px as usize);
            for pixel_x in 0..width_px {
                let start_sample = (pixel_x as f32 * samples_per_pixel) as usize;
                let end_sample = (((pixel_x + 1) as f32 * samples_per_pixel) as usize).min(samples.len());
                let mut min_val = 1.0f32;
                let mut max_val = -1.0f32;
                if start_sample < samples.len() {
                    for sample in &samples[start_sample..end_sample] {
                        let clamped = sample.clamp(-1.0, 1.0);
                        min_val = min_val.min(clamped);
                        max_val = max_val.max(clamped);
                    }
                }
                self.envelope.push((min_val, max_val));
            }
            self.envelope_key = Some(key);
        }

        let trace_color = theme::CYAN.gamma_multiply(0.85);
        let amplitude_scale = wave_rect.height() * 0.42;
        for (pixel_x, &(min_val, max_val)) in self.envelope.iter().enumerate() {
            if max_val < min_val {
                continue; // column had no samples
            }
            let min_y = center_y - min_val * amplitude_scale;
            let max_y = center_y - max_val * amplitude_scale;
            let x = wave_rect.min.x + pixel_x as f32;

            painter.line_segment(
                [egui::Pos2::new(x, min_y), egui::Pos2::new(x, max_y)],
                egui::Stroke::new(1.0, trace_color),
            );
        }

        // Amber tick markers at detected sync-tone positions (cached on load)
        let sync_color = theme::AMBER.gamma_multiply(0.75);
        for &sync_pos in sync_positions {
            if sync_pos < samples.len() {
                let x = wave_rect.min.x + (sync_pos as f32 / samples.len() as f32) * wave_rect.width();
                painter.line_segment(
                    [
                        egui::Pos2::new(x, wave_rect.min.y),
                        egui::Pos2::new(x, wave_rect.min.y + 10.0),
                    ],
                    egui::Stroke::new(2.0, theme::AMBER),
                );
                painter.line_segment(
                    [
                        egui::Pos2::new(x, wave_rect.min.y + 10.0),
                        egui::Pos2::new(x, wave_rect.max.y),
                    ],
                    egui::Stroke::new(1.0, sync_color.gamma_multiply(0.35)),
                );
            }
        }

        // Playhead cursor in teal
        if current_position < samples.len() {
            let position_x = wave_rect.min.x + (current_position as f32 / samples.len() as f32) * wave_rect.width();
            painter.line_segment(
                [
                    egui::Pos2::new(position_x, wave_rect.min.y),
                    egui::Pos2::new(position_x, wave_rect.max.y),
                ],
                egui::Stroke::new(2.0, theme::ACCENT),
            );
        }

        // Hover cursor
        if let Some(hover_x) = hover_position {
            let hover_pixel_x = wave_rect.min.x + hover_x * wave_rect.width();
            painter.line_segment(
                [
                    egui::Pos2::new(hover_pixel_x, wave_rect.min.y),
                    egui::Pos2::new(hover_pixel_x, wave_rect.max.y),
                ],
                egui::Stroke::new(1.0, theme::TEXT_MUTED),
            );
        }

        // Time axis labels along the bottom
        let total_secs = samples.len() as f32 / sample_rate.max(1) as f32;
        let label_font = egui::FontId::monospace(10.0);
        let ticks = 5;
        for i in 0..=ticks {
            let frac = i as f32 / ticks as f32;
            let x = wave_rect.min.x + frac * wave_rect.width();
            let text = format_duration(total_secs * frac);
            let align = if i == 0 {
                egui::Align2::LEFT_BOTTOM
            } else if i == ticks {
                egui::Align2::RIGHT_BOTTOM
            } else {
                egui::Align2::CENTER_BOTTOM
            };
            painter.text(
                egui::Pos2::new(x, rect.max.y - 2.0),
                align,
                text,
                label_font.clone(),
                theme::TEXT_MUTED,
            );
        }
    }
}
