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

    pub fn get_waveform_preview(&self, width: usize, channel: WaveformChannel) -> Vec<f32> {
        let samples = match channel {
            WaveformChannel::Left => &self.left_channel,
            WaveformChannel::Right => &self.right_channel,
        };
        
        if samples.is_empty() || width == 0 {
            return vec![];
        }
        
        let samples_per_pixel = samples.len() / width.max(1);
        
        // For very small files or when we have more pixels than samples
        if samples_per_pixel <= 1 {
            // Interpolate or pad samples to match the requested width
            if samples.len() >= width {
                return samples[..width].to_vec();
            } else {
                // For very short audio, repeat samples to fill the width
                let mut result = Vec::with_capacity(width);
                for i in 0..width {
                    let sample_idx = (i * samples.len()) / width;
                    result.push(samples[sample_idx.min(samples.len() - 1)]);
                }
                return result;
            }
        }
        
        // Use min/max envelope with RMS for better representation
        let mut result = Vec::with_capacity(width);
        
        for i in 0..width {
            let start = i * samples_per_pixel;
            let end = ((i + 1) * samples_per_pixel).min(samples.len());
            let chunk = &samples[start..end];
            
            if chunk.is_empty() {
                result.push(0.0);
                continue;
            }
            
            // Calculate min, max, and RMS for this chunk
            let mut min_val = f32::INFINITY;
            let mut max_val = f32::NEG_INFINITY;
            let mut sum_squares = 0.0;
            
            for &sample in chunk {
                min_val = min_val.min(sample);
                max_val = max_val.max(sample);
                sum_squares += sample * sample;
            }
            
            let rms = (sum_squares / chunk.len() as f32).sqrt();
            
            // Use the maximum of absolute min, max, and RMS to preserve both
            // peak information and overall energy level
            let peak = f32::max(max_val.abs(), min_val.abs());
            let representative_value = f32::max(peak * 0.7, rms * 1.2).min(1.0);
            
            // Preserve the sign of the dominant peak for visual clarity
            let sign = if max_val.abs() > min_val.abs() { 
                max_val.signum() 
            } else { 
                min_val.signum() 
            };
            
            result.push(representative_value * sign);
        }
        
        result
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
