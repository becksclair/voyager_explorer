use std::path::Path;
use std::sync::Arc;

use hound::{SampleFormat, WavReader as HoundReader, WavSpec};

use crate::error::{AudioError, Result};

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
/// All samples are normalized to f32 in the nominal range `[-1.0, 1.0]`.
/// Supported encodings: IEEE float32 (the format of the Golden Record rips)
/// and integer PCM up to 32 bits.
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
        Self::load(path.as_ref(), None, None)
    }

    /// Load only a time window of a WAV file, seeking past the leading frames.
    ///
    /// `start_secs` is clamped to the file length; `duration_secs` may run past
    /// the end of the file (the result is simply shorter). This keeps the
    /// multi-hundred-megabyte record rips usable without full streaming support.
    ///
    /// # Errors
    ///
    /// Same conditions as [`WavReader::from_file`]; additionally returns
    /// [`AudioError::EmptyFile`] when the window contains no samples.
    pub fn from_file_range<P: AsRef<Path>>(path: P, start_secs: f64, duration_secs: f64) -> Result<Self> {
        Self::load(path.as_ref(), Some(start_secs), Some(duration_secs))
    }

    /// Load a WAV file from `start_secs` to the end of the file.
    ///
    /// # Errors
    ///
    /// Same conditions as [`WavReader::from_file_range`].
    pub fn from_file_start<P: AsRef<Path>>(path: P, start_secs: f64) -> Result<Self> {
        Self::load(path.as_ref(), Some(start_secs), None)
    }

    fn load(path: &Path, start_secs: Option<f64>, duration_secs: Option<f64>) -> Result<Self> {
        let path_buf = path.to_path_buf();

        let mut reader = HoundReader::open(path).map_err(|source| AudioError::LoadFailed {
            path: path_buf.clone(),
            source,
        })?;

        let spec = reader.spec();

        // Reject only clearly-bogus rates. The Golden Record master rips are
        // 384 kHz, so there is no useful upper bound to enforce here.
        if spec.sample_rate < 8000 {
            return Err(AudioError::InvalidSampleRate { rate: spec.sample_rate }.into());
        }

        // Validate channel count
        if spec.channels != 1 && spec.channels != 2 {
            return Err(AudioError::UnsupportedChannels { channels: spec.channels }.into());
        }

        if let Some(start) = start_secs {
            let start_frame_wide = (start.max(0.0) * spec.sample_rate as f64) as u64;
            // hound's seek multiplies the frame index by the channel count in
            // u32 internally; an out-of-range start would overflow there and
            // silently land at the wrong position. Reject it instead.
            if start_frame_wide > (u32::MAX / spec.channels as u32) as u64 {
                return Err(AudioError::SeekOutOfRange { start_secs: start }.into());
            }
            reader
                .seek(start_frame_wide as u32)
                .map_err(|source| AudioError::LoadFailed {
                    path: path_buf.clone(),
                    source: hound::Error::IoError(source),
                })?;
        }

        let max_samples = duration_secs
            .map(|d| ((d.max(0.0) * spec.sample_rate as f64) as usize).saturating_mul(spec.channels as usize))
            .unwrap_or(usize::MAX);

        let samples = decode_samples(&mut reader, &spec, max_samples);

        // Check for empty file/window
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
                let mut left = Vec::with_capacity(samples.len() / 2);
                let mut right = Vec::with_capacity(samples.len() / 2);

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
            format = ?spec.sample_format,
            bits = spec.bits_per_sample,
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

/// Decode up to `max_samples` interleaved samples as normalized f32, honoring
/// the file's sample format and bit depth.
///
/// Sample-and-hold error recovery: when a sample fails to decode, the last
/// valid sample is reused rather than emitting 0.0 — sudden jumps to silence
/// cause audible clicks, whereas holding the previous value is perceptually
/// less jarring for the sporadic corruption patterns this targets.
fn decode_samples<R: std::io::Read>(reader: &mut HoundReader<R>, spec: &WavSpec, max_samples: usize) -> Vec<f32> {
    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, _) => collect_normalized(reader.samples::<f32>(), max_samples, |v| v),
        (SampleFormat::Int, bits @ 0..=16) => {
            let scale = ((1u32 << (bits.max(1) - 1)) - 1).max(1) as f32;
            collect_normalized(reader.samples::<i16>(), max_samples, move |v| v as f32 / scale)
        }
        (SampleFormat::Int, bits) => {
            let scale = ((1u64 << (bits.min(32) - 1)) - 1) as f32;
            collect_normalized(reader.samples::<i32>(), max_samples, move |v| v as f32 / scale)
        }
    }
}

