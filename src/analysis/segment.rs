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
}

impl Default for SegmentImagesParams {
    fn default() -> Self {
        Self {
            sync: SyncParams::default(),
            gap_factor: 1.5,
            min_lines: 200,
            expected_lines: 600,
            filter_tones: true,
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
    let mut runs: Vec<&[usize]> = Vec::new();
    let mut run_start = 0usize;
    for i in 1..syncs.len() {
        if (syncs[i] - syncs[i - 1]) as f64 > break_threshold {
            runs.push(&syncs[run_start..i]);
            run_start = i;
        }
    }
    runs.push(&syncs[run_start..]);

    runs.into_iter()
        .filter(|run| run.len() >= params.min_lines.max(2))
        .filter(|run| !params.filter_tones || !is_mostly_tone(samples, sample_rate, run))
        .map(|run| {
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
