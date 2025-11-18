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
pub fn generate_sine_wave(
    frequency: f32,
    duration_secs: f32,
    sample_rate: u32,
    amplitude: f32,
) -> Vec<f32> {
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
pub fn generate_chirp(
    start_freq: f32,
    end_freq: f32,
    duration_secs: f32,
    sample_rate: u32,
    amplitude: f32,
) -> Vec<f32> {
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
pub fn generate_square_wave(
    frequency: f32,
    duration_secs: f32,
    sample_rate: u32,
    amplitude: f32,
) -> Vec<f32> {
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
    signal.extend(generate_sine_wave(
        sync_freq,
        sync_duration,
        sample_rate,
        0.8,
    ));

    // Silence 1
    signal.extend(vec![0.0; (silence_duration * sample_rate as f32) as usize]);

    // Sync 2
    signal.extend(generate_sine_wave(
        sync_freq,
        sync_duration,
        sample_rate,
        0.8,
    ));

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

/// Create a complete WAV file in memory for testing
///
/// Returns a temporary file handle that can be used with WavReader.
/// Available in tests and when test_fixtures feature is enabled.
pub fn create_test_wav_file(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> tempfile::NamedTempFile {
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
    file.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes())
        .unwrap(); // byte rate
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
        let chunk2_energy: f32 = signal[chunk_size..chunk_size * 2]
            .iter()
            .map(|x| x * x)
            .sum();

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