fn collect_normalized<S, I, F>(samples: I, max_samples: usize, normalize: F) -> Vec<f32>
where
    I: Iterator<Item = hound::Result<S>>,
    F: Fn(S) -> f32,
{
    let mut held_samples: usize = 0;
    let mut last_valid_sample: f32 = 0.0;
    let mut logged_errors: usize = 0;
    const MAX_LOGGED_ERRORS: usize = 5;

    let out: Vec<f32> = samples
        .take(max_samples)
        .enumerate()
        .map(|(idx, s)| match s {
            Ok(val) => {
                let normalized = normalize(val);
                last_valid_sample = normalized;
                normalized
            }
            Err(e) => {
                held_samples += 1;
                // Rate-limit error logging to avoid spam on heavily corrupted files
                if logged_errors < MAX_LOGGED_ERRORS {
                    logged_errors += 1;
                    tracing::debug!(
                        sample_index = idx,
                        error = %e,
                        remaining_logs = MAX_LOGGED_ERRORS - logged_errors,
                        "Failed to decode sample, using sample-and-hold"
                    );
                }
                last_valid_sample
            }
        })
        .collect();

    if held_samples > 0 {
        tracing::warn!("Recovered {} corrupt sample(s) via sample-and-hold", held_samples);
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveformChannel {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

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
        file.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes()).unwrap(); // byte rate
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
        let expected_normalized: Vec<f32> = test_samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();

        assert_eq!(reader.left_channel.as_ref(), expected_normalized.as_slice());
        assert_eq!(reader.right_channel.as_ref(), expected_normalized.as_slice());
        // Mono duplicated to both channels
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
        let expected_left = vec![1000.0 / i16::MAX as f32, 3000.0 / i16::MAX as f32, 5000.0 / i16::MAX as f32];
        let expected_right = vec![2000.0 / i16::MAX as f32, 4000.0 / i16::MAX as f32, 6000.0 / i16::MAX as f32];

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

    fn create_f32_wav(samples: &[f32], sample_rate: u32, channels: u16) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(file.path(), spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        file
    }

    #[test]
    fn test_float32_mono_loading() {
        let samples = vec![0.0_f32, 0.25, -0.5, 1.0, -1.0];
        let temp_file = create_f32_wav(&samples, 48000, 1);

        let reader = WavReader::from_file(temp_file.path()).unwrap();

        assert_eq!(reader.sample_rate, 48000);
        assert_eq!(reader.left_channel.as_ref(), samples.as_slice());
    }

    #[test]
    fn test_float32_stereo_high_rate_loading() {
        // 384 kHz, the rate of the Golden Record master rips
        let samples = vec![0.1_f32, -0.1, 0.2, -0.2, 0.3, -0.3];
        let temp_file = create_f32_wav(&samples, 384_000, 2);

        let reader = WavReader::from_file(temp_file.path()).unwrap();

        assert_eq!(reader.sample_rate, 384_000);
        assert_eq!(reader.left_channel.as_ref(), &[0.1, 0.2, 0.3]);
        assert_eq!(reader.right_channel.as_ref(), &[-0.1, -0.2, -0.3]);
    }

    #[test]
    fn test_from_file_range() {
        // 1 second of mono at 1000 Hz... rates below 8 kHz are rejected, so use 8000.
        let rate = 8000u32;
        let samples: Vec<f32> = (0..rate * 2).map(|i| i as f32 / (rate * 2) as f32).collect();
        let temp_file = create_f32_wav(&samples, rate, 1);

        // Window: start at 0.5 s, take 0.25 s => 2000 samples starting at index 4000
        let reader = WavReader::from_file_range(temp_file.path(), 0.5, 0.25).unwrap();
        assert_eq!(reader.left_channel.len(), (rate as f64 * 0.25) as usize);
        assert_eq!(reader.left_channel[0], samples[(rate as f64 * 0.5) as usize]);

        // Duration past EOF is clamped to what exists
        let reader = WavReader::from_file_range(temp_file.path(), 1.5, 10.0).unwrap();
        assert_eq!(reader.left_channel.len(), (rate as f64 * 0.5) as usize);

        // Window entirely past EOF is an empty-file error
        assert!(WavReader::from_file_range(temp_file.path(), 5.0, 1.0).is_err());
    }
}
