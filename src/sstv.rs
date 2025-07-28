use realfft::{RealFftPlanner, RealToComplex};
use std::f32::consts::PI;

// const TARGET_FREQ_HZ: f32 = 3500.0;
const TARGET_FREQ_HZ: f32 = 1200.0;
const SAMPLE_RATE: f32 = 48000.0;
const CHUNK_SIZE: usize = 2048;


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

    pub fn detect_sync_tone(samples: &[f32], fft: &dyn RealToComplex<f32>, window: &[f32]) -> bool {
        let mut input: Vec<f32> = samples.iter().zip(window.iter()).map(|(s, w)| s * w).collect();
        let mut spectrum = fft.make_output_vec();
        fft.process(&mut input, &mut spectrum).unwrap();

        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();
        let bin_size = SAMPLE_RATE / CHUNK_SIZE as f32;
        let target_bin = (TARGET_FREQ_HZ / bin_size).round() as usize;

        let peak = magnitudes[target_bin];
        let avg = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;

        peak > (avg * 10.0) // Simple threshold, tweak as needed
    }

    pub fn detect_sync(&self, samples: Vec<f32>) {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(CHUNK_SIZE);
        let window = Self::hann_window(CHUNK_SIZE);

        for chunk in samples.chunks(CHUNK_SIZE) {
            if chunk.len() < CHUNK_SIZE {
                break;
            }
            let sync_detected = Self::detect_sync_tone(chunk, &*fft, &window);
            if sync_detected {
                println!("Sync tone detected!");
                break;
            }
        }
        println!("Sync tone not detected!");
    }

    pub fn decode(&self, samples: &[f32], params: &DecoderParams, sample_rate: u32) -> Vec<u8> {



        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;
        let width = 512;
        let max_lines = 16_384; // GPU texture limit
        let mut image: Vec<u8> = Vec::new();
        let mut i = 0;
        let mut lines_decoded = 0;

        while i + samples_per_line <= samples.len() && lines_decoded < max_lines {
            let slice = &samples[i..i + samples_per_line];

            // Resample slice to 512 pixels
            for x in 0..width {
                let src_idx = ((x as f32 / width as f32) * samples_per_line as f32).round() as usize;
                let src_idx = src_idx.min(samples_per_line - 1);
                let s = slice[src_idx];
                let pixel = if s.abs() > params.threshold { 255 } else { 0 };
                image.push(pixel);
            }
            i += samples_per_line;
            lines_decoded += 1;
        }

        image
    }
}
