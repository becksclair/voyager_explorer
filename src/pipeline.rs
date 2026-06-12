use anyhow::{Context, Result};
use egui::ColorImage;
use image::{DynamicImage, GrayImage, Luma, Rgba, RgbaImage};
use thiserror::Error;

use crate::sstv::{DecoderMode, DecoderParams, SstvDecoder};

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
            DecoderMode::Grayscale => (self.width * self.height) as usize,
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
            DecoderMode::Grayscale => {
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
                        if idx + 3 <= self.pixels.len() {
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
            DecoderMode::Grayscale => {
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
                    if src_idx + 3 <= self.pixels.len() {
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

/// Composite three grayscale frames (red, green, blue members of a Voyager
/// color triplet) into one RGB image.
///
/// Per-line sync locking aligns content *within* each scan line, but the
/// frames start at their own segmentation boundaries, so the planes can be
/// offset from each other by tens of scan lines (rows). Registration:
/// cross-correlate each plane's row-mean luminance profile against the red
/// plane and shift by the best lag before stacking, cropping to the common
/// overlap.
pub fn composite_rgb(red: &PipelineResult, grn: &PipelineResult, blu: &PipelineResult) -> Result<DynamicImage> {
    for (name, frame) in [("red", red), ("green", grn), ("blue", blu)] {
        anyhow::ensure!(
            frame.mode == DecoderMode::Grayscale,
            "{name} frame is {:?}, composite needs Grayscale",
            frame.mode
        );
    }
    anyhow::ensure!(
        red.width == grn.width && grn.width == blu.width,
        "frame widths differ: {} / {} / {}",
        red.width,
        grn.width,
        blu.width
    );
    let min_height = red.height.min(grn.height).min(blu.height);
    anyhow::ensure!(min_height > 0, "empty frame in triplet");

    // Row offsets of green/blue relative to red, from profile correlation.
    let max_lag = (min_height / 4).clamp(1, 128) as i64;
    let profile_r = row_profile(red);
    let lag_g = best_row_lag(&profile_r, &row_profile(grn), max_lag);
    let lag_b = best_row_lag(&profile_r, &row_profile(blu), max_lag);
    tracing::debug!(lag_g, lag_b, "composite registration offsets (rows vs red)");

    // Overlap in red-plane row coordinates: row y reads green at y+lag_g and
    // blue at y+lag_b, all of which must be in range.
    let y_min = 0.max(-lag_g).max(-lag_b);
    let y_max = (red.height as i64)
        .min(grn.height as i64 - lag_g)
        .min(blu.height as i64 - lag_b);
    anyhow::ensure!(y_max > y_min, "no overlapping rows after registration");

    let width = red.width;
    let height = (y_max - y_min) as u32;
    let mut buffer = image::RgbImage::new(width, height);
    for y in 0..height {
        let ry = (y as i64 + y_min) as usize;
        for x in 0..width {
            let x = x as usize;
            let r = red.pixels[ry * width as usize + x];
            let g = grn.pixels[(ry as i64 + lag_g) as usize * width as usize + x];
            let b = blu.pixels[(ry as i64 + lag_b) as usize * width as usize + x];
            buffer.put_pixel(x as u32, y, image::Rgb([r, g, b]));
        }
    }
    Ok(DynamicImage::ImageRgb8(buffer))
}

/// Composite a color triplet from raw decoded levels (red, green, blue
/// planes, row-major at `width` per line). Percentile bounds are computed
/// jointly over the three planes — stretching each frame by its own bounds
/// would skew the color balance — then the planes are normalized with the
/// shared transform, registered, and stacked via [`composite_rgb`].
pub fn composite_triplet_levels(planes: [&[f32]; 3], width: usize, invert: bool, gamma: f32) -> Result<DynamicImage> {
    anyhow::ensure!(width > 0, "width must be non-zero");
    let joint: Vec<f32> = planes.iter().flat_map(|p| p.iter().copied()).collect();
    let (lo, hi) = crate::sstv::percentile_bounds(&joint, 0.01, 0.99);
    let frames: Vec<PipelineResult> = planes
        .iter()
        .map(|levels| PipelineResult {
            pixels: crate::sstv::normalize_levels(levels, lo, hi, invert, gamma),
            width: width as u32,
            height: (levels.len() / width) as u32,
            mode: DecoderMode::Grayscale,
        })
        .collect();
    composite_rgb(&frames[0], &frames[1], &frames[2])
}

/// Mean luminance per row (scan line) of a grayscale frame, computed over
/// the central columns only: the line-start region holds sync residue and
/// the row ends hold edge junk, both of which would dominate the profile.
fn row_profile(frame: &PipelineResult) -> Vec<f64> {
    let width = frame.width as usize;
    let lo = width / 5;
    let hi = (width * 9) / 10;
    let span = (hi - lo).max(1) as f64;
    frame
        .pixels
        .chunks_exact(width)
        .map(|row| {
            row[lo.min(row.len() - 1)..hi.min(row.len())]
                .iter()
                .map(|&p| p as f64)
                .sum::<f64>()
                / span
        })
        .collect()
}

/// Lag of `b` relative to `a` (in rows) maximizing normalized correlation:
/// a[i] aligns with b[i + lag]. Correlates only over the central 60% of
/// `a`'s rows — the first/last rows of a segmented frame are inter-image
/// leader junk whose strong, repetitive structure can outweigh the picture
/// content. Returns 0 for degenerate (flat) profiles.
fn best_row_lag(a: &[f64], b: &[f64], max_lag: i64) -> i64 {
    let a_lo = (a.len() / 5) as i64;
    let a_hi = (a.len() * 4 / 5) as i64;
    let mut best = (0i64, f64::NEG_INFINITY);
    for lag in -max_lag..=max_lag {
        let start = a_lo.max(-lag);
        let end = a_hi.min(b.len() as i64 - lag);
        if end - start < 16 {
            continue;
        }
        let n = (end - start) as f64;
        let (mut sa, mut sb) = (0.0, 0.0);
        for i in start..end {
            sa += a[i as usize];
            sb += b[(i + lag) as usize];
        }
        let (ma, mb) = (sa / n, sb / n);
        let (mut num, mut va, mut vb) = (0.0, 0.0, 0.0);
        for i in start..end {
            let da = a[i as usize] - ma;
            let db = b[(i + lag) as usize] - mb;
            num += da * db;
            va += da * da;
            vb += db * db;
        }
        if va <= f64::EPSILON || vb <= f64::EPSILON {
            continue;
        }
        let corr = num / (va * vb).sqrt();
        if corr > best.1 {
            best = (lag, corr);
        }
    }
    best.0
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

    pub fn process(&self, samples: &[f32], params: &DecoderParams, sample_rate: u32) -> Result<PipelineResult> {
        let pixels = self
            .decoder
            .decode(samples, params, sample_rate)
            .context("Failed to decode audio")?;

        // Detect empty pixel data immediately, fail before computing dimensions
        if pixels.is_empty() {
            anyhow::bail!("Decoded pixels empty");
        }

        let width = params.effective_width();
        let row_size = match params.mode {
            DecoderMode::Grayscale => width,
            DecoderMode::PseudoColor => width * 3,
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
            width: width as u32,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gray_frame(width: u32, height: u32, value: u8) -> PipelineResult {
        PipelineResult {
            pixels: vec![value; (width * height) as usize],
            width,
            height,
            mode: DecoderMode::Grayscale,
        }
    }

    #[test]
    fn composite_crops_to_smallest_height_and_maps_planes() {
        // Flat planes: degenerate profiles, no registration shift.
        let r = gray_frame(4, 10, 200);
        let g = gray_frame(4, 8, 100);
        let b = gray_frame(4, 9, 50);
        let img = composite_rgb(&r, &g, &b).unwrap();
        assert_eq!((img.width(), img.height()), (4, 8));
        let rgb = img.to_rgb8();
        assert_eq!(rgb.get_pixel(0, 0), &image::Rgb([200, 100, 50]));
    }

    fn banded_frame(height: u32, band_row: u32) -> PipelineResult {
        let width = 8u32;
        let mut pixels = vec![0u8; (width * height) as usize];
        for x in 0..width {
            pixels[(band_row * width + x) as usize] = 255;
        }
        PipelineResult {
            pixels,
            width,
            height,
            mode: DecoderMode::Grayscale,
        }
    }

    #[test]
    fn composite_triplet_levels_uses_joint_bounds() {
        // Red plane spans [0,1], green/blue sit lower; joint bounds keep the
        // relative plane intensities instead of stretching each to full range.
        let width = 4usize;
        let r: Vec<f32> = (0..width * 8).map(|i| (i % 8) as f32 / 7.0).collect();
        let g: Vec<f32> = r.iter().map(|v| v * 0.5).collect();
        let b: Vec<f32> = r.iter().map(|v| v * 0.25).collect();
        let img = composite_triplet_levels([&r, &g, &b], width, false, 1.0).unwrap().to_rgb8();
        // Where red is at its max, green must sit near half and blue near a
        // quarter of it — per-plane stretching would push all three to ~255.
        let px = img.get_pixel(3, 1); // column with the largest level
        assert!(px[0] > 220, "{px:?}");
        assert!((px[1] as i32 - px[0] as i32 / 2).abs() < 25, "{px:?}");
        assert!((px[2] as i32 - px[0] as i32 / 4).abs() < 25, "{px:?}");
    }

    #[test]
    fn composite_registers_row_offsets_between_planes() {
        // The same bright band sits at different rows in each plane;
        // registration must line the planes up into one white band.
        let r = banded_frame(64, 20);
        let g = banded_frame(64, 26);
        let b = banded_frame(64, 17);
        let img = composite_rgb(&r, &g, &b).unwrap().to_rgb8();
        let white_rows: Vec<u32> = (0..img.height())
            .filter(|&y| img.get_pixel(0, y) == &image::Rgb([255, 255, 255]))
            .collect();
        assert_eq!(white_rows.len(), 1, "expected one fully aligned band: {white_rows:?}");
        // No partially-colored rows: every other row must be black.
        for y in 0..img.height() {
            if y != white_rows[0] {
                assert_eq!(img.get_pixel(0, y), &image::Rgb([0, 0, 0]), "row {y}");
            }
        }
    }

    #[test]
    fn composite_rejects_mismatched_widths() {
        let r = gray_frame(4, 8, 0);
        let g = gray_frame(5, 8, 0);
        let b = gray_frame(4, 8, 0);
        assert!(composite_rgb(&r, &g, &b).is_err());
    }

    #[test]
    fn composite_rejects_pseudocolor_frames() {
        let r = gray_frame(4, 8, 0);
        let g = gray_frame(4, 8, 0);
        let mut b = gray_frame(4, 8, 0);
        b.mode = DecoderMode::PseudoColor;
        assert!(composite_rgb(&r, &g, &b).is_err());
    }
}
