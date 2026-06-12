//! Dark "mission console" theme shared by all UI panels.
//!
//! Centralizes the color palette and egui style overrides so every panel
//! draws from the same vocabulary: near-black background, slightly lighter
//! panels with subtle borders, muted labels with bright values, and a small
//! set of accent colors (teal for active/positive, amber for sync, cyan for
//! signal traces, red for errors only).

use eframe::egui;
use egui::{Color32, CornerRadius, Margin, Stroke};

/// Application background (near-black).
pub const BG: Color32 = Color32::from_rgb(0x0E, 0x11, 0x16);
/// Panel surface, slightly lighter than the background.
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x1A, 0x21);
/// Subtle 1px panel border.
pub const PANEL_BORDER: Color32 = Color32::from_rgb(0x26, 0x2D, 0x36);
/// Recessed surfaces (plots, waveform strip, text edits).
pub const WELL: Color32 = Color32::from_rgb(0x0A, 0x0D, 0x11);
/// Muted gray for labels and secondary text.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8B, 0x94, 0x9E);
/// Bright text for values and numerals.
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xE6, 0xED, 0xF3);
/// Teal/green accent: active and positive states, live indicators.
pub const ACCENT: Color32 = Color32::from_rgb(0x2D, 0xD4, 0xA0);
/// Amber accent: sync markers and sync-related actions.
pub const AMBER: Color32 = Color32::from_rgb(0xE8, 0xA3, 0x3D);
/// Cyan: spectrum and waveform signal traces.
pub const CYAN: Color32 = Color32::from_rgb(0x4F, 0xC3, 0xE8);
/// Red: errors only.
pub const ERROR: Color32 = Color32::from_rgb(0xF8, 0x51, 0x49);

/// Apply the mission-console style to the egui context. Call once at startup.
pub fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, PANEL_BORDER);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.extreme_bg_color = WELL;
    visuals.faint_bg_color = Color32::from_rgb(0x1A, 0x20, 0x28);
    visuals.hyperlink_color = CYAN;
    visuals.error_fg_color = ERROR;
    visuals.warn_fg_color = AMBER;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.slider_trailing_fill = true;

    let corner = CornerRadius::same(6);
    let widgets = &mut visuals.widgets;

    widgets.noninteractive.corner_radius = corner;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.weak_bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, PANEL_BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);

    widgets.inactive.corner_radius = corner;
    widgets.inactive.bg_fill = Color32::from_rgb(0x1C, 0x23, 0x2C);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(0x1C, 0x23, 0x2C);
    widgets.inactive.bg_stroke = Stroke::new(1.0, PANEL_BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);

    widgets.hovered.corner_radius = corner;
    widgets.hovered.bg_fill = Color32::from_rgb(0x24, 0x2C, 0x37);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0x24, 0x2C, 0x37);
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3A, 0x44, 0x50));
    widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT_BRIGHT);

    widgets.active.corner_radius = corner;
    widgets.active.bg_fill = Color32::from_rgb(0x2A, 0x34, 0x40);
    widgets.active.weak_bg_fill = Color32::from_rgb(0x2A, 0x34, 0x40);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(1.5, TEXT_BRIGHT);

    widgets.open.corner_radius = corner;
    widgets.open.bg_fill = Color32::from_rgb(0x1C, 0x23, 0x2C);
    widgets.open.weak_bg_fill = Color32::from_rgb(0x1C, 0x23, 0x2C);
    widgets.open.bg_stroke = Stroke::new(1.0, PANEL_BORDER);
    widgets.open.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.slider_width = 120.0;

    ctx.set_style(style);
}

/// Standard bordered panel frame used by every console section.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, PANEL_BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(10))
}

/// Frame for the outer chrome strips (header, transport, status bar).
pub fn strip_frame() -> egui::Frame {
    egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(14, 8))
}

/// Small uppercase section label in muted gray.
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).color(TEXT_MUTED).strong());
}

/// Muted "key" label followed by a bright "value", on one row.
pub fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).size(12.0).color(TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).size(12.0).color(TEXT_BRIGHT).monospace());
        });
    });
}
