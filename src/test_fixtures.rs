//! Synthetic audio test fixtures for development and testing
//!
//! This module provides deterministic audio signals with known properties,
//! allowing testing without committing large binary WAV files to the repository.

use std::f32::consts::PI;

/// Generate a pure sine wave at the given frequency
///
/// # Arguments
/// * `frequency` - Frequency in Hz (e.g., 440.0 for A4)
/// * `duration_secs` - Duration in seconds
/// * `sample_rate` - Sample rate in Hz (typically 44100 or 48000)
/// * `amplitude` - Peak amplitude, 0.0 to 1.0
///
/// # Example
/// ```
/// use voyager_explorer::test_fixtures::generate_sine_wave;
/// // Generate 1 second of A4 (440Hz) at half volume
/// let tone = generate_sine_wave(440.0, 1.0, 44100, 0.5);
/// assert_eq!(tone.len(), 44100);
/// ```
pub fn generate_sine_wave(frequency: f32, duration_secs: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            amplitude * (2.0 * PI * frequency * t).sin()
        })
        .collect()
}

/// Generate a linear chirp (frequency sweep)
///
/// Useful for testing frequency response and sync detection across ranges.
///
/// # Arguments
/// * `start_freq` - Starting frequency in Hz
/// * `end_freq` - Ending frequency in Hz
/// * `duration_secs` - Duration in seconds
/// * `sample_rate` - Sample rate in Hz
/// * `amplitude` - Peak amplitude, 0.0 to 1.0
pub fn generate_chirp(start_freq: f32, end_freq: f32, duration_secs: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let freq_range = end_freq - start_freq;

    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let t_norm = t / duration_secs; // 0.0 to 1.0
            let freq = start_freq + freq_range * t_norm;
            let phase = 2.0 * PI * freq * t;
            amplitude * phase.sin()
        })
        .collect()
}

/// Generate pseudo-random white noise
///
/// Deterministic noise based on sample index for reproducible tests.
pub fn generate_white_noise(duration_secs: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let num_samples = (duration_secs * sample_rate as f32) as usize;
    (0..num_samples)
        .map(|i| {
            let mut hasher = DefaultHasher::new();
            i.hash(&mut hasher);
            let hash = hasher.finish();
            // Convert hash to [-1.0, 1.0] range
            let normalized = ((hash % 2000) as f32 / 1000.0) - 1.0;
            amplitude * normalized
        })
        .collect()
}

/// Generate alternating high/low pattern for visual image testing
///
/// Creates a square wave pattern useful for SSTV decoding tests.
/// Produces clear stripes in decoded images.
pub fn generate_square_wave(frequency: f32, duration_secs: f32, sample_rate: u32, amplitude: f32) -> Vec<f32> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let phase = (2.0 * PI * frequency * t) % (2.0 * PI);
            if phase < PI {
                amplitude
            } else {
                -amplitude
            }
        })
        .collect()
}

/// Generate a sync signal pattern for SSTV testing
///
/// Pattern: sync tone → silence → sync tone → silence
/// Useful for testing sync detection and navigation.
pub fn generate_sync_pattern(sample_rate: u32) -> Vec<f32> {
    let sync_freq = 1200.0; // Voyager sync frequency
    let sync_duration = 0.1; // 100ms sync pulses
    let silence_duration = 0.5; // 500ms silence between

    let mut signal = Vec::new();

    // Sync 1
    signal.extend(generate_sine_wave(sync_freq, sync_duration, sample_rate, 0.8));

    // Silence 1
    signal.extend(vec![0.0; (silence_duration * sample_rate as f32) as usize]);

    // Sync 2
    signal.extend(generate_sine_wave(sync_freq, sync_duration, sample_rate, 0.8));

    // Silence 2
    signal.extend(vec![0.0; (silence_duration * sample_rate as f32) as usize]);

    signal
}

/// Generate a composite test signal with multiple features
///
/// Combines tone, noise, and silence for comprehensive testing.
pub fn generate_composite_signal(sample_rate: u32) -> Vec<f32> {
    let mut signal = Vec::new();

    // 0.5s of 440Hz tone (easy to recognize by ear)
    signal.extend(generate_sine_wave(440.0, 0.5, sample_rate, 0.6));

    // 0.2s silence
    signal.extend(vec![0.0; (0.2 * sample_rate as f32) as usize]);

    // 0.5s of noise
    signal.extend(generate_white_noise(0.5, sample_rate, 0.1));

    // 0.2s silence
    signal.extend(vec![0.0; (0.2 * sample_rate as f32) as usize]);

    // 1.0s chirp from 200Hz to 2000Hz
    signal.extend(generate_chirp(200.0, 2000.0, 1.0, sample_rate, 0.5));

    signal
}

