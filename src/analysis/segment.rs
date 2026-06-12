//! Image-boundary segmentation: split a continuous record region into
//! per-image sample ranges using breaks in the scan-line sync cadence.
//!
//! Within an image the sync intervals are rock-steady at the line period
//! (~8.32 ms). Between images the cadence breaks: syncs disappear for a few
//! line periods or land at irregular spacing. A break is declared wherever an
//! inter-sync interval exceeds `gap_factor` times the median interval; runs
//! of consecutive in-cadence lines between breaks become image candidates,
//! and runs shorter than `min_lines` (boundary junk, leaders) are dropped.

use super::classify::{classify_segments, ClassifyParams, SegmentLabel};
use super::sync::{detect_line_syncs, interval_summary, SyncParams};

#[derive(Debug, Clone)]
pub struct SegmentImagesParams {
    pub sync: SyncParams,
    /// An interval larger than this multiple of the median interval marks a
    /// cadence break. Must clear within-image detector glitches (mistimed
    /// peaks stay under ~1.25x) while catching real inter-image gaps (2-4
    /// line periods).
    pub gap_factor: f32,
    /// Minimum scan lines for a run to count as an image candidate.
    pub min_lines: usize,
    /// Nominal lines per image; used only for the reported confidence.
    /// The 48 kHz remaster runs ~550-750 lines per catalog slot.
    pub expected_lines: usize,
    /// Drop runs whose audio classifies mostly as steady tone. The record's
    /// lead-in calibration tone is line-periodic enough to sync-lock, so
    /// cadence alone cannot reject it.
    pub filter_tones: bool,
    /// Repair false splits and fused runs against the median run length
    /// (needs at least [`MIN_RUNS_FOR_CLEANUP`] runs to establish it): two
    /// adjacent short runs whose combined span fits one slot are merged; a
    /// run spanning k slots is split at its weakest cadence points.
    pub cleanup: bool,
    /// Maximum silence between two runs for them to be merge candidates,
    /// in seconds.
    pub merge_gap_secs: f32,
}

/// Minimum number of runs before the median run length is trusted for
/// merge/split cleanup.
pub const MIN_RUNS_FOR_CLEANUP: usize = 5;

impl Default for SegmentImagesParams {
    fn default() -> Self {
        Self {
            sync: SyncParams::default(),
            gap_factor: 1.5,
            min_lines: 200,
            expected_lines: 600,
            filter_tones: true,
            cleanup: true,
            merge_gap_secs: 1.0,
        }
    }
}

/// One detected image region. Sample indices are relative to the analyzed
/// buffer; seconds are derived from them at the analysis sample rate.
#[derive(Debug, Clone)]
pub struct ImageBounds {
    pub start_sample: usize,
    /// Exclusive end: one median line period past the final sync, clamped to
    /// the buffer, so the last scan line is included.
    pub end_sample: usize,
    pub start_secs: f64,
    pub end_secs: f64,
    pub line_count: usize,
    pub median_interval_samples: f64,
    /// How close the run's line count is to `expected_lines`, in [0, 1].
    /// Low values flag truncated images or fused multi-image runs.
    pub confidence: f32,
}

