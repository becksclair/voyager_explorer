//! Diagnostics CLI: scriptable analysis and decode commands for iterating on
//! the decoder against real record audio without the GUI.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::analysis::{
    classify_segments, compute_stats, detect_line_syncs, find_image_bounds, interval_summary, rolling_stats, ClassifyParams,
    SegmentImagesParams, SignalStats, SpectrogramParams, SyncParams,
};
use crate::audio::{WavReader, WaveformChannel};
use crate::pipeline::DecodingPipeline;
use crate::sstv::{DecoderMode, DecoderParams};

#[derive(Subcommand)]
pub enum DiagnosticsCommand {
    /// Decode a time window of a WAV file to a PNG image
    Decode {
        #[arg(short, long)]
        input: PathBuf,
        /// Start offset in seconds
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        /// Window length in seconds (omit to decode to end of file)
        #[arg(short, long)]
        duration: Option<f64>,
        /// Output PNG path
        #[arg(short, long)]
        out: PathBuf,
        /// Channel for stereo files
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
        /// Image width in pixels
        #[arg(long, default_value_t = 512)]
        width: u32,
        /// Scan line duration in milliseconds
        #[arg(long, default_value_t = 8.32)]
        line_ms: f32,
        /// Invert brightness polarity (rip-dependent)
        #[arg(long, default_value_t = false)]
        invert: bool,
        /// Gamma applied after normalization
        #[arg(long, default_value_t = 1.0)]
        gamma: f32,
        /// Disable per-line sync alignment (fixed-period slicing instead)
        #[arg(long, default_value_t = false)]
        no_sync_lock: bool,
        /// Decoder mode
        #[arg(long, value_enum, default_value_t = CliMode::Grayscale)]
        mode: CliMode,
        /// Rotate output 90° clockwise (Voyager lines are vertical scans)
        #[arg(long, default_value_t = false)]
        rotate: bool,
        /// Mirror the output horizontally (scan direction correction)
        #[arg(long, default_value_t = false)]
        flip: bool,
    },

    /// Render a spectrogram PNG of a time window, with frequency markers
    Spectrogram {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: Option<f64>,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
        /// Upper frequency bound to display, Hz
        #[arg(long)]
        fmax: Option<f32>,
        /// Draw horizontal marker lines at these frequencies (Hz, repeatable)
        #[arg(long = "mark-freq", default_values_t = vec![1200.0])]
        mark_freqs: Vec<f32>,
        /// FFT window size in samples
        #[arg(long, default_value_t = 1024)]
        fft_size: usize,
        /// Output plot width in pixels
        #[arg(long, default_value_t = 1600)]
        plot_width: u32,
    },

    /// Detect scan-line sync positions and print interval statistics
    Syncs {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: Option<f64>,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
        /// Expected line duration in milliseconds
        #[arg(long, default_value_t = 8.32)]
        line_ms: f32,
        /// Peak threshold as a fraction of the robust maximum
        #[arg(long, default_value_t = 0.45)]
        peak_height: f32,
        /// Print every position instead of only the summary
        #[arg(long, default_value_t = false)]
        verbose: bool,
    },

    /// Classify a file into silence / tone / image-periodic / broadband segments
    Classify {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: Option<f64>,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
    },

    /// Print signal statistics for a time window
    Stats {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: Option<f64>,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
        /// Also print rolling stats with this window length in seconds
        #[arg(long)]
        rolling: Option<f64>,
    },

    /// Find per-image boundaries from sync-cadence breaks; optionally decode
    /// each detected image to a PNG
    Segment {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: Option<f64>,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
        /// Expected line duration in milliseconds
        #[arg(long, default_value_t = 8.32)]
        line_ms: f32,
        /// Cadence-break threshold as a multiple of the median sync interval
        #[arg(long, default_value_t = 1.5)]
        gap_factor: f32,
        /// Minimum scan lines for a run to count as an image
        #[arg(long, default_value_t = 200)]
        min_lines: usize,
        /// Nominal lines per image (used for the confidence column)
        #[arg(long, default_value_t = 600)]
        expected_lines: usize,
        /// Keep runs that classify as steady tone (lead-in/calibration tone)
        #[arg(long, default_value_t = false)]
        keep_tones: bool,
        /// Decode each detected image to PNG files in this directory
        #[arg(long)]
        decode_dir: Option<PathBuf>,
        /// With --decode-dir: name files from the reference catalog and
        /// composite R/G/B frame triplets into color images (requires the
        /// candidate count to match the published 78 frames per channel)
        #[arg(long, default_value_t = false)]
        color: bool,
        /// Image width in pixels (decode)
        #[arg(long, default_value_t = 512)]
        width: u32,
        /// Invert brightness polarity (decode, rip-dependent)
        #[arg(long, default_value_t = false)]
        invert: bool,
        /// Gamma applied after normalization (decode)
        #[arg(long, default_value_t = 1.0)]
        gamma: f32,
        /// Rotate output 90° clockwise (decode)
        #[arg(long, default_value_t = false)]
        rotate: bool,
        /// Mirror the output horizontally (decode)
        #[arg(long, default_value_t = false)]
        flip: bool,
    },