/// Options for the forward image-to-audio model.
#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    /// Per-line timing drift in samples (analog tape-speed slant). Each line's
    /// period deviates from nominal by this amount, accumulating like real
    /// hardware drift.
    pub slant_samples_per_line: f32,
    /// Additive deterministic noise amplitude.
    pub noise_amplitude: f32,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            slant_samples_per_line: 0.0,
            noise_amplitude: 0.0,
        }
    }
}

/// Fraction of each line occupied by the sync structure (spike + dip).
pub const ENCODE_SYNC_FRAC: f32 = 0.06;
/// Peak level of the sync spike.
const SYNC_SPIKE_LEVEL: f32 = 1.0;
/// Bottom level of the falling-edge dip that marks the line start.
const SYNC_DIP_LEVEL: f32 = -0.8;
/// Maximum content level (pixel 255). Kept below the spike so sync detection
/// has headroom, mirroring the real signal where sync exceeds the video band.
const CONTENT_MAX_LEVEL: f32 = 0.7;

/// Encode a grayscale image into record-style baseband audio.
///
/// True forward model of the Voyager encoding as understood from reference
/// decodes: each scan line is a sync spike followed by a falling edge to a
/// dip (the line start marker), then the pixel luminance trace as the
/// instantaneous signal level. Intermediate gray levels map to intermediate
/// amplitudes — deliberately NOT mirroring decoder internals, so round-trip
/// tests exercise the decoder against an independent model.
pub fn encode_image_to_audio(pixels: &[u8], width: usize, sample_rate: u32, line_duration_ms: f32) -> Vec<f32> {
    encode_image_to_audio_with(pixels, width, sample_rate, line_duration_ms, &EncodeOptions::default())
}

/// [`encode_image_to_audio`] with slant and noise injection.
pub fn encode_image_to_audio_with(
    pixels: &[u8],
    width: usize,
    sample_rate: u32,
    line_duration_ms: f32,
    opts: &EncodeOptions,
) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    assert!(width > 0, "width must be non-zero");
    let nominal = line_duration_ms / 1000.0 * sample_rate as f32;
    assert!(nominal as usize >= 16, "line too short for sync structure");

    let n_lines = pixels.len() / width;
    let mut audio = Vec::with_capacity((nominal as usize + 1) * n_lines);

    // Accumulate line boundaries in f64 so slant builds up like real drift.
    let mut t = 0.0f64;
    for (line_idx, line) in pixels.chunks_exact(width).enumerate() {
        let period = nominal as f64 + opts.slant_samples_per_line as f64 * line_idx as f64;
        let start = t.round() as usize;
        let end = (t + period).round() as usize;
        let samples_this_line = end.saturating_sub(start).max(16);

        let sync_len = ((samples_this_line as f32 * ENCODE_SYNC_FRAC) as usize).max(4);
        let spike_len = sync_len / 2;
        let content_len = samples_this_line - sync_len;

        for i in 0..samples_this_line {
            let base = if i < spike_len {
                SYNC_SPIKE_LEVEL
            } else if i < sync_len {
                SYNC_DIP_LEVEL
            } else {
                // Pixel luminance trace: nearest-pixel level
                let ci = i - sync_len;
                let px = (ci * width / content_len).min(width - 1);
                line[px] as f32 / 255.0 * CONTENT_MAX_LEVEL
            };

            let noise = if opts.noise_amplitude > 0.0 {
                let mut hasher = DefaultHasher::new();
                (line_idx, i).hash(&mut hasher);
                ((hasher.finish() % 2000) as f32 / 1000.0 - 1.0) * opts.noise_amplitude
            } else {
                0.0
            };

            audio.push(base + noise);
        }
        t += period;
    }

    audio
}

