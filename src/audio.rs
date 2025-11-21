use crate::error::{AudioError, Result};
use hound::WavReader as HoundReader;
use std::path::Path;
use std::sync::Arc;

/// WAV file reader with normalized `f32` samples and zero-copy buffer sharing.
///
/// # Architecture Decision: Arc&lt;[f32]&gt; vs Vec&lt;f32&gt;
///
/// This struct uses `Arc<[f32]>` instead of `Vec<f32>` for the audio buffers to enable
/// efficient zero-copy sharing with audio playback components.
///
/// **Key benefit**: When seeking during playback, we can create new `AudioBufferSource`
/// instances that share the same underlying buffer via `Arc::clone()`, which only
/// increments a reference count (O(1)) rather than copying megabytes of sample data (O(n)).
///
/// **Memory impact**: For a 100MB audio file with frequent seeking:
/// - With `Vec<f32>`: Each seek allocates 50MB average (half the file)
/// - With `Arc<[f32]>`: Each seek allocates 16 bytes (Arc pointer + metadata)
///
/// **Trade-off**: Arc adds 16 bytes of overhead vs Vec, but this is negligible compared
/// to the sample data size (millions of f32 values).
///
/// # Sample Normalization
///
/// All samples are normalized to f32 in the range `[-1.0, 1.0]` for consistent
/// processing. Currently only 16-bit PCM WAV files are supported.
pub struct WavReader {
    /// Left channel samples (or mono channel for mono files).
    /// Shared via Arc for zero-copy playback.
    pub left_channel: Arc<[f32]>,
    /// Right channel samples (duplicated from left for mono files).
    /// Shared via Arc for zero-copy playback.
    pub right_channel: Arc<[f32]>,
    /// Original sample rate in Hz (e.g., 44100, 48000).
    pub sample_rate: u32,
    /// Number of channels in the original file (1 for mono, 2 for stereo).
    pub channels: u16,
}

impl WavReader {
    /// Load and decode a WAV file with comprehensive error handling.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::LoadFailed`] if the file cannot be opened or read.
    /// Returns [`AudioError::UnsupportedChannels`] for files with > 2 channels.
    /// Returns [`AudioError::InvalidSampleRate`] for sample rates outside 8kHz-192kHz.
    /// Returns [`AudioError::EmptyFile`] if the file contains no audio samples.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use voyager_explorer::audio::WavReader;
    ///
    /// let reader = WavReader::from_file("audio.wav")?;
    /// println!("Loaded {} samples at {} Hz", reader.left_channel.len(), reader.sample_rate);
    /// # Ok::<(), voyager_explorer::error::VoyagerError>(())
    /// ```
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let path_buf = path.to_path_buf();

        let mut reader = HoundReader::open(path).map_err(|source| AudioError::LoadFailed {
            path: path_buf.clone(),
            source,
        })?;

        let spec = reader.spec();

        // Validate sample rate
        if !(8000..=192_000).contains(&spec.sample_rate) {
            return Err(AudioError::InvalidSampleRate {
                rate: spec.sample_rate,
            }
            .into());
        }

        // Validate channel count
        if spec.channels != 1 && spec.channels != 2 {
            return Err(AudioError::UnsupportedChannels {
                channels: spec.channels,
            }
            .into());
        }

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap_or(0) as f32 / i16::MAX as f32)
            .collect();

        // Check for empty file
        if samples.is_empty() {
            return Err(AudioError::EmptyFile { path: path_buf }.into());
        }

        let (left_channel, right_channel) = match spec.channels {
            1 => {
                // Mono: duplicate to both channels (convert to Arc)
                let arc_samples: Arc<[f32]> = samples.into();
                (Arc::clone(&arc_samples), arc_samples)
            }
            2 => {
                // Stereo: split interleaved samples
                let mut left = Vec::new();
                let mut right = Vec::new();

                for chunk in samples.chunks_exact(2) {
                    left.push(chunk[0]);
                    right.push(chunk[1]);
                }

                (left.into(), right.into())
            }
            _ => unreachable!(),
        };

        tracing::info!(
            path = %path.display(),
            sample_rate = spec.sample_rate,
            channels = spec.channels,
            samples = left_channel.len(),
            "Successfully loaded WAV file"
        );

        Ok(Self {
            left_channel,
            right_channel,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
        })
    }

    pub fn get_samples(&self, channel: WaveformChannel) -> &[f32] {
        match channel {
            WaveformChannel::Left => &self.left_channel,
            WaveformChannel::Right => &self.right_channel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WaveformChannel {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_wav(samples: &[i16], sample_rate: u32, channels: u16) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();

        // WAV header
        let data_size = (samples.len() * 2) as u32;
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
        for &sample in samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }

        file.flush().unwrap();
        file
    }

    #[test]
    fn test_mono_wav_loading() {
        let test_samples = vec![1000, 2000, 3000, 4000, 5000];
        let temp_file = create_test_wav(&test_samples, 44100, 1);

        let reader = WavReader::from_file(temp_file.path()).unwrap();

        assert_eq!(reader.sample_rate, 44100);
        assert_eq!(reader.channels, 1);
        assert_eq!(reader.left_channel.len(), 5);
        assert_eq!(reader.right_channel.len(), 5);

        // Check that samples are normalized to f32
        let expected_normalized: Vec<f32> = test_samples
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect();

        assert_eq!(reader.left_channel.as_ref(), expected_normalized.as_slice());
        assert_eq!(
            reader.right_channel.as_ref(),
            expected_normalized.as_slice()
        ); // Mono duplicated to both channels
    }

    #[test]
    fn test_stereo_wav_loading() {
        let test_samples = vec![1000, 2000, 3000, 4000, 5000, 6000]; // 3 stereo pairs
        let temp_file = create_test_wav(&test_samples, 48000, 2);

        let reader = WavReader::from_file(temp_file.path()).unwrap();

        assert_eq!(reader.sample_rate, 48000);
        assert_eq!(reader.channels, 2);
        assert_eq!(reader.left_channel.len(), 3);
        assert_eq!(reader.right_channel.len(), 3);

        // Check that left and right channels are separated correctly
        let expected_left = vec![
            1000.0 / i16::MAX as f32,
            3000.0 / i16::MAX as f32,
            5000.0 / i16::MAX as f32,
        ];
        let expected_right = vec![
            2000.0 / i16::MAX as f32,
            4000.0 / i16::MAX as f32,
            6000.0 / i16::MAX as f32,
        ];

        assert_eq!(reader.left_channel.as_ref(), expected_left.as_slice());
        assert_eq!(reader.right_channel.as_ref(), expected_right.as_slice());
    }

    #[test]
    fn test_get_samples() {
        let test_samples = vec![1000, 2000, 3000, 4000, 5000, 6000];
        let temp_file = create_test_wav(&test_samples, 44100, 2);
        let reader = WavReader::from_file(temp_file.path()).unwrap();

        let left_samples = reader.get_samples(WaveformChannel::Left);
        let right_samples = reader.get_samples(WaveformChannel::Right);

        assert_eq!(left_samples.len(), 3);
        assert_eq!(right_samples.len(), 3);
        assert_ne!(left_samples, right_samples); // Should be different for stereo
    }

    #[test]
    fn test_invalid_wav_file() {
        let result = WavReader::from_file("nonexistent_file.wav");
        assert!(result.is_err());
    }
}
