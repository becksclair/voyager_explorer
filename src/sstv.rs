pub struct DecoderParams {
    pub line_duration_ms: u32,
    pub threshold: f32,
}

impl Default for DecoderParams {
    fn default() -> Self {
        Self {
            line_duration_ms: 32,
            threshold: 0.2,
        }
    }
}

pub struct SstvDecoder;

impl SstvDecoder {
    pub fn new() -> Self {
        Self
    }

    pub fn decode(&self, samples: &[f32], params: &DecoderParams, sample_rate: u32) -> Vec<u8> {
        let samples_per_line = (params.line_duration_ms as f32 / 1000.0
            * sample_rate as f32)
            .round() as usize;

        let mut image: Vec<u8> = Vec::new();
        let mut i = 0;

        while i + samples_per_line <= samples.len() {
            let slice = &samples[i..i + samples_per_line];
            let brightness = slice
                .iter()
                .map(|s| if s.abs() > params.threshold { 255 } else { 0 })
                .collect::<Vec<u8>>();

            image.extend(brightness);
            i += samples_per_line;
        }

        image
    }
}
