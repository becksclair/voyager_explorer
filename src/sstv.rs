use std::f32::consts::PI;

use realfft::{RealFftPlanner, RealToComplex};

use crate::analysis::sync::{interval_summary, track_line_syncs, SyncParams};
use crate::error::{DecoderError, Result, VoyagerError};

/// Calibration tone frequency in Hz. Long ~1200 Hz tone regions precede image
/// sections on the record; this drives navigation, not per-line decoding.
const TARGET_FREQ_HZ: f32 = 1200.0;
/// FFT chunk size for frequency analysis
const CHUNK_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderMode {
    Grayscale,
    PseudoColor,
}

#[derive(Debug, Clone, Copy)]
pub struct DecoderParams {
    /// Nominal scan-line duration in ms. With `sync_lock` this seeds the sync
    /// search and serves as the fallback slicing period; the actual per-line
    /// timing comes from the detected sync positions.
    pub line_duration_ms: f32,
    /// Invert brightness polarity. The record rips differ in sign relative to
    /// the cover instructions, so this is empirical per source.
    pub invert: bool,
    /// Gamma applied after normalization (1.0 = linear).
    pub gamma: f32,
    /// Align lines to detected sync edges instead of fixed-period slicing.
    pub sync_lock: bool,
    /// Live-decode window length in seconds (used by the decode worker to
    /// slice around the playback position, not by `decode` itself).
    pub decode_window_secs: f64,
    pub mode: DecoderMode,
    /// Image width in pixels. Default is 512, which matches the Voyager spacecraft
    /// imaging system's standard frame width used for transmitting planetary imagery.
    pub width: u32,
}

impl DecoderParams {
    /// Image width as a usable row size, clamped so it can never be a zero
    /// divisor in row/height math. Both the decoder and the pipeline must
    /// use this same value or their row-size contracts diverge.
    pub fn effective_width(&self) -> usize {
        (self.width as usize).max(1)
    }
}

impl Default for DecoderParams {
    fn default() -> Self {
        Self {
            line_duration_ms: 8.32,
            invert: false,
            gamma: 1.0,
            sync_lock: true,
            decode_window_secs: 2.0,
            mode: DecoderMode::Grayscale,
            width: 512,
        }
    }
}

#[derive(Default)]
pub struct SstvDecoder;

impl SstvDecoder {
    pub fn new() -> Self {
        Self
    }