/// Create a complete WAV file in memory for testing
///
/// Returns a temporary file handle that can be used with WavReader.
/// Available in tests and when test_fixtures feature is enabled.
pub fn create_test_wav_file(samples: &[f32], sample_rate: u32, channels: u16) -> tempfile::NamedTempFile {
    use std::io::Write;

    let mut file = tempfile::NamedTempFile::new().expect("create temp file");

    // Convert f32 samples to i16
    let i16_samples: Vec<i16> = samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();

    // Write WAV header
    let data_size = (i16_samples.len() * 2) as u32;
    let file_size = data_size + 36;

    // RIFF header
    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();

    // fmt chunk
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap(); // chunk size
    file.write_all(&1u16.to_le_bytes()).unwrap(); // PCM format
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes()).unwrap(); // byte rate
    file.write_all(&(channels * 2).to_le_bytes()).unwrap(); // block align
    file.write_all(&16u16.to_le_bytes()).unwrap(); // bits per sample

    // data chunk
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    // sample data
    for &sample in &i16_samples {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }

    file.flush().unwrap();
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_image_structure() {
        let width = 4;
        let pixels = vec![
            0, 85, 170, 255, // line 1: gradient
            255, 170, 85, 0, // line 2: reverse gradient
        ];
        let sample_rate = 40_000;
        let line_duration_ms = 10.0; // -> 400 samples/line
        let samples_per_line = (line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;

        let audio = encode_image_to_audio(&pixels, width, sample_rate, line_duration_ms);
        assert_eq!(audio.len(), samples_per_line * 2);

        // Sync spike at line start, dip after it
        assert_eq!(audio[0], 1.0);
        let sync_len = ((samples_per_line as f32 * ENCODE_SYNC_FRAC) as usize).max(4);
        assert_eq!(audio[sync_len - 1], -0.8);

        // Gray levels map to intermediate amplitudes, not binary
        let mid_line: Vec<f32> = audio[sync_len..samples_per_line].to_vec();
        let distinct: std::collections::BTreeSet<i32> = mid_line.iter().map(|v| (v * 1000.0) as i32).collect();
        assert!(distinct.len() >= 4, "expected >=4 gray levels, got {distinct:?}");
    }

    #[test]
    fn test_encode_slant_accumulates() {
        let width = 4;
        let pixels = vec![128u8; width * 100];
        let no_slant = encode_image_to_audio(&pixels, width, 48_000, 8.32);
        let slanted = encode_image_to_audio_with(
            &pixels,
            width,
            48_000,
            8.32,
            &EncodeOptions {
                slant_samples_per_line: 1.0,
                ..Default::default()
            },
        );
        // 100 lines at +1 sample/line drift accumulate ~ n*(n-1)/2 extra samples
        assert!(slanted.len() > no_slant.len() + 4000);
    }

    #[test]
    fn test_sine_wave_generation() {
        let signal = generate_sine_wave(440.0, 0.1, 44100, 0.5);
        assert_eq!(signal.len(), 4410); // 0.1s * 44100 samples/s

        // Check amplitude is within expected range
        let max = signal.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(max > 0.45 && max < 0.55); // Should be close to 0.5
    }

    #[test]
    fn test_chirp_generation() {
        let signal = generate_chirp(200.0, 2000.0, 0.5, 44100, 0.8);
        assert_eq!(signal.len(), 22050); // 0.5s * 44100
        assert!(!signal.is_empty());
    }

    #[test]
    fn test_white_noise_generation() {
        let signal = generate_white_noise(0.1, 44100, 0.3);
        assert_eq!(signal.len(), 4410);

        // Noise should have varying values (not all zeros)
        let variance = signal.iter().map(|&x| x * x).sum::<f32>() / signal.len() as f32;
        assert!(variance > 0.01); // Has some energy
    }

    #[test]
    fn test_square_wave_generation() {
        let signal = generate_square_wave(100.0, 0.1, 44100, 1.0);
        assert_eq!(signal.len(), 4410);

        // Should have values close to +1.0 and -1.0
        let positive_count = signal.iter().filter(|&&x| x > 0.5).count();
        let negative_count = signal.iter().filter(|&&x| x < -0.5).count();
        assert!(positive_count > 1000 && negative_count > 1000);
    }

    #[test]
    fn test_sync_pattern_generation() {
        let signal = generate_sync_pattern(44100);
        assert!(!signal.is_empty());

        // Should have alternating high-energy and low-energy regions
        let chunk_size = signal.len() / 4;
        let chunk1_energy: f32 = signal[..chunk_size].iter().map(|x| x * x).sum();
        let chunk2_energy: f32 = signal[chunk_size..chunk_size * 2].iter().map(|x| x * x).sum();

        // First chunk (sync) should have much more energy than second (silence)
        assert!(chunk1_energy > chunk2_energy * 10.0);
    }

    #[test]
    fn test_composite_signal_generation() {
        let signal = generate_composite_signal(44100);
        assert!(!signal.is_empty());
        assert!(signal.len() > 44100); // Should be > 1 second
    }

    #[test]
    fn test_wav_file_creation() {
        let samples = generate_sine_wave(440.0, 0.01, 44100, 0.5);
        let temp_file = create_test_wav_file(&samples, 44100, 1);

        // Verify file exists and has content
        let metadata = std::fs::metadata(temp_file.path()).unwrap();
        assert!(metadata.len() > 100); // Has WAV header + data
    }
}
