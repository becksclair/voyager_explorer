//! Transport bar: file open, playback controls, timecode readout, sync skip.

use eframe::egui;

use crate::ui::theme;
use crate::utils::format_timecode;

pub enum ControlAction {
    OpenWav,
    TogglePlayback,
    StopPlayback,
    SeekToNextSync,
}

pub struct ControlsPanel;

impl ControlsPanel {
    /// Draw the transport strip. `current_secs`/`total_secs` feed the
    /// monospace timecode readout; playback buttons are disabled until a
    /// file is loaded.
    pub fn draw(
        ui: &mut egui::Ui,
        is_playing: bool,
        has_audio: bool,
        current_secs: f64,
        total_secs: f64,
    ) -> Option<ControlAction> {
        let mut action = None;

        ui.horizontal(|ui| {
            if ui.button("Open WAV…").clicked() {
                action = Some(ControlAction::OpenWav);
            }

            ui.separator();

            let play_text = if is_playing {
                egui::RichText::new("⏸ Pause").color(theme::ACCENT)
            } else {
                egui::RichText::new("▶ Play")
            };
            if ui.add_enabled(has_audio, egui::Button::new(play_text)).clicked() {
                action = Some(ControlAction::TogglePlayback);
            }

            if ui.add_enabled(has_audio, egui::Button::new("⏹ Stop")).clicked() {
                action = Some(ControlAction::StopPlayback);
            }

            ui.separator();

            ui.label(
                egui::RichText::new(format!("{} / {}", format_timecode(current_secs), format_timecode(total_secs)))
                    .monospace()
                    .size(15.0)
                    .color(theme::TEXT_BRIGHT),
            );

            ui.separator();

            let skip_text = egui::RichText::new("⏭ Skip to Next Sync").color(theme::AMBER);
            if ui.add_enabled(has_audio, egui::Button::new(skip_text)).clicked() {
                action = Some(ControlAction::SeekToNextSync);
            }
        });

        action
    }
}
