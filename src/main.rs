#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;
use tracing_subscriber::{fmt, EnvFilter};

mod app;
use app::VoyagerApp;

pub mod audio;
pub mod audio_state;
pub mod config;
pub mod error;
pub mod image_output;
pub mod metrics;
pub mod presets;
pub mod session;
pub mod sstv;
pub mod utils;

fn main() -> eframe::Result {
    // Initialize tracing subscriber with environment filter
    // Use RUST_LOG=debug for detailed logging, RUST_LOG=info for normal operation
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("voyager_explorer=info")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .init();

    tracing::info!("Starting Voyager Golden Record Explorer");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Voyager Golden Record Explorer",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);
            tracing::info!("UI context initialized");
            Ok(Box::<VoyagerApp>::default())
        }),
    )
}
