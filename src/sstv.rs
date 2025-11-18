use crate::error::{DecoderError, Result, VoyagerError};
use realfft::{RealFftPlanner, RealToComplex};
use std::f32::consts::PI;

/// Target sync frequency in Hz (Voyager Golden Record standard)
const TARGET_FREQ_HZ: f32 = 1200.0;
/// FFT chunk size for frequency analysis
const CHUNK_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy)]
pub struct DecoderParams {
    pub line_duration_ms: f32,
    pub threshold: f32,
}

impl Default for DecoderParams {
    fn default() -> Self {
        Self {
            line_duration_ms: 8.3,
            threshold: 0.2,
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
        (0..size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (size as f32 - 1.0)).cos()))
            .collect()
    }

    pub fn detect_sync_tone(
        samples: &[f32],
        fft: &dyn RealToComplex<f32>,
        window: &[f32],
        sample_rate: f32,
    ) -> bool {
        let mut input: Vec<f32> = samples
            .iter()
            .zip(window.iter())
            .map(|(s, w)| s * w)
            .collect();
        let mut spectrum = fft.make_output_vec();
        fft.process(&mut input, &mut spectrum).unwrap();

        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();
        let bin_size = sample_rate / CHUNK_SIZE as f32;
        let target_bin = (TARGET_FREQ_HZ / bin_size).round() as usize;

        if target_bin >= magnitudes.len() {
            return false;
        }

        let peak = magnitudes[target_bin];
        let avg = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;

        peak > (avg * 10.0) // Simple threshold, tweak as needed
    }

    /// Detect presence of sync tone in audio samples.
    ///
    /// This method performs a simple detection pass through the samples,
    /// stopping at the first detected sync. For more comprehensive sync
    /// analysis, use `find_sync_positions()` or `find_next_sync()`.
    ///
    /// # Performance Note
    /// This method processes chunks sequentially until a sync is found.
    /// Consider using `find_sync_positions()` for batch analysis.
    pub fn detect_sync(&self, samples: Vec<f32>, sample_rate: u32) -> bool {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = Self::hann_window(CHUNK_SIZE);

        for chunk in samples.chunks(CHUNK_SIZE) {
            if chunk.len() < CHUNK_SIZE {
                break;
            }
            let sync_detected = Self::detect_sync_tone(chunk, &*fft, &window, sample_rate as f32);
            if sync_detected {
                return true;
            }
        }
        false
    }

    /// Find all sync signal positions in the audio samples
    pub fn find_sync_positions(&self, samples: &[f32], sample_rate: u32) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = Self::hann_window(CHUNK_SIZE);

        let mut i = 0;
        while i + CHUNK_SIZE <= samples.len() {
            let chunk = &samples[i..i + CHUNK_SIZE];
            let sync_detected = Self::detect_sync_tone(chunk, &*fft, &window, sample_rate as f32);
            if sync_detected {
                positions.push(i);
                // Skip ahead to avoid detecting the same sync signal multiple times
                i += CHUNK_SIZE * 2;
            } else {
                i += CHUNK_SIZE / 4; // Overlap for better detection
            }
        }

        positions
    }

    /// Find the next sync position after the given sample position
    pub fn find_next_sync(
        &self,
        samples: &[f32],
        start_position: usize,
        sample_rate: u32,
    ) -> Option<usize> {
        if start_position >= samples.len() {
            return None;
        }

        let remaining_samples = &samples[start_position..];
        let sync_positions = self.find_sync_positions(remaining_samples, sample_rate);

        sync_positions.first().map(|&pos| start_position + pos)
    }

    /// Decode audio samples into SSTV image pixels with comprehensive error handling.
    ///
    /// # Arguments
    ///
    /// * `samples` - Audio samples normalized to [-1.0, 1.0]
    /// * `params` - Decoder parameters (line duration, threshold)
    /// * `sample_rate` - Audio sample rate in Hz (8kHz-192kHz)
    ///
    /// # Returns
    ///
    /// Grayscale pixels (0-255) in row-major order, width=512px
    ///
    /// # Errors
    ///
    /// Returns [`DecoderError::InvalidLineDuration`] if line duration is out of range 1-100ms.
    /// Returns [`DecoderError::InvalidThreshold`] if threshold is out of range 0.0-1.0.
    /// Returns [`DecoderError::InsufficientSamples`] if buffer is too short.
    pub fn decode(
        &self,
        samples: &[f32],
        params: &DecoderParams,
        sample_rate: u32,
    ) -> Result<Vec<u8>> {
        // Validate parameters
        if !(1.0..=100.0).contains(&params.line_duration_ms) {
            return Err(VoyagerError::Decoder(DecoderError::InvalidLineDuration {
                duration_ms: params.line_duration_ms,
            }));
        }

        if !(0.0..=1.0).contains(&params.threshold) {
            return Err(VoyagerError::Decoder(DecoderError::InvalidThreshold {
                threshold: params.threshold,
            }));
        }

        // Validate samples
        if samples.is_empty() {
            return Err(VoyagerError::Decoder(DecoderError::InsufficientSamples {
                needed: 1,
                actual: 0,
            }));
        }

        let samples_per_line =
            (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;
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

        let width = 512;
        let max_lines = 16_384; // GPU texture limit
        let mut image: Vec<u8> = Vec::new();
        let mut i = 0;
        let mut lines_decoded = 0;

        while i + samples_per_line <= samples.len() && lines_decoded < max_lines {
            let slice = &samples[i..i + samples_per_line];

            // Resample slice to 512 pixels
            for x in 0..width {
                let src_idx =
                    ((x as f32 / width as f32) * samples_per_line as f32).round() as usize;
                let src_idx = src_idx.min(samples_per_line - 1);
                let s = slice[src_idx];
                let pixel = if s.abs() > params.threshold { 255 } else { 0 };
                image.push(pixel);
            }
            i += samples_per_line;
            lines_decoded += 1;
        }

        tracing::debug!(
            lines_decoded,
            pixels = image.len(),
            "Decode operation completed"
        );

        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
        assert_eq!(params.line_duration_ms, 8.3);
        assert_eq!(params.threshold, 0.2);
    }

    #[test]
    fn test_hann_window() {
        let window = SstvDecoder::hann_window(10);
        assert_eq!(window.len(), 10);

        // Hann window should be symmetric and start/end at 0
        assert!((window[0] - 0.0).abs() < 1e-6);
        assert!((window[9] - 0.0).abs() < 1e-6);

        // Maximum should be around the middle
        let max_val = window.iter().fold(0.0f32, |max, &val| max.max(val));
        assert!(max_val > 0.9); // Should be close to 1.0
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
            let detected =
                SstvDecoder::detect_sync_tone(&test_signal, &*fft, &window, sample_rate as f32);
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
            let detected =
                SstvDecoder::detect_sync_tone(&test_signal, &*fft, &window, sample_rate as f32);
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
    fn test_decode_basic() {
        let decoder = SstvDecoder::new();
        let params = DecoderParams {
            line_duration_ms: 10.0, // Short duration for testing
            threshold: 0.3,
        };

        let sample_rate = 44100;
        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32) as usize;

        // Create test data with alternating high/low values
        let mut test_samples = Vec::new();
        for i in 0..(samples_per_line * 3) {
            // 3 lines worth
            let value = if (i / samples_per_line).is_multiple_of(2) {
                0.5
            } else {
                -0.5
            };
            test_samples.push(value);
        }

        let result = decoder
            .decode(&test_samples, &params, sample_rate)
            .expect("Decode should succeed");

        assert!(!result.is_empty());
        assert_eq!(result.len() % 512, 0); // Should be multiples of width

        // Check that pixels are either 0 or 255 (binary)
        for &pixel in &result {
            assert!(pixel == 0 || pixel == 255);
        }
    }

    #[test]
    fn test_find_sync_positions() {
        let decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Create a signal with sync tone at the beginning
        let sync_signal = generate_test_signal(TARGET_FREQ_HZ, 0.2, sample_rate);
        let noise_signal = generate_noise(0.2, sample_rate);

        let mut combined_signal = Vec::new();
        combined_signal.extend(&sync_signal);
        combined_signal.extend(&noise_signal);
        combined_signal.extend(&sync_signal); // Another sync signal

        let positions = decoder.find_sync_positions(&combined_signal, sample_rate);

        // Should find at least one sync position
        assert!(!positions.is_empty());

        // First position should be near the beginning
        assert!(positions[0] < sync_signal.len());
    }

    #[test]
    fn test_find_next_sync() {
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
        let next_sync = decoder.find_next_sync(&test_signal, start_pos, sample_rate);

        if let Some(pos) = next_sync {
            assert!(pos > start_pos);
        }
    }

    #[test]
    fn test_find_next_sync_none() {
        let decoder = SstvDecoder::new();
        let sample_rate = 44100;

        // Create signal with only noise (no sync)
        let noise_signal = generate_noise(0.5, sample_rate);

        let next_sync = decoder.find_next_sync(&noise_signal, 0, sample_rate);

        // Should return None if no sync found
        assert!(next_sync.is_none());
    }
}
