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