/// Find per-image sample ranges in `samples` from sync-cadence breaks.
///
/// Returns bounds in time order. A fused run (two images with no detectable
/// gap) is reported as a single low-confidence bounds entry rather than
/// guessed apart.
pub fn find_image_bounds(samples: &[f32], sample_rate: u32, params: &SegmentImagesParams) -> Vec<ImageBounds> {
    let syncs = detect_line_syncs(samples, sample_rate, &params.sync);
    let Some(summary) = interval_summary(&syncs, sample_rate) else {
        return Vec::new();
    };
    let break_threshold = summary.median_samples * params.gap_factor as f64;

    // Split the sync sequence into runs at cadence breaks.
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut run_start = 0usize;
    for i in 1..syncs.len() {
        if (syncs[i] - syncs[i - 1]) as f64 > break_threshold {
            runs.push(syncs[run_start..i].to_vec());
            run_start = i;
        }
    }
    runs.push(syncs[run_start..].to_vec());

    runs.retain(|run| run.len() >= params.min_lines.max(2));
    runs.retain(|run| !params.filter_tones || !is_mostly_tone(samples, sample_rate, run));

    if params.cleanup && runs.len() >= MIN_RUNS_FOR_CLEANUP {
        let merge_gap = (params.merge_gap_secs as f64 * sample_rate as f64) as usize;
        runs = merge_false_splits(runs, merge_gap);
        runs = split_fused_runs(runs);
        // Split parts are not re-checked against min_lines by construction;
        // drop anything too short to summarize.
        runs.retain(|run| run.len() >= 2);
    }

    runs.into_iter()
        .map(|run| {
            let run = run.as_slice();
            // Per-run median is more faithful than the global one when tape
            // speed drifts across the file.
            let run_summary = interval_summary(run, sample_rate).expect("run has >= 2 syncs");
            let start_sample = run[0];
            let end_sample = (*run.last().unwrap() + run_summary.median_samples.round() as usize).min(samples.len());
            // Lines = intervals + the final line after the last sync.
            let line_count = run.len();
            let confidence = if params.expected_lines == 0 {
                1.0
            } else {
                let ratio = line_count as f32 / params.expected_lines as f32;
                (1.0 - (ratio - 1.0).abs()).clamp(0.0, 1.0)
            };
            ImageBounds {
                start_sample,
                end_sample,
                start_secs: start_sample as f64 / sample_rate as f64,
                end_secs: end_sample as f64 / sample_rate as f64,
                line_count,
                median_interval_samples: run_summary.median_samples,
                confidence,
            }
        })
        .collect()
}

/// Median run length in lines. Caller guarantees `runs` is non-empty.
fn median_run_lines(runs: &[Vec<usize>]) -> f64 {
    let mut lens: Vec<usize> = runs.iter().map(Vec::len).collect();
    lens.sort_unstable();
    super::sync::median_of_sorted(&lens)
}

/// Merge adjacent short runs that together span one image slot: the cadence
/// detector sometimes splits a single image in half at a weak-sync patch,
/// leaving two sub-slot runs separated by a small gap.
fn merge_false_splits(runs: Vec<Vec<usize>>, merge_gap_samples: usize) -> Vec<Vec<usize>> {
    let slot = median_run_lines(&runs);
    let mut merged: Vec<Vec<usize>> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(prev) = merged.last_mut() {
            let gap = run[0].saturating_sub(*prev.last().unwrap());
            let both_short = (prev.len() as f64) < 0.75 * slot && (run.len() as f64) < 0.75 * slot;
            let combined_fits = ((prev.len() + run.len()) as f64) < 1.3 * slot;
            if both_short && combined_fits && gap < merge_gap_samples {
                prev.extend(run);
                continue;
            }
        }
        merged.push(run);
    }
    merged
}

/// Split runs spanning multiple image slots (images so close together that
/// no cadence break separates them). Each split lands on the run's weakest
/// cadence point near the expected slot boundary.
fn split_fused_runs(runs: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let slot = median_run_lines(&runs);
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(runs.len());
    for run in runs {
        let parts = (run.len() as f64 / slot).round() as usize;
        if parts < 2 || (run.len() as f64) < 1.6 * slot {
            out.push(run);
            continue;
        }
        // Cut before the sync with the largest preceding interval within
        // ±10% of each expected boundary; falls back to a mid-slot cut when
        // the cadence is perfectly steady across the join. Successive
        // search windows can overlap for k >= 3, so cuts are forced strictly
        // forward — otherwise two adjacent cuts could yield an empty part.
        let mut cuts: Vec<usize> = Vec::with_capacity(parts - 1);
        for j in 1..parts {
            let target = j * run.len() / parts;
            let radius = run.len() / 10;
            let lo = target
                .saturating_sub(radius)
                .max(cuts.last().map_or(1, |&c| c + 2))
                .min(run.len() - 1);
            let hi = (target + radius).clamp(lo, run.len() - 1);
            let cut = (lo..=hi)
                .max_by_key(|&i| (run[i] - run[i - 1], std::cmp::Reverse(i.abs_diff(target))))
                .expect("split window is non-empty");
            cuts.push(cut);
        }
        let mut prev = 0usize;
        for cut in cuts {
            out.push(run[prev..cut].to_vec());
            prev = cut;
        }
        out.push(run[prev..].to_vec());
    }
    out
}

