#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;
use tracing_subscriber::{fmt, EnvFilter};

mod app;
use app::VoyagerApp;

pub mod analysis;
pub mod audio;
pub mod audio_state;
pub mod batch;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod error;
pub mod image_output;
pub mod metrics;
pub mod pipeline;
pub mod services;
pub mod sstv;
pub mod test_fixtures;
pub mod ui;
pub mod utils;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::sstv::DecoderMode;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Load this WAV file on startup (GUI mode)
    #[arg(long, global = false)]
    load: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run batch processing on multiple files
    Batch {
        /// Input glob pattern (e.g. "assets/*.wav")
        #[arg(short, long)]
        input: String,

        /// Output directory
        #[arg(short, long)]
        output: PathBuf,

        /// Decoder mode (Grayscale or PseudoColor)
        #[arg(short, long, value_enum, default_value_t = ModeArg::Grayscale)]
        mode: ModeArg,
    },

    /// Diagnostics: decode windows, spectrograms, sync detection, stats
    #[command(flatten)]
    Diagnostics(cli::DiagnosticsCommand),
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ModeArg {
    Grayscale,
    PseudoColor,
}

impl From<ModeArg> for DecoderMode {
    fn from(val: ModeArg) -> Self {
        match val {
            ModeArg::Grayscale => DecoderMode::Grayscale,
            ModeArg::PseudoColor => DecoderMode::PseudoColor,
        }
    }
}

fn main() -> eframe::Result {
    // Initialize tracing subscriber
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("voyager_explorer=info")))
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Batch { input, output, mode }) => {
            let args = batch::BatchArgs {
                input_pattern: input,
                output_dir: output,
                mode: mode.into(),
            };

            if let Err(e) = batch::run_batch_processing(args) {
                tracing::error!("Batch processing failed: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(Commands::Diagnostics(command)) => {
            if let Err(e) = cli::run(command) {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
            return Ok(());
        }
        None => {}
    }

    tracing::info!("Starting Voyager Golden Record Explorer");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Voyager Golden Record Explorer",
        options,
        Box::new(move |cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);
            // Apply the mission-console dark theme once at startup
            ui::theme::apply_theme(&cc.egui_ctx);
            tracing::info!("UI context initialized");
            let mut app = VoyagerApp::default();
            if let Some(path) = cli.load {
                app.load_wav_from_path(&path);
            }
            Ok(Box::new(app))
        }),
    )
}
