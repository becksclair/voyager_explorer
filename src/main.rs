#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;
use tracing_subscriber::{fmt, EnvFilter};

mod app;
use app::VoyagerApp;

pub mod analysis;
pub mod batch;
pub mod ui;
pub mod services;
pub mod pipeline;
pub mod audio;
pub mod audio_state;
pub mod config;
pub mod error;
pub mod image_output;
pub mod metrics;
pub mod sstv;
pub mod utils;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use crate::sstv::DecoderMode;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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

        /// Decoder mode (BinaryGrayscale or PseudoColor)
        #[arg(short, long, value_enum, default_value_t = ModeArg::BinaryGrayscale)]
        mode: ModeArg,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ModeArg {
    BinaryGrayscale,
    PseudoColor,
}

impl From<ModeArg> for DecoderMode {
    fn from(val: ModeArg) -> Self {
        match val {
            ModeArg::BinaryGrayscale => DecoderMode::BinaryGrayscale,
            ModeArg::PseudoColor => DecoderMode::PseudoColor,
        }
    }
}

fn main() -> eframe::Result {
    // Initialize tracing subscriber
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

    let cli = Cli::parse();

    if let Some(Commands::Batch { input, output, mode }) = cli.command {
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
