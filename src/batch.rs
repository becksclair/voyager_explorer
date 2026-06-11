use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::audio::WavReader;
use crate::pipeline::DecodingPipeline;
use crate::sstv::{DecoderMode, DecoderParams};

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

fn process_file(input_path: &Path, output_dir: &Path, pipeline: &DecodingPipeline, params: &DecoderParams) -> Result<()> {
    // Load WAV file
    let reader = WavReader::from_file(input_path).context("Failed to load WAV file")?;

    // Use left channel by default for batch processing
    let samples = reader.get_samples(crate::audio::WaveformChannel::Left);

    // Decode
    let result = pipeline
        .process(samples, params, reader.sample_rate)
        .context("Failed to decode audio")?;

    if result.pixels.is_empty() {
        tracing::warn!("No image data decoded for {:?}", input_path);
        return Ok(());
    }

    // Convert to image
    let image_buffer = result.to_dynamic_image().context("Failed to convert pixel data to image")?;

    // Save image
    let file_stem = input_path
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("Invalid filename: {:?} has no stem", input_path))?;

    let output_path = unique_output_path(output_dir, file_stem);

    image_buffer.save(&output_path).context("Failed to save image")?;

    Ok(())
}

/// Pick a non-colliding output path: `<stem>.png`, then `<stem>_1.png`,
/// `<stem>_2.png`, ... so batch runs never silently overwrite earlier
/// outputs (including same-stem inputs from different directories).
fn unique_output_path(output_dir: &Path, file_stem: &std::ffi::OsStr) -> PathBuf {
    let candidate = output_dir.join(PathBuf::from(file_stem).with_extension("png"));
    if !candidate.exists() {
        return candidate;
    }
    let stem = file_stem.to_string_lossy();
    let mut counter = 1u32;
    loop {
        let candidate = output_dir.join(format!("{stem}_{counter}.png"));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

pub fn process_single_file(path: &Path, output_dir: &Path, params: &DecoderParams) -> Result<()> {
    let pipeline = DecodingPipeline::new();
    process_file(path, output_dir, &pipeline, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unique_output_path_avoids_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let stem = std::ffi::OsStr::new("frame");

        let first = unique_output_path(dir.path(), stem);
        assert_eq!(first, dir.path().join("frame.png"));

        fs::write(&first, b"x").unwrap();
        let second = unique_output_path(dir.path(), stem);
        assert_eq!(second, dir.path().join("frame_1.png"));

        fs::write(&second, b"x").unwrap();
        let third = unique_output_path(dir.path(), stem);
        assert_eq!(third, dir.path().join("frame_2.png"));
    }
}
