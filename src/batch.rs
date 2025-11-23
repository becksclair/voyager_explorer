use crate::audio::WavReader;
use crate::pipeline::DecodingPipeline;
use crate::sstv::{DecoderMode, DecoderParams};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::fs;

#[derive(Debug)]
pub struct BatchArgs {
    pub input_pattern: String,
    pub output_dir: PathBuf,
    pub mode: DecoderMode,
}

pub fn run_batch_processing(args: BatchArgs) -> Result<()> {
    tracing::info!("Starting batch processing");
    tracing::info!("Input pattern: {}", args.input_pattern);
    tracing::info!("Output directory: {:?}", args.output_dir);
    tracing::info!("Mode: {:?}", args.mode);

    // Create output directory if it doesn't exist
    fs::create_dir_all(&args.output_dir).context("Failed to create output directory")?;

    // Find files matching the pattern
    let paths: Vec<PathBuf> = glob::glob(&args.input_pattern)
        .context("Failed to read glob pattern")?
        .filter_map(|entry| entry.ok())
        .collect();

    if paths.is_empty() {
        tracing::warn!("No files found matching pattern: {}", args.input_pattern);
        return Ok(());
    }

    tracing::info!("Found {} files to process", paths.len());

    let pipeline = DecodingPipeline::new();
    let params = DecoderParams {
        mode: args.mode,
        ..DecoderParams::default()
    };

    for path in paths {
        tracing::info!("Processing file: {:?}", path);

        match process_file(&path, &args.output_dir, &pipeline, &params) {
            Ok(_) => tracing::info!("Successfully processed {:?}", path),
            Err(e) => tracing::error!("Failed to process {:?}: {}", path, e),
        }
    }

    tracing::info!("Batch processing complete");
    Ok(())
}

fn process_file(
    input_path: &Path,
    output_dir: &Path,
    pipeline: &DecodingPipeline,
    params: &DecoderParams,
) -> Result<()> {
    // Load WAV file
    let reader = WavReader::from_file(input_path).context("Failed to load WAV file")?;

    // Use left channel by default for batch processing
    let samples = reader.get_samples(crate::audio::WaveformChannel::Left);

    // Decode
    let result = pipeline.process(samples, params, reader.sample_rate)
        .context("Failed to decode audio")?;

    if result.pixels.is_empty() {
        tracing::warn!("No image data decoded for {:?}", input_path);
        return Ok(());
    }

    // Convert to image
    let image_buffer = result.to_dynamic_image()
        .context("Failed to convert pixel data to image")?;

    // Save image
    let file_stem = input_path
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("Invalid filename: {:?} has no stem", input_path))?;

    // Ensure stem is not empty to prevent ".png" overwrites
    if file_stem.is_empty() {
        anyhow::bail!("Invalid filename: {:?} has empty stem", input_path);
    }

    let output_filename = PathBuf::from(file_stem).with_extension("png");
    let output_path = output_dir.join(output_filename);

    image_buffer.save(&output_path).context("Failed to save image")?;

    Ok(())
}
