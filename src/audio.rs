use hound::WavReader as HoundReader;
use std::path::Path;

pub struct WavReader {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl WavReader {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut reader = HoundReader::open(path).map_err(|e| format!("Failed to open WAV: {e}"))?;
        let spec = reader.spec();

        if spec.channels != 1 {
            return Err("Only mono WAV files are supported right now.".into());
        }

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap_or(0) as f32 / i16::MAX as f32)
            .collect();

        Ok(Self {
            samples,
            sample_rate: spec.sample_rate,
        })
    }

    pub fn get_waveform_preview(&self, width: usize) -> Vec<f32> {
        let chunk_size = self.samples.len() / width.max(1);
        self.samples
            .chunks(chunk_size)
            .map(|chunk| {
                chunk.iter().copied().fold(0.0, |acc, v| f32::max(acc, v.abs()))
            })
            .collect()
    }
}
