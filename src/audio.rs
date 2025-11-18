use hound::WavReader as HoundReader;
use std::path::Path;

pub struct WavReader {
    pub left_channel: Vec<f32>,
    pub right_channel: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl WavReader {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut reader = HoundReader::open(path).map_err(|e| format!("Failed to open WAV: {e}"))?;
        let spec = reader.spec();

        if spec.channels != 1 && spec.channels != 2 {
            return Err("Only mono and stereo WAV files are supported.".into());
        }

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap_or(0) as f32 / i16::MAX as f32)
            .collect();

        let (left_channel, right_channel) = match spec.channels {
            1 => {
                // Mono: duplicate to both channels
                (samples.clone(), samples)
            }
            2 => {
                // Stereo: split interleaved samples
                let mut left = Vec::new();
                let mut right = Vec::new();

                for chunk in samples.chunks_exact(2) {
                    left.push(chunk[0]);
                    right.push(chunk[1]);
                }

                (left, right)
            }
            _ => unreachable!(),
        };

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

#[derive(Debug, Clone, Copy, PartialEq)]
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

        assert_eq!(reader.left_channel, expected_normalized);
        assert_eq!(reader.right_channel, expected_normalized); // Mono duplicated to both channels
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

        assert_eq!(reader.left_channel, expected_left);
        assert_eq!(reader.right_channel, expected_right);
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
