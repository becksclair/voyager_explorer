use eframe::egui;

pub enum ControlAction {
    TogglePlayback,
    StopPlayback,
    SeekToNextSync,
}

pub struct ControlsPanel;

impl ControlsPanel {
    pub fn draw(ui: &mut egui::Ui, is_playing: bool) -> Option<ControlAction> {
        let mut action = None;

        ui.horizontal(|ui| {
            let play_button_text = if is_playing { "⏸ Pause" } else { "▶ Play" };
            if ui.button(play_button_text).clicked() {
                action = Some(ControlAction::TogglePlayback);
            }

            if ui.button("⏹ Stop").clicked() {
                action = Some(ControlAction::StopPlayback);
            }

            if ui.button("⏭ Skip to Next Sync").clicked() {
                action = Some(ControlAction::SeekToNextSync);
            }
        });

        action
    }
}