    /// Cut a time window out of a WAV file into a new (mono) WAV file
    Carve {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long, default_value_t = 0.0)]
        start: f64,
        #[arg(short, long)]
        duration: f64,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(short, long, value_enum, default_value_t = ChannelArg::Left)]
        channel: ChannelArg,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum ChannelArg {
    Left,
    Right,
}

impl From<ChannelArg> for WaveformChannel {
    fn from(val: ChannelArg) -> Self {
        match val {
            ChannelArg::Left => WaveformChannel::Left,
            ChannelArg::Right => WaveformChannel::Right,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum CliMode {
    Grayscale,
    PseudoColor,
}

impl From<CliMode> for DecoderMode {
    fn from(val: CliMode) -> Self {
        match val {
            CliMode::Grayscale => DecoderMode::Grayscale,
            CliMode::PseudoColor => DecoderMode::PseudoColor,
        }
    }
}

/// Load the requested window/channel of a WAV file. Returns the shared
/// channel buffer without copying it.
fn load_window(input: &PathBuf, start: f64, duration: Option<f64>, channel: ChannelArg) -> Result<(Arc<[f32]>, u32)> {
    let reader = match duration {
        Some(d) => WavReader::from_file_range(input, start, d),
        None if start > 0.0 => WavReader::from_file_start(input, start),
        None => WavReader::from_file(input),
    }
    .with_context(|| format!("loading {}", input.display()))?;
    let samples = match WaveformChannel::from(channel) {
        WaveformChannel::Left => Arc::clone(&reader.left_channel),
        WaveformChannel::Right => Arc::clone(&reader.right_channel),
    };
    Ok((samples, reader.sample_rate))
}

pub fn run(command: DiagnosticsCommand) -> Result<()> {
    match command {
        DiagnosticsCommand::Decode {
            input,
            start,
            duration,
            out,
            channel,
            width,
            line_ms,
            invert,
            gamma,
            no_sync_lock,
            mode,
            rotate,
            flip,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let params = DecoderParams {
                line_duration_ms: line_ms,
                invert,
                gamma,
                sync_lock: !no_sync_lock,
                mode: mode.into(),
                width,
                ..DecoderParams::default()
            };
            let result = DecodingPipeline::new()
                .process(&samples, &params, sample_rate)
                .context("decode failed")?;
            let mut img = result.to_dynamic_image().context("building image")?;
            if rotate {
                img = img.rotate90();
            }
            if flip {
                img = img.fliph();
            }
            img.save(&out).with_context(|| format!("writing {}", out.display()))?;
            println!(
                "decoded {}x{} ({} lines) from {start:.3}s -> {}",
                img.width(),
                img.height(),
                result.height,
                out.display()
            );
        }

        DiagnosticsCommand::Spectrogram {
            input,
            start,
            duration,
            out,
            channel,
            fmax,
            mark_freqs,
            fft_size,
            plot_width,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let params = SpectrogramParams {
                fft_size,
                hop: (fft_size / 4).max(1),
                fmax,
            };
            let spec = crate::analysis::compute_spectrogram(&samples, sample_rate, &params);
            anyhow::ensure!(!spec.frames.is_empty(), "window too short for FFT size {fft_size}");
            let img = crate::analysis::render_spectrogram(&spec, start, &mark_freqs, plot_width);
            img.save(&out).with_context(|| format!("writing {}", out.display()))?;
            println!(
                "spectrogram {} frames x {} bins ({:.1} Hz/bin, {:.2} ms/frame) -> {}",
                spec.frames.len(),
                spec.bins,
                spec.freq_step,
                spec.time_step * 1000.0,
                out.display()
            );
        }

        DiagnosticsCommand::Syncs {
            input,
            start,
            duration,
            channel,
            line_ms,
            peak_height,
            verbose,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let params = SyncParams {
                expected_line_ms: line_ms,
                peak_height,
                ..SyncParams::default()
            };
            let positions = detect_line_syncs(&samples, sample_rate, &params);
            println!("{} sync positions detected", positions.len());
            if verbose {
                println!("{:>12} {:>12} {:>10}", "sample", "abs_secs", "delta");
                let mut prev: Option<usize> = None;
                for &p in &positions {
                    let delta = prev.map(|q| format!("{}", p - q)).unwrap_or_else(|| "-".into());
                    println!("{:>12} {:>12.4} {:>10}", p, start + p as f64 / sample_rate as f64, delta);
                    prev = Some(p);
                }
            }
            if let Some(summary) = interval_summary(&positions, sample_rate) {
                println!(
                    "intervals: n={} median={:.1} samples ({:.3} ms) mean={:.1} std={:.1} min={} max={}",
                    summary.count,
                    summary.median_samples,
                    summary.median_ms,
                    summary.mean_samples,
                    summary.std_samples,
                    summary.min_samples,
                    summary.max_samples,
                );
                // Coarse histogram around the median to expose drift/double-trigger modes
                let median = summary.median_samples;
                let mut buckets = [0usize; 7];
                for w in positions.windows(2) {
                    let iv = (w[1] - w[0]) as f64;
                    let rel = (iv - median) / median;
                    let idx = if rel < -0.5 {
                        0
                    } else if rel < -0.05 {
                        1
                    } else if rel < -0.01 {
                        2
                    } else if rel <= 0.01 {
                        3
                    } else if rel <= 0.05 {
                        4
                    } else if rel <= 0.5 {
                        5
                    } else {
                        6
                    };
                    buckets[idx] += 1;
                }
                let labels = ["<-50%", "-50..-5%", "-5..-1%", "±1%", "+1..5%", "+5..50%", ">+50%"];
                println!("interval histogram (relative to median):");
                for (label, count) in labels.iter().zip(buckets.iter()) {
                    println!("  {label:>9}: {count}");
                }
            }
        }

        DiagnosticsCommand::Classify {
            input,
            start,
            duration,
            channel,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let segments = classify_segments(&samples, sample_rate, &ClassifyParams::default());
            println!(
                "{:>10} {:>10} {:>16} {:>6} {:>10}",
                "start_s", "end_s", "label", "conf", "period_ms"
            );
            for seg in segments {
                println!(
                    "{:>10.3} {:>10.3} {:>16} {:>6.2} {:>10}",
                    start + seg.start_secs,
                    start + seg.end_secs,
                    seg.label.to_string(),
                    seg.confidence,
                    seg.period_ms.map(|p| format!("{p:.3}")).unwrap_or_else(|| "-".into()),
                );
            }
        }

        DiagnosticsCommand::Stats {
            input,
            start,
            duration,
            channel,
            rolling,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let stats = compute_stats(&samples, sample_rate);
            println!(
                "samples={} rate={} duration={:.3}s",
                samples.len(),
                sample_rate,
                samples.len() as f64 / sample_rate as f64
            );
            print_stats_row(None, &stats);
            if let Some(window_secs) = rolling {
                println!("rolling ({window_secs}s windows):");
                for (t, row) in rolling_stats(&samples, sample_rate, window_secs) {
                    print_stats_row(Some(start + t), &row);
                }
            }
        }

        DiagnosticsCommand::Segment {
            input,
            start,
            duration,
            channel,
            line_ms,
            gap_factor,
            min_lines,
            expected_lines,
            keep_tones,
            decode_dir,
            color,
            width,
            invert,
            gamma,
            rotate,
            flip,
        } => {
            let (samples, sample_rate) = load_window(&input, start, duration, channel)?;
            let params = SegmentImagesParams {
                sync: SyncParams {
                    expected_line_ms: line_ms,
                    ..SyncParams::default()
                },
                gap_factor,
                min_lines,
                expected_lines,
                filter_tones: !keep_tones,
                ..SegmentImagesParams::default()
            };
            let bounds = find_image_bounds(&samples, sample_rate, &params);
            println!("{} image candidates", bounds.len());
            println!(
                "{:>4} {:>10} {:>10} {:>8} {:>7} {:>10} {:>6}",
                "idx", "start_s", "end_s", "dur_s", "lines", "line_ms", "conf"
            );
            for (idx, b) in bounds.iter().enumerate() {
                println!(
                    "{idx:>4} {:>10.3} {:>10.3} {:>8.3} {:>7} {:>10.3} {:>6.2}",
                    start + b.start_secs,
                    start + b.end_secs,
                    b.end_secs - b.start_secs,
                    b.line_count,
                    b.median_interval_samples / sample_rate as f64 * 1000.0,
                    b.confidence,
                );
            }

            if let Some(dir) = decode_dir {
                std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
                let decode_params = DecoderParams {
                    line_duration_ms: line_ms,
                    invert,
                    gamma,
                    sync_lock: true,
                    mode: DecoderMode::Grayscale,
                    width,
                    ..DecoderParams::default()
                };

                let catalog = if color {
                    if bounds.len() == crate::catalog::FRAMES_PER_CHANNEL {
                        Some(crate::catalog::channel_catalog(channel.into()))
                    } else {
                        tracing::warn!(
                            "--color: {} candidates != {} catalog frames; falling back to plain naming",
                            bounds.len(),
                            crate::catalog::FRAMES_PER_CHANNEL
                        );
                        None
                    }
                } else {
                    None
                };

                let orient = |mut img: image::DynamicImage| {
                    if rotate {
                        img = img.rotate90();
                    }
                    if flip {
                        img = img.fliph();
                    }
                    img
                };

                // Triplet members keep their raw levels for the joint-bounds
                // composite pass, so each frame is decoded exactly once.
                let triplets = match catalog {
                    Some(_) => crate::catalog::color_triplets(channel.into()),
                    None => Vec::new(),
                };
                let member_idxs: std::collections::HashSet<usize> = triplets.iter().flatten().copied().collect();

                let decoder = crate::sstv::SstvDecoder::new();
                let plane_width = decode_params.effective_width();
                let mut member_levels: std::collections::HashMap<usize, Vec<f32>> = std::collections::HashMap::new();
                for (idx, b) in bounds.iter().enumerate() {
                    let window = &samples[b.start_sample..b.end_sample];
                    let levels = match decoder.decode_levels(window, &decode_params, sample_rate) {
                        Ok(levels) => levels,
                        Err(e) => {
                            tracing::warn!("image {idx} at {:.3}s failed to decode: {e:#}", start + b.start_secs);
                            continue;
                        }
                    };
                    // The standalone PNG keeps per-frame contrast bounds.
                    let (lo, hi) = crate::sstv::percentile_bounds(&levels, 0.01, 0.99);
                    let frame = crate::pipeline::PipelineResult {
                        pixels: crate::sstv::normalize_levels(&levels, lo, hi, invert, gamma),
                        width: plane_width as u32,
                        height: (levels.len() / plane_width) as u32,
                        mode: DecoderMode::Grayscale,
                    };
                    if member_idxs.contains(&idx) {
                        member_levels.insert(idx, levels);
                    }
                    let img = orient(frame.to_dynamic_image().context("building image")?);
                    let name = match catalog {
                        Some(cat) => format!("image_{idx:03}_{}.png", slugify(cat[idx].label)),
                        None => format!("image_{idx:03}_{:.3}s.png", start + b.start_secs),
                    };
                    let path = dir.join(name);
                    img.save(&path).with_context(|| format!("writing {}", path.display()))?;
                    println!("  [{idx:03}] {} lines -> {}", frame.height, path.display());
                }

                if let Some(cat) = catalog {
                    for triplet in triplets {
                        let [r, _, bl] = triplet;
                        let planes: Vec<&[f32]> = triplet
                            .iter()
                            .filter_map(|idx| member_levels.get(idx).map(Vec::as_slice))
                            .collect();
                        let [pr, pg, pb] = planes.as_slice() else {
                            tracing::warn!("triplet {r}-{bl}: missing decoded frame, skipping composite");
                            continue;
                        };
                        let img = match crate::pipeline::composite_triplet_levels([pr, pg, pb], plane_width, invert, gamma) {
                            Ok(img) => orient(img),
                            Err(e) => {
                                tracing::warn!("triplet {r}-{bl}: composite failed: {e:#}");
                                continue;
                            }
                        };
                        let (first, last) = (triplet.iter().min().unwrap(), triplet.iter().max().unwrap());
                        let path = dir.join(format!("color_{first:03}-{last:03}_{}.png", slugify(cat[*first].label)));
                        img.save(&path).with_context(|| format!("writing {}", path.display()))?;
                        println!("  [color {first:03}-{last:03}] -> {}", path.display());
                    }
                }
            }
        }

        DiagnosticsCommand::Carve {
            input,
            start,
            duration,
            out,
            channel,
        } => {
            let (samples, sample_rate) = load_window(&input, start, Some(duration), channel)?;
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::create(&out, spec).with_context(|| format!("creating {}", out.display()))?;
            for &s in samples.iter() {
                writer.write_sample(s)?;
            }
            writer.finalize()?;
            println!(
                "carved {:.3}s ({} samples @ {} Hz) from {start:.3}s -> {}",
                samples.len() as f64 / sample_rate as f64,
                samples.len(),
                sample_rate,
                out.display()
            );
        }
    }
    Ok(())
}

/// File-name slug from a catalog label's title (the part before the credit).
fn slugify(label: &str) -> String {
    let title = label.split(',').next().unwrap_or(label);
    let mut slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    slug.trim_matches('_').chars().take(48).collect()
}

fn print_stats_row(t: Option<f64>, s: &SignalStats) {
    let prefix = t.map(|t| format!("t={t:>9.3}s ")).unwrap_or_default();
    println!(
        "{prefix}rms={:.4} peak={:.4} dc={:+.5} zcr={:.0}/s crest={:.1}dB dominant={:.1}Hz",
        s.rms, s.peak, s.dc_offset, s.zero_crossing_rate, s.crest_db, s.dominant_freq_hz
    );
}
