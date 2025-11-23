use crate::sstv::{DecoderMode, DecoderParams, SstvDecoder};
use anyhow::{Context, Result};
use egui::ColorImage;
use image::{DynamicImage, GrayImage, Luma, Rgba, RgbaImage};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("Truncated pixel data: expected {expected} bytes, found {found} bytes")]
    TruncatedPixelData { expected: usize, found: usize },
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mode: DecoderMode,
}

impl PipelineResult {
    pub fn to_dynamic_image(&self) -> Result<DynamicImage, PipelineError> {
        // Compute expected length based on mode
        let expected_len = match self.mode {
            DecoderMode::BinaryGrayscale => (self.width * self.height) as usize,
            DecoderMode::PseudoColor => (self.width * self.height * 3) as usize,
        };

        // Validate pixel buffer length upfront
        if self.pixels.len() < expected_len {
            return Err(PipelineError::TruncatedPixelData {
                expected: expected_len,
                found: self.pixels.len(),
            });
        }

        // Proceed with current loops unchanged
        match self.mode {
            DecoderMode::BinaryGrayscale => {
                let mut buffer = GrayImage::new(self.width, self.height);
                for y in 0..self.height {
                    for x in 0..self.width {
                        let idx = (y * self.width + x) as usize;
                        if idx < self.pixels.len() {
                            let pixel = self.pixels[idx];
                            buffer.put_pixel(x, y, Luma([pixel]));
                        }
                    }
                }
                Ok(DynamicImage::ImageLuma8(buffer))
            }
            DecoderMode::PseudoColor => {
                let mut buffer = RgbaImage::new(self.width, self.height);
                for y in 0..self.height {
                    for x in 0..self.width {
                        let idx = (y * self.width + x) as usize * 3;
                        if idx + 2 < self.pixels.len() {
                            let r = self.pixels[idx];
                            let g = self.pixels[idx + 1];
                            let b = self.pixels[idx + 2];
                            buffer.put_pixel(x, y, Rgba([r, g, b, 255]));
                        }
                    }
                }
                Ok(DynamicImage::ImageRgba8(buffer))
            }
        }
    }

    pub fn to_egui_image(&self) -> ColorImage {
        match self.mode {
            DecoderMode::BinaryGrayscale => {
                let mut img = ColorImage::new(
                    [self.width as usize, self.height as usize],
                    vec![egui::Color32::BLACK; (self.width * self.height) as usize],
                );
                for (i, p) in self.pixels.iter().enumerate() {
                    if i < img.pixels.len() {
                        img.pixels[i] = egui::Color32::from_gray(*p);
                    }
                }
                img
            }
            DecoderMode::PseudoColor => {
                let mut img = ColorImage::new(
                    [self.width as usize, self.height as usize],
                    vec![egui::Color32::BLACK; (self.width * self.height) as usize],
                );
                for i in 0..img.pixels.len() {
                    let src_idx = i * 3;
                    if src_idx + 2 < self.pixels.len() {
                        let r = self.pixels[src_idx];
                        let g = self.pixels[src_idx + 1];
                        let b = self.pixels[src_idx + 2];
                        img.pixels[i] = egui::Color32::from_rgb(r, g, b);
                    }
                }
                img
            }
        }
    }
}

pub struct DecodingPipeline {
    decoder: SstvDecoder,
}

impl DecodingPipeline {
    pub fn new() -> Self {
        Self {
            decoder: SstvDecoder::new(),
        }
    }

    pub fn process(
        &self,
        samples: &[f32],
        params: &DecoderParams,
        sample_rate: u32,
    ) -> Result<PipelineResult> {
        let pixels = self
            .decoder
            .decode(samples, params, sample_rate)
            .context("Failed to decode audio")?;

        // Detect empty pixel data immediately, fail before computing dimensions
        if pixels.is_empty() {
            anyhow::bail!("Decoded pixels empty");
        }

        let width = 512;
        let row_size = match params.mode {
            DecoderMode::BinaryGrayscale => width as usize,
            DecoderMode::PseudoColor => (width as usize) * 3,
        };

        if pixels.len() % row_size != 0 {
            anyhow::bail!(
                "Pixel buffer length ({}) not evenly divisible by row size ({}) for mode {:?}",
                pixels.len(),
                row_size,
                params.mode
            );
        }

        let height = (pixels.len() / row_size) as u32;

        Ok(PipelineResult {
            pixels,
            width,
            height,
            mode: params.mode,
        })
    }
}

impl Default for DecodingPipeline {
    fn default() -> Self {
        Self::new()
    }
}