/// True when more than half of the run's duration classifies as steady tone.
fn is_mostly_tone(samples: &[f32], sample_rate: u32, run: &[usize]) -> bool {
    let start = run[0];
    let end = (*run.last().unwrap()).min(samples.len());
    if end <= start {
        return false;
    }
    let segments = classify_segments(&samples[start..end], sample_rate, &ClassifyParams::default());
    let total: f64 = segments.iter().map(|s| s.end_secs - s.start_secs).sum();
    let tone: f64 = segments
        .iter()
        .filter(|s| s.label == SegmentLabel::Tone)
        .map(|s| s.end_secs - s.start_secs)
        .sum();
    total > 0.0 && tone / total > 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::encode_image_to_audio;

    const RATE: u32 = 48_000;
    const LINE_MS: f32 = 8.32;
    const WIDTH: usize = 64;

    fn test_image(lines: usize, seed: u8) -> Vec<u8> {
        (0..lines * WIDTH)
            .map(|i| ((i as u32 * 31 + seed as u32 * 7) % 200 + 30) as u8)
            .collect()
    }

    fn params(min_lines: usize, expected_lines: usize) -> SegmentImagesParams {
        SegmentImagesParams {
            min_lines,
            expected_lines,
            ..SegmentImagesParams::default()
        }
    }

    #[test]
    fn merges_a_false_split_back_into_one_image() {
        // Four full images plus one split in half by a 30 ms dropout: the
        // dropout breaks the cadence (>1.5x line period) but the halves
        // together span one slot, so cleanup must rejoin them.
        let gap = vec![0.0f32; (0.05 * RATE as f32) as usize];
        let dropout = vec![0.0f32; (0.03 * RATE as f32) as usize];
        let mut audio = Vec::new();
        for seed in 0..2u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }
        audio.extend(encode_image_to_audio(&test_image(126, 2), WIDTH, RATE, LINE_MS));
        audio.extend(&dropout);
        audio.extend(encode_image_to_audio(&test_image(126, 3), WIDTH, RATE, LINE_MS));
        audio.extend(&gap);
        for seed in 4..6u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 5, "{bounds:?}");
        let rejoined = &bounds[2];
        assert!(
            rejoined.line_count > 230,
            "halves should merge into ~252 lines, got {}",
            rejoined.line_count
        );
    }

    #[test]
    fn splits_a_fused_double_image() {
        // Four full images plus one back-to-back pair with no gap: the pair
        // sync-locks as one 512-line run and must be split near its middle.
        let gap = vec![0.0f32; (0.05 * RATE as f32) as usize];
        let mut audio = Vec::new();
        for seed in 0..2u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }
        audio.extend(encode_image_to_audio(&test_image(256, 2), WIDTH, RATE, LINE_MS));
        audio.extend(encode_image_to_audio(&test_image(256, 3), WIDTH, RATE, LINE_MS));
        audio.extend(&gap);
        for seed in 4..6u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 6, "{bounds:?}");
        for b in &bounds {
            assert!(
                (b.line_count as i64 - 256).abs() <= 30,
                "expected ~256 lines per part, got {} ({b:?})",
                b.line_count
            );
        }
    }

    #[test]
    fn splits_a_five_way_fused_run_without_panicking() {
        // k >= 3 splits have overlapping cut-search windows; this used to be
        // able to produce empty parts and panic downstream. Steady cadence
        // also exercises the proximity tie-break fallback for every cut.
        let gap = vec![0.0f32; (0.05 * RATE as f32) as usize];
        let mut audio = Vec::new();
        for seed in 0..4u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }
        for seed in 4..9u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
        }

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 9, "{bounds:?}");
        for pair in bounds.windows(2) {
            assert!(pair[0].end_sample <= pair[1].start_sample, "{pair:?}");
        }
        for b in &bounds {
            assert!((b.line_count as i64 - 256).abs() <= 35, "{b:?}");
        }
    }

    #[test]
    fn line_periodic_tone_is_filtered() {
        // A sine at exactly the line rate sync-locks (one peak per period)
        // but classifies as tone; the filter must drop it.
        let line_rate = 1000.0 / LINE_MS; // ~120.2 Hz
        let audio = crate::test_fixtures::generate_sine_wave(line_rate, 3.0, RATE, 0.8);
        let with_filter = find_image_bounds(&audio, RATE, &params(100, 256));
        assert!(with_filter.is_empty(), "{with_filter:?}");

        let mut p = params(100, 256);
        p.filter_tones = false;
        let without_filter = find_image_bounds(&audio, RATE, &p);
        assert!(!without_filter.is_empty(), "sine should sync-lock without the filter");
    }

    #[test]
    fn finds_three_images_separated_by_silence() {
        let gap = vec![0.0f32; (0.05 * RATE as f32) as usize]; // 50 ms silence
        let mut audio = Vec::new();
        for seed in 0..3u8 {
            audio.extend(encode_image_to_audio(&test_image(256, seed), WIDTH, RATE, LINE_MS));
            audio.extend(&gap);
        }

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 3, "{bounds:?}");
        for b in &bounds {
            assert!(
                (b.line_count as i64 - 256).abs() <= 3,
                "expected ~256 lines, got {} ({b:?})",
                b.line_count
            );
            assert!(b.confidence > 0.9, "confidence {}", b.confidence);
            assert!((b.median_interval_samples - 399.4).abs() < 3.0);
        }
        // Bounds must be ordered and non-overlapping.
        for pair in bounds.windows(2) {
            assert!(pair[0].end_sample <= pair[1].start_sample, "{pair:?}");
        }
    }

    #[test]
    fn silence_yields_no_bounds() {
        let audio = vec![0.0f32; RATE as usize];
        assert!(find_image_bounds(&audio, RATE, &SegmentImagesParams::default()).is_empty());
    }

    #[test]
    fn short_runs_are_dropped() {
        // One real image plus a 20-line stub: the stub must not survive.
        let gap = vec![0.0f32; (0.05 * RATE as f32) as usize];
        let mut audio = encode_image_to_audio(&test_image(256, 0), WIDTH, RATE, LINE_MS);
        audio.extend(&gap);
        audio.extend(encode_image_to_audio(&test_image(20, 1), WIDTH, RATE, LINE_MS));

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 1, "{bounds:?}");
    }

    #[test]
    fn fused_images_report_low_confidence() {
        // Two images back-to-back with no gap: one run, double line count.
        let mut audio = encode_image_to_audio(&test_image(256, 0), WIDTH, RATE, LINE_MS);
        audio.extend(encode_image_to_audio(&test_image(256, 1), WIDTH, RATE, LINE_MS));

        let bounds = find_image_bounds(&audio, RATE, &params(100, 256));
        assert_eq!(bounds.len(), 1, "{bounds:?}");
        assert!(
            bounds[0].line_count > 450,
            "fused run should keep both images' lines, got {}",
            bounds[0].line_count
        );
        assert!(bounds[0].confidence < 0.5, "confidence {}", bounds[0].confidence);
    }
}