    fn hann_window(size: usize) -> Vec<f32> {
        // The (size - 1) denominator underflows for size == 0 and divides by
        // zero for size == 1; both degenerate windows are all-ones.
        if size <= 1 {
            return vec![1.0; size];
        }
        (0..size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / ((size - 1) as f32)).cos()))
            .collect()
    }

    /// Detect the 1200 Hz calibration tone in a single FFT-sized chunk.
    ///
    /// This is navigation only: the ~1200 Hz tone marks image-section
    /// boundaries on the record. Per-line decode sync is a different mechanism
    /// entirely — see [`crate::analysis::sync::detect_line_syncs`].
    pub fn detect_tone_chunk(
        samples: &[f32],
        fft: &dyn RealToComplex<f32>,
        window: &[f32],
        sample_rate: f32,
        input_buffer: &mut [f32],
        spectrum_buffer: &mut [num_complex::Complex<f32>],
    ) -> bool {
        // Use reusable input buffer
        for ((s, w), dest) in samples.iter().zip(window.iter()).zip(input_buffer.iter_mut()) {
            *dest = s * w;
        }

        // Process FFT using reusable buffers. A size mismatch is the only
        // failure mode and it's unreachable by construction; degrade to "no
        // tone" rather than panicking the background worker thread.
        if fft.process(input_buffer, spectrum_buffer).is_err() {
            return false;
        }

        let bin_size = sample_rate / CHUNK_SIZE as f32;
        let target_bin = (TARGET_FREQ_HZ / bin_size).round() as usize;

        if target_bin >= spectrum_buffer.len() {
            return false;
        }

        let peak = spectrum_buffer[target_bin].norm();
        let avg = spectrum_buffer.iter().map(|c| c.norm()).sum::<f32>() / spectrum_buffer.len() as f32;

        peak > (avg * 10.0) // Simple threshold, tweak as needed
    }

    /// Detect presence of the 1200 Hz calibration tone in audio samples.
    ///
    /// This method performs a simple detection pass through the samples,
    /// stopping at the first detected tone region. For more comprehensive
    /// analysis, use `find_tone_regions()` or `find_next_tone_region()`. This
    /// is navigation only — not per-line decode sync (see
    /// [`crate::analysis::sync::detect_line_syncs`]).
    ///
    /// # Performance Note
    /// This method processes chunks sequentially until a tone is found.
    /// Consider using `find_tone_regions()` for batch analysis.
    pub fn detect_tone(&self, samples: &[f32], sample_rate: u32) -> bool {
        tracing::debug!(samples_len = samples.len(), "Starting tone detection");
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = Self::hann_window(CHUNK_SIZE);

        // Reusable buffers
        let mut input_buffer = fft.make_input_vec();
        let mut spectrum_buffer = fft.make_output_vec();

        for chunk in samples.chunks(CHUNK_SIZE) {
            if chunk.len() < CHUNK_SIZE {
                break;
            }
            let sync_detected = Self::detect_tone_chunk(
                chunk,
                &*fft,
                &window,
                sample_rate as f32,
                &mut input_buffer,
                &mut spectrum_buffer,
            );
            if sync_detected {
                tracing::debug!("Calibration tone detected");
                return true;
            }
        }
        tracing::debug!("No calibration tone detected");
        false
    }

    /// Find all 1200 Hz calibration-tone positions in the audio samples
    /// (navigation markers, not per-line decode sync).
    pub fn find_tone_regions(&self, samples: &[f32], sample_rate: u32) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = Self::hann_window(CHUNK_SIZE);

        // Reusable buffers
        let mut input_buffer = fft.make_input_vec();
        let mut spectrum_buffer = fft.make_output_vec();

        let mut i = 0;
        while i + CHUNK_SIZE <= samples.len() {
            let chunk = &samples[i..i + CHUNK_SIZE];
            let sync_detected = Self::detect_tone_chunk(
                chunk,
                &*fft,
                &window,
                sample_rate as f32,
                &mut input_buffer,
                &mut spectrum_buffer,
            );

            if sync_detected {
                positions.push(i);
                // Asymmetric stepping: Skip ahead 2x CHUNK_SIZE after finding sync to avoid
                // re-detecting the same tone in the following chunks. After sync is found,
                // the next line data follows, so we skip more aggressively.
                i += CHUNK_SIZE * 2;
            } else {
                // Overlap for better detection: step forward by 1/4 CHUNK_SIZE
                // to ensure we don't miss sync signals between chunk boundaries
                i += CHUNK_SIZE / 4;
            }
        }

        positions
    }

    /// Find the next sync position after the given sample position
    pub fn find_next_tone_region(&self, samples: &[f32], start_position: usize, sample_rate: u32) -> Option<usize> {
        if start_position >= samples.len() {
            return None;
        }

        let remaining_samples = &samples[start_position..];
        let sync_positions = self.find_tone_regions(remaining_samples, sample_rate);

        sync_positions.first().map(|&pos| start_position + pos)
    }

    /// Decode audio samples into image pixels.
    ///
    /// The record encodes images as baseband slow-scan video: the
    /// instantaneous signal level is the luminance trace, one scan line per
    /// ~8.32 ms, each line preceded by a sync spike with a falling edge.
    /// Decoding: segment into lines (sync-locked when possible), resample
    /// each line to `params.width` pixels (bin-averaging when downsampling),
    /// then normalize levels to 0-255 with a percentile contrast stretch,
    /// optional inversion, and gamma.
    ///
    /// # Arguments
    ///
    /// * `samples` - Audio samples normalized to [-1.0, 1.0]
    /// * `params` - Decoder parameters
    /// * `sample_rate` - Audio sample rate in Hz
    ///
    /// # Returns
    ///
    /// Grayscale pixels (0-255) in row-major order, `params.width` pixels
    /// wide (default 512). In PseudoColor mode, returns RGB pixels (3 bytes per pixel).
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidLineDuration`] if line duration is out of range 1-100ms.
    /// Returns [`DecoderError::InvalidParams`] if gamma is out of range 0.1-10.
    /// Returns [`DecoderError::InsufficientSamples`] if buffer is too short.
    pub fn decode(&self, samples: &[f32], params: &DecoderParams, sample_rate: u32) -> Result<Vec<u8>> {
        // Validate parameters
        if !(1.0..=100.0).contains(&params.line_duration_ms) {
            return Err(VoyagerError::Decoder(DecoderError::InvalidLineDuration {
                duration_ms: params.line_duration_ms,
            }));
        }

        if !(0.1..=10.0).contains(&params.gamma) {
            return Err(VoyagerError::Decoder(DecoderError::InvalidParams {
                reason: format!("gamma {} out of range 0.1-10.0", params.gamma),
            }));
        }

        // Validate samples
        if samples.is_empty() {
            return Err(VoyagerError::Decoder(DecoderError::InsufficientSamples {
                needed: 1,
                actual: 0,
            }));
        }

        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;
        if samples_per_line == 0 {
            return Err(VoyagerError::Decoder(DecoderError::InvalidParams {
                reason: format!(
                    "Calculated samples_per_line is 0 (line_duration={}, sample_rate={})",
                    params.line_duration_ms, sample_rate
                ),
            }));
        }

        if samples.len() < samples_per_line {
            return Err(VoyagerError::Decoder(DecoderError::InsufficientSamples {
                needed: samples_per_line,
                actual: samples.len(),
            }));
        }

        let width = params.effective_width();
        let max_lines = 16_384; // GPU texture limit

        // --- Line segmentation ---
        // Sync-locked when the detector finds a consistent line cadence;
        // otherwise fixed-period slicing at the nominal duration. Re-anchoring
        // at every detected sync keeps timing error from accumulating (slant).
        let line_ranges = self.segment_lines(samples, params, sample_rate, samples_per_line, max_lines);
        let lines_decoded = line_ranges.len();

        // --- Per-line level extraction ---
        // Resample each line to `width` luminance levels. Bin-averaging on
        // downsample doubles as the anti-alias filter; linear interpolation
        // covers the upsample case (e.g. 400 samples/line at 48 kHz -> 512 px).
        let mut levels: Vec<f32> = Vec::with_capacity(width * lines_decoded);
        for range in &line_ranges {
            let slice = &samples[range.clone()];
            resample_line(slice, width, &mut levels);
        }

        // --- Normalization ---
        // Percentile contrast stretch is robust to sync-spike outliers.
        let (lo, hi) = percentile_bounds(&levels, 0.01, 0.99);
        let span = (hi - lo).max(1e-6);
        let inv_gamma = 1.0 / params.gamma;
        let mut image: Vec<u8> = Vec::with_capacity(levels.len());
        for &level in &levels {
            let mut v = ((level - lo) / span).clamp(0.0, 1.0);
            if params.invert {
                v = 1.0 - v;
            }
            if params.gamma != 1.0 {
                v = v.powf(inv_gamma);
            }
            image.push((v * 255.0).round() as u8);
        }

        // Post-process for PseudoColor mode
        if params.mode == DecoderMode::PseudoColor {
            // Group 3 lines (R, G, B) into one color line
            // Current image buffer contains grayscale pixels (0 or 255)
            // We need to transform this into RGB pixels
            // Format: [R, G, B, R, G, B, ...]

            let num_pixels = image.len();
            let num_lines = num_pixels / width;
            let num_color_lines = num_lines / 3;

            // Warn if we're discarding incomplete color lines (not divisible by 3)
            if !num_lines.is_multiple_of(3) {
                tracing::warn!(
                    num_lines,
                    discarded_lines = num_lines % 3,
                    "PseudoColor: incomplete color lines detected, truncating to {} complete color lines",
                    num_color_lines
                );
            }

            let mut color_image = Vec::with_capacity(num_color_lines * width * 3);

            for line_idx in 0..num_color_lines {
                let r_start = (line_idx * 3) * width;
                let g_start = (line_idx * 3 + 1) * width;
                let b_start = (line_idx * 3 + 2) * width;

                for x in 0..width {
                    let r = image[r_start + x];
                    let g = image[g_start + x];
                    let b = image[b_start + x];

                    color_image.push(r);
                    color_image.push(g);
                    color_image.push(b);
                }
            }

            // Replace image with color image
            image = color_image;
        }

        tracing::debug!(lines_decoded, pixels = image.len(), "Decode operation completed");

        Ok(image)
    }

    /// Segment samples into per-line ranges. Prefers sync-locked boundaries;
    /// falls back to fixed-period slicing when sync structure is absent or
    /// inconsistent with the nominal line duration.
    fn segment_lines(
        &self,
        samples: &[f32],
        params: &DecoderParams,
        sample_rate: u32,
        samples_per_line: usize,
        max_lines: usize,
    ) -> Vec<std::ops::Range<usize>> {
        if params.sync_lock {
            let sync_params = SyncParams {
                expected_line_ms: params.line_duration_ms,
                ..SyncParams::default()
            };
            let positions = track_line_syncs(samples, sample_rate, &sync_params);
            if let Some(summary) = interval_summary(&positions, sample_rate) {
                let median = summary.median_samples;
                let nominal = samples_per_line as f64;
                // Trust sync lock only when the detected cadence is plausibly
                // the line cadence the caller asked about.
                if positions.len() >= 4 && (median - nominal).abs() / nominal < 0.3 {
                    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
                    let mut skipped = 0usize;
                    for w in positions.windows(2) {
                        let interval = (w[1] - w[0]) as f64;
                        // Skip gaps that are not a single line (dropouts, the
                        // inter-image boundary, double-triggers).
                        if interval >= median * 0.7 && interval <= median * 1.3 {
                            ranges.push(w[0]..w[1]);
                            if ranges.len() >= max_lines {
                                break;
                            }
                        } else {
                            skipped += 1;
                        }
                    }
                    if skipped > 0 && params.mode == DecoderMode::PseudoColor {
                        // Dropped lines shift the positional R/G/B grouping for
                        // everything after the gap; the mode has no way to
                        // recover phase, so at least say so.
                        tracing::warn!(
                            skipped,
                            "PseudoColor: sync gaps broke line continuity; RGB channel grouping may be shifted"
                        );
                    }
                    if !ranges.is_empty() {
                        tracing::debug!(
                            lines = ranges.len(),
                            median_interval = median,
                            "Sync-locked line segmentation"
                        );
                        return ranges;
                    }
                }
            }
            tracing::debug!("Sync lock requested but no consistent line cadence found; using fixed-period slicing");
        }

        let mut ranges = Vec::new();
        let mut i = 0;
        while i + samples_per_line <= samples.len() && ranges.len() < max_lines {
            ranges.push(i..i + samples_per_line);
            i += samples_per_line;
        }
        ranges
    }
}

/// Resample one line of samples to `width` luminance levels, appending to
/// `out`. Bin-averaging when downsampling (anti-aliased), linear
/// interpolation when upsampling.
fn resample_line(slice: &[f32], width: usize, out: &mut Vec<f32>) {
    let n = slice.len();
    if n == 0 {
        out.extend(std::iter::repeat_n(0.0, width));
        return;
    }
    if n >= width {
        // Downsample: average each pixel's sample bin.
        for x in 0..width {
            let a = x * n / width;
            let b = (((x + 1) * n / width).max(a + 1)).min(n);
            let sum: f32 = slice[a..b].iter().sum();
            out.push(sum / (b - a) as f32);
        }
    } else {
        // Upsample: linear interpolation.
        for x in 0..width {
            let exact_idx = if width > 1 {
                (x as f32 / (width as f32 - 1.0)) * (n as f32 - 1.0)
            } else {
                0.0
            };
            let idx_floor = exact_idx.floor() as usize;
            let idx_ceil = (idx_floor + 1).min(n - 1);
            let fract = exact_idx - idx_floor as f32;
            out.push(slice[idx_floor] * (1.0 - fract) + slice[idx_ceil] * fract);
        }
    }
}

/// Robust lower/upper bounds of `values` at the given percentiles.
///
/// Non-finite values (NaN/Inf from corrupt float WAVs) are excluded so they
/// cannot poison the normalization span and silently black out the image.
fn percentile_bounds(values: &[f32], lo_pct: f64, hi_pct: f64) -> (f32, f32) {
    let mut finite: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return (0.0, 1.0);
    }
    let lo_idx = ((finite.len() as f64 - 1.0) * lo_pct) as usize;
    let hi_idx = (((finite.len() as f64 - 1.0) * hi_pct) as usize).max(lo_idx);
    // Two selections beat a full sort: select the high percentile first, then
    // the low one within the lower partition.
    let (lower, hi_val, _) = finite.select_nth_unstable_by(hi_idx, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let hi = *hi_val;
    let lo = if lo_idx == hi_idx {
        hi
    } else {
        let (_, lo_val, _) = lower.select_nth_unstable_by(lo_idx, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        *lo_val
    };
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;

    fn generate_test_signal(frequency: f32, duration_secs: f32, sample_rate: u32) -> Vec<f32> {
        let num_samples = (duration_secs * sample_rate as f32) as usize;
        (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * PI * frequency * t).sin() * 0.5 // Half amplitude
            })
            .collect()
    }

    fn generate_noise(duration_secs: f32, sample_rate: u32) -> Vec<f32> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let num_samples = (duration_secs * sample_rate as f32) as usize;
        (0..num_samples)
            .map(|i| {
                // Simple deterministic pseudo-random noise
                let mut hasher = DefaultHasher::new();
                i.hash(&mut hasher);
                let hash = hasher.finish();
                ((hash % 1000) as f32 / 1000.0) * 0.1 - 0.05 // Small amplitude noise
            })
            .collect()
    }

    #[test]
    fn test_decoder_creation() {
        let _decoder = SstvDecoder::new();
        // Test that decoder can be created without panicking
    }

    #[test]
    fn test_default_params() {
        let params = DecoderParams::default();
        assert_eq!(params.line_duration_ms, 8.32);
        assert_eq!(params.gamma, 1.0);
        assert!(!params.invert);
        assert!(params.sync_lock);
        assert_eq!(params.mode, DecoderMode::Grayscale);
    }

    #[test]
    fn test_hann_window() {
        let window = SstvDecoder::hann_window(10);
        assert_eq!(window.len(), 10);

        // Standard symmetric Hann window (denominator = N-1)
        // Both endpoints should be exactly 0
        assert!((window[0] - 0.0).abs() < 1e-6, "First sample should be 0");
        assert!((window[9] - 0.0).abs() < 1e-6, "Last sample should be 0");
        // Peak is close to 1.0 (exactly 1.0 only when N is odd)
        let max_val = window.iter().fold(0.0f32, |max, &val| max.max(val));
        assert!(max_val > 0.95, "Peak should be close to 1.0, got {}", max_val);

        // Verify odd-length window has exact 1.0 peak at center
        let window_odd = SstvDecoder::hann_window(11);
        assert!(
            (window_odd[5] - 1.0).abs() < 1e-6,
            "Center of odd window should be exactly 1.0"
        );
    }

    #[test]
    fn test_sync_detection_positive() {
        let _decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Generate a signal with the target sync frequency
        let mut test_signal = generate_test_signal(TARGET_FREQ_HZ, 0.1, sample_rate);

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = SstvDecoder::hann_window(CHUNK_SIZE);

        if test_signal.len() >= CHUNK_SIZE {
            test_signal.truncate(CHUNK_SIZE);

            let mut input_buffer = fft.make_input_vec();
            let mut spectrum_buffer = fft.make_output_vec();

            let detected = SstvDecoder::detect_tone_chunk(
                &test_signal,
                &*fft,
                &window,
                sample_rate as f32,
                &mut input_buffer,
                &mut spectrum_buffer,
            );
            assert!(detected, "Should detect sync tone at target frequency");
        }
    }

    #[test]
    fn test_sync_detection_negative() {
        let _decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Generate noise that shouldn't trigger sync detection
        let mut test_signal = generate_noise(0.1, sample_rate);

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = SstvDecoder::hann_window(CHUNK_SIZE);

        if test_signal.len() >= CHUNK_SIZE {
            test_signal.truncate(CHUNK_SIZE);

            let mut input_buffer = fft.make_input_vec();
            let mut spectrum_buffer = fft.make_output_vec();

            let detected = SstvDecoder::detect_tone_chunk(
                &test_signal,
                &*fft,
                &window,
                sample_rate as f32,
                &mut input_buffer,
                &mut spectrum_buffer,
            );
            assert!(!detected, "Should not detect sync tone in noise");
        }
    }

    #[test]
    fn test_decode_empty_samples() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams::default();
        let empty_samples = Vec::new();

        let result = decoder.decode(&empty_samples, &params, 44100);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VoyagerError::Decoder(DecoderError::InsufficientSamples { .. })
        ));
    }

    #[test]
    fn test_decode_preserves_grayscale() {
        // A horizontal gradient must decode to a spread of gray levels, not a
        // binary collapse — the defining regression test for the old
        // threshold "decoder".
        let decoder = SstvDecoder::new();
        let params = DecoderParams::default();
        let sample_rate = 48_000;

        let width = 512usize;
        let n_lines = 24usize;
        let mut pixels = Vec::with_capacity(width * n_lines);
        for _ in 0..n_lines {
            for x in 0..width {
                pixels.push((x * 255 / (width - 1)) as u8);
            }
        }
        let audio = crate::test_fixtures::encode_image_to_audio(&pixels, width, sample_rate, params.line_duration_ms);

        let result = decoder.decode(&audio, &params, sample_rate).expect("Decode should succeed");

        assert!(!result.is_empty());
        assert_eq!(result.len() % width, 0);

        let distinct: std::collections::BTreeSet<u8> = result.iter().copied().collect();
        assert!(
            distinct.len() > 32,
            "gradient collapsed to {} distinct levels",
            distinct.len()
        );

        // Rows must be monotonically brighter left-to-right (sampled coarsely,
        // away from the sync-adjacent edges).
        let row = &result[..width];
        let quarter = row[width / 4];
        let mid = row[width / 2];
        let three_quarter = row[3 * width / 4];
        assert!(quarter < mid && mid < three_quarter, "{quarter} {mid} {three_quarter}");
    }

    #[test]
    fn test_decode_invert_flips_polarity() {
        let decoder = SstvDecoder::new();
        let sample_rate = 48_000;
        let width = 512usize;
        let mut pixels = vec![0u8; width * 8];
        for line in pixels.chunks_exact_mut(width) {
            for (x, p) in line.iter_mut().enumerate() {
                *p = (x * 255 / (width - 1)) as u8;
            }
        }
        let params = DecoderParams::default();
        let audio = crate::test_fixtures::encode_image_to_audio(&pixels, width, sample_rate, params.line_duration_ms);

        let normal = decoder.decode(&audio, &params, sample_rate).unwrap();
        let inverted = decoder
            .decode(&audio, &DecoderParams { invert: true, ..params }, sample_rate)
            .unwrap();

        assert_eq!(normal.len(), inverted.len());
        // Inversion: bright becomes dark, so the mid-row gradient flips direction
        let w = width;
        assert!(normal[w / 4] < normal[3 * w / 4]);
        assert!(inverted[w / 4] > inverted[3 * w / 4]);
    }

    #[test]
    fn test_decode_honors_non_default_width() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams {
            line_duration_ms: 10.0,
            width: 100,
            ..Default::default()
        };

        let sample_rate = 44100;
        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32) as usize;
        let test_samples = vec![0.5; samples_per_line * 3]; // 3 lines worth

        let result = decoder
            .decode(&test_samples, &params, sample_rate)
            .expect("Decode should succeed");

        // 3 lines of exactly params.width pixels each
        assert_eq!(result.len(), 300);
        assert_eq!(result.len() % params.width as usize, 0);
    }

    #[test]
    fn test_decode_pseudo_color_rgb_packing() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams {
            line_duration_ms: 1.0,
            mode: DecoderMode::PseudoColor,
            ..Default::default()
        };

        let sample_rate = 1000;
        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;
        assert_eq!(samples_per_line, 1);

        // Three lines: R=high, G=low, B=high -> expect (255,0,255) for every pixel.
        let samples = vec![
            1.0, // R line
            0.0, // G line
            1.0, // B line
        ];

        let result = decoder
            .decode(&samples, &params, sample_rate)
            .expect("PseudoColor decode should succeed");

        // One color line of width 512 => 512 * 3 bytes
        assert_eq!(result.len(), 512 * 3);

        // All pixels should match the expected RGB triplet
        for chunk in result.chunks_exact(3) {
            assert_eq!(chunk, [255, 0, 255]);
        }
    }

    #[test]
    fn test_find_tone_regions() {
        let decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Create a signal with sync tone at the beginning
        let sync_signal = generate_test_signal(TARGET_FREQ_HZ, 0.2, sample_rate);
        let noise_signal = generate_noise(0.2, sample_rate);

        let mut combined_signal = Vec::new();
        combined_signal.extend(&sync_signal);
        combined_signal.extend(&noise_signal);
        combined_signal.extend(&sync_signal); // Another sync signal

        let positions = decoder.find_tone_regions(&combined_signal, sample_rate);

        // Should find at least one sync position
        assert!(!positions.is_empty());

        // First position should be near the beginning
        assert!(positions[0] < sync_signal.len());
    }

    #[test]
    fn test_find_next_tone_region() {
        let decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Create signal with sync at position 0 and later
        let sync_signal = generate_test_signal(TARGET_FREQ_HZ, 0.1, sample_rate);
        let noise_signal = generate_noise(0.2, sample_rate);

        let mut test_signal = Vec::new();
        test_signal.extend(&sync_signal);
        test_signal.extend(&noise_signal);
        test_signal.extend(&sync_signal);

        // Search from the middle of noise section
        let start_pos = sync_signal.len() + noise_signal.len() / 2;
        let next_sync = decoder.find_next_tone_region(&test_signal, start_pos, sample_rate);

        if let Some(pos) = next_sync {
            assert!(pos > start_pos);
        }
    }

    #[test]
    fn test_find_next_tone_region_none() {
        let decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Create signal with only noise (no sync)
        let noise_signal = generate_noise(0.5, sample_rate);

        let next_sync = decoder.find_next_tone_region(&noise_signal, 0, sample_rate);

        // Should return None if no sync found
        assert!(next_sync.is_none());
    }

    // ---- resample_line: append-to-out contract ----

    #[test]
    fn test_resample_line_empty_slice_pushes_zeros() {
        let mut out = Vec::new();
        resample_line(&[], 8, &mut out);
        assert_eq!(out, vec![0.0; 8]);
    }

    #[test]
    fn test_resample_line_downsample_constant_is_constant() {
        // n >= width: every output bin averages a constant slice to that constant.
        let slice = vec![0.5f32; 100];
        let mut out = Vec::new();
        resample_line(&slice, 10, &mut out);
        assert_eq!(out.len(), 10);
        for v in out {
            assert!((v - 0.5).abs() < 1e-6, "expected 0.5, got {v}");
        }
    }

    #[test]
    fn test_resample_line_downsample_ramp_bin_averages() {
        // 0..10 downsampled to width 5 averages adjacent pairs: (0+1)/2, (2+3)/2, ...
        let slice: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let mut out = Vec::new();
        resample_line(&slice, 5, &mut out);
        assert_eq!(out, vec![0.5, 2.5, 4.5, 6.5, 8.5]);
    }

    #[test]
    fn test_resample_line_upsample_linear_interpolation() {
        // [0.0, 1.0] upsampled to width 5 is the evenly-spaced linear ramp.
        let mut out = Vec::new();
        resample_line(&[0.0, 1.0], 5, &mut out);
        assert_eq!(out.len(), 5);
        let expected = [0.0, 0.25, 0.5, 0.75, 1.0];
        for (got, want) in out.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "got {got}, want {want}");
        }
    }

    #[test]
    fn test_resample_line_width_one_averages_whole_slice() {
        // width == 1 takes the downsample branch with b clamped to n.
        let slice = vec![0.25f32; 7];
        let mut out = Vec::new();
        resample_line(&slice, 1, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.25).abs() < 1e-6);
    }

    // ---- percentile_bounds: robustness to non-finite values ----

    #[test]
    fn test_percentile_bounds_basic() {
        let values: Vec<f32> = (0..=100).map(|i| i as f32).collect();
        let (lo, hi) = percentile_bounds(&values, 0.01, 0.99);
        // len 101 => lo_idx = 100*0.01 = 1, hi_idx = 100*0.99 = 99.
        assert_eq!(lo, 1.0);
        assert_eq!(hi, 99.0);
    }

    #[test]
    fn test_percentile_bounds_lo_eq_hi_returns_same() {
        let (lo, hi) = percentile_bounds(&[5.0], 0.0, 1.0);
        assert_eq!(lo, 5.0);
        assert_eq!(hi, 5.0);
    }

    #[test]
    fn test_percentile_bounds_excludes_non_finite() {
        let values = [1.0, f32::NAN, 2.0, f32::INFINITY, 3.0, f32::NEG_INFINITY];
        let (lo, hi) = percentile_bounds(&values, 0.0, 1.0);
        // Only {1,2,3} participate; bounds must be drawn from them, never NaN/Inf.
        assert!(lo.is_finite() && hi.is_finite());
        assert_eq!(lo, 1.0);
        assert_eq!(hi, 3.0);
    }

    #[test]
    fn test_percentile_bounds_all_non_finite_fallback() {
        let values = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        assert_eq!(percentile_bounds(&values, 0.01, 0.99), (0.0, 1.0));
    }

    // ---- segment_lines: sync-locked vs fixed-period fallback ----

    #[test]
    fn test_segment_lines_sync_locked() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams::default(); // sync_lock = true, line_duration_ms = 8.32
        let sample_rate = 48_000;
        let width = 64usize;
        let n_lines = 12usize;
        // A simple gradient image, encoded with real per-line sync spikes.
        let mut pixels = Vec::with_capacity(width * n_lines);
        for _ in 0..n_lines {
            for x in 0..width {
                pixels.push((x * 255 / (width - 1)) as u8);
            }
        }
        let audio = crate::test_fixtures::encode_image_to_audio(&pixels, width, sample_rate, params.line_duration_ms);
        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;

        let ranges = decoder.segment_lines(&audio, &params, sample_rate, samples_per_line, 1000);

        // The sync-locked path must engage (>= 4 detected line syncs) and yield a
        // line per detected interval, near the nominal cadence.
        assert!(
            ranges.len() >= 4,
            "expected sync-locked segmentation, got {} ranges",
            ranges.len()
        );
        for r in &ranges {
            let len = r.end - r.start;
            let drift = (len as f32 - samples_per_line as f32).abs() / samples_per_line as f32;
            assert!(drift < 0.3, "line length {len} drifts too far from {samples_per_line}");
        }
        // Prove the sync-locked branch actually engaged rather than silently
        // falling through to fixed-period slicing: the fixed-period path emits
        // ranges all exactly `samples_per_line` wide, whereas sync-locked
        // follows the encoded (drifting) cadence, so some ranges differ.
        let all_nominal = ranges.iter().all(|r| r.end - r.start == samples_per_line);
        assert!(
            !all_nominal,
            "segmentation matches fixed-period fallback exactly; sync lock did not engage"
        );
    }

    #[test]
    fn test_segment_lines_falls_back_to_fixed_period() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams::default(); // sync_lock = true
        let sample_rate = 48_000;
        let samples_per_line = 400usize;
        // A flat signal has no sync structure, so sync lock finds no cadence and
        // the decoder falls back to evenly-spaced fixed-period slicing.
        let samples = vec![0.5f32; samples_per_line * 5];

        let ranges = decoder.segment_lines(&samples, &params, sample_rate, samples_per_line, 1000);

        assert_eq!(ranges.len(), 5);
        for (i, r) in ranges.iter().enumerate() {
            assert_eq!(r.start, i * samples_per_line);
            assert_eq!(r.end - r.start, samples_per_line);
        }
    }

    #[test]
    fn test_segment_lines_honors_max_lines() {
        let decoder = SstvDecoder::new();
        // Drive the fixed-period path explicitly so the cap is tested
        // independent of sync-detector behavior.
        let params = DecoderParams {
            sync_lock: false,
            ..DecoderParams::default()
        };
        let sample_rate = 48_000;
        let samples_per_line = 400usize;
        let samples = vec![0.5f32; samples_per_line * 10];

        let ranges = decoder.segment_lines(&samples, &params, sample_rate, samples_per_line, 3);
        assert_eq!(ranges.len(), 3);
    }

    // Property-based tests using proptest
    #[cfg(test)]
    mod proptests {
        use proptest::prelude::*;

        use super::*;

        /// Generate valid decoder parameters for property testing
        fn valid_decoder_params() -> impl Strategy<Value = DecoderParams> {
            (1.0f32..=100.0, 0.2f32..=3.0, any::<bool>(), any::<bool>()).prop_map(
                |(line_duration_ms, gamma, invert, sync_lock)| DecoderParams {
                    line_duration_ms,
                    gamma,
                    invert,
                    sync_lock,
                    mode: DecoderMode::Grayscale,
                    ..Default::default()
                },
            )
        }

        /// Generate valid sample rates for property testing
        fn valid_sample_rate() -> impl Strategy<Value = u32> {
            prop::sample::select(vec![8000, 11025, 16000, 22050, 44100, 48000, 96000])
        }

        /// Generate random audio samples in valid range
        fn random_samples(min_samples: usize, max_samples: usize) -> impl Strategy<Value = Vec<f32>> {
            prop::collection::vec(-1.0f32..=1.0f32, min_samples..=max_samples)
        }

        proptest! {
            /// Property: Decoder never panics with valid inputs
            #[test]
            fn prop_decode_never_panics(
                samples in random_samples(1000, 100_000),
                params in valid_decoder_params(),
                sample_rate in valid_sample_rate()
            ) {
                let decoder = SstvDecoder::new();
                let _ = decoder.decode(&samples, &params, sample_rate);
                // Test passes if no panic occurs
            }

            /// Property: Decoded output width is always 512 pixels
            #[test]
            fn prop_decode_width_always_512(
                samples in random_samples(1000, 50_000),
                params in valid_decoder_params(),
                sample_rate in valid_sample_rate()
            ) {
                let decoder = SstvDecoder::new();
                if let Ok(pixels) = decoder.decode(&samples, &params, sample_rate) {
                    if !pixels.is_empty() {
                        // All pixels should be multiples of 512
                        prop_assert_eq!(pixels.len() % 512, 0);
                    }
                }
            }

            /// Property: Decoder is deterministic (same input always produces same output)
            #[test]
            fn prop_decode_is_deterministic(
                samples in random_samples(1000, 20_000),
                params in valid_decoder_params(),
                sample_rate in valid_sample_rate()
            ) {
                let decoder = SstvDecoder::new();

                let result1 = decoder.decode(&samples, &params, sample_rate);
                let result2 = decoder.decode(&samples, &params, sample_rate);

                // Both should succeed or both should fail
                prop_assert_eq!(result1.is_ok(), result2.is_ok());

                // If successful, outputs should be identical
                if let (Ok(pixels1), Ok(pixels2)) = (result1, result2) {
                    prop_assert_eq!(pixels1, pixels2);
                }
            }

            /// Property: Invalid line duration returns error
            #[test]
            fn prop_invalid_line_duration_errors(
                samples in random_samples(1000, 10_000),
                invalid_duration in prop::num::f32::ANY.prop_filter(
                    "Not in valid range",
                    |&d| {
                        let valid_range = 1.0f32..=100.0;
                        !valid_range.contains(&d) || d.is_nan()
                    }
                ),
                sample_rate in valid_sample_rate()
            ) {
                let decoder = SstvDecoder::new();
                let params = DecoderParams {
                    line_duration_ms: invalid_duration,
                    mode: DecoderMode::Grayscale,
                    ..Default::default()
                };

                let result = decoder.decode(&samples, &params, sample_rate);
                prop_assert!(result.is_err());
            }

            /// Property: Invalid gamma returns error
            #[test]
            fn prop_invalid_gamma_errors(
                samples in random_samples(1000, 10_000),
                invalid_gamma in prop::num::f32::ANY.prop_filter(
                    "Not in valid range",
                    |&g| {
                        let valid_range = 0.1f32..=10.0;
                        !valid_range.contains(&g) || g.is_nan()
                    }
                ),
                sample_rate in valid_sample_rate()
            ) {
                let decoder = SstvDecoder::new();
                let params = DecoderParams {
                    line_duration_ms: 10.0,
                    gamma: invalid_gamma,
                    mode: DecoderMode::Grayscale,
                    ..Default::default()
                };

                let result = decoder.decode(&samples, &params, sample_rate);
                prop_assert!(result.is_err());
            }

            /// Property: Output size grows with more input samples.
            /// Holds for fixed-period slicing only — under sync lock, appending
            /// silence legitimately changes the detection threshold and thus
            /// which peaks qualify as line syncs.
            #[test]
            fn prop_output_grows_with_input(
                base_samples in random_samples(5000, 10_000),
                params_any in valid_decoder_params(),
                sample_rate in valid_sample_rate()
            ) {
                let params = DecoderParams { sync_lock: false, ..params_any };
                let decoder = SstvDecoder::new();

                // Decode with base samples
                let result1 = decoder.decode(&base_samples, &params, sample_rate);

                // Extend samples and decode again
                let mut extended_samples = base_samples.clone();
                extended_samples.extend(vec![0.0; 10_000]);
                let result2 = decoder.decode(&extended_samples, &params, sample_rate);

                // Both should succeed
                if let (Ok(pixels1), Ok(pixels2)) = (result1, result2) {
                    // Extended should have more or equal pixels
                    prop_assert!(pixels2.len() >= pixels1.len());
                }
            }
        }
    }
}
