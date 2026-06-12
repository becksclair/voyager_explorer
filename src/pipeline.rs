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
/// 2D normalized cross-correlation of per-scan-line-demeaned content
/// blocks against the red plane; shift by the best lag before stacking,
/// cropping to the common overlap.
pub fn composite_rgb(red: &PipelineResult, grn: &PipelineResult, blu: &PipelineResult) -> Result<DynamicImage> {
    for (name, frame) in [("red", red), ("green", grn), ("blue", blu)] {
        anyhow::ensure!(
            frame.mode == DecoderMode::Grayscale,
            "{name} frame is {:?}, composite needs Grayscale",
            frame.mode
        );
    }
    anyhow::ensure!(red.width > 0, "zero-width frame in triplet");
    anyhow::ensure!(
        red.width == grn.width && grn.width == blu.width,
        "frame widths differ: {} / {} / {}",
        red.width,
        grn.width,
        blu.width
    );
    let min_height = red.height.min(grn.height).min(blu.height);
    anyhow::ensure!(min_height > 0, "empty frame in triplet");

    let (lag_g, lag_b) = registration_lags(
        [red.pixels.as_slice(), grn.pixels.as_slice(), blu.pixels.as_slice()],
        red.width as usize,
    );
    stack_registered(red, grn, blu, lag_g, lag_b)
}

/// Stack three registered grayscale planes into one RGB image, cropping to
/// the rows where red row `y`, green row `y + lag_g`, and blue row
/// `y + lag_b` all exist.
fn stack_registered(
    red: &PipelineResult,
    grn: &PipelineResult,
    blu: &PipelineResult,
    lag_g: i64,
    lag_b: i64,
) -> Result<DynamicImage> {
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
/// jointly over the three planes' content regions — stretching each frame
/// by its own bounds would skew the color balance. Registration runs on
/// the raw levels *before* normalization (clipping to the joint bounds
/// can flatten a plane and starve the correlation); the planes are then
/// normalized with the shared transform and stacked at the found lags.
pub fn composite_triplet_levels(planes: [&[f32]; 3], width: usize, invert: bool, gamma: f32) -> Result<DynamicImage> {
    anyhow::ensure!(width > 0, "width must be non-zero");
    let mut joint: Vec<f32> = planes.iter().flat_map(|p| content_levels(p, width)).collect();
    if joint.is_empty() {
        joint = planes.iter().flat_map(|p| p.iter().copied()).collect();
    }
    let (lo, hi) = crate::sstv::percentile_bounds(&joint, 0.01, 0.99);

    let min_height = planes.iter().map(|p| p.len() / width).min().unwrap_or(0);
    anyhow::ensure!(min_height > 0, "empty plane in triplet");
    let (lag_g, lag_b) = registration_lags(planes, width);

    let frames: Vec<PipelineResult> = planes
        .iter()
        .map(|levels| PipelineResult {
            pixels: crate::sstv::normalize_levels(levels, lo, hi, invert, gamma),
            width: width as u32,
            height: (levels.len() / width) as u32,
            mode: DecoderMode::Grayscale,
        })
        .collect();
    stack_registered(&frames[0], &frames[1], &frames[2], lag_g, lag_b)
}

/// Central-column window shared by content-bounds gathering and plane
/// registration: line starts hold sync residue, line ends hold edge junk.
/// Non-empty and within `0..width` for every `width >= 1`.
fn content_cols(width: usize) -> std::ops::Range<usize> {
    let lo = width / 5;
    let hi = ((width * 9) / 10).max(lo + 1).min(width);
    lo..hi
}

/// Levels from a plane's content region: central columns (see
/// [`content_cols`]) and central rows (inter-image leader bands sit at
/// the frame edges; the 15–85% row window is looser than the 20–80%
/// correlation window in [`best_block_lag`] because the bounds only need
/// the junk *minority*, not exclusion of every edge row). Segmented
/// frames can carry 15–25% junk pixels; letting them into the percentile
/// population stretches the bounds far past the content range and washes
/// the composite out.
fn content_levels(levels: &[f32], width: usize) -> impl Iterator<Item = f32> + '_ {
    let rows = levels.len() / width;
    let row_lo = rows * 3 / 20;
    let row_hi = ((rows * 17) / 20).max((row_lo + 1).min(rows));
    let cols = content_cols(width);
    levels
        .chunks_exact(width)
        .skip(row_lo)
        .take(row_hi.saturating_sub(row_lo))
        .flat_map(move |row| row[cols.clone()].iter().copied())
}

/// Per-scan-line-demeaned content block for plane registration: the
/// central columns of every row (line-start sync residue and row-end junk
/// excluded, as in [`content_levels`]) with each row's mean subtracted.
/// Removing the per-line mean zeroes out flat junk/leader rows entirely —
/// only structure *within* a scan line survives, and that structure
/// matches across planes solely at the true row offset. A 1D row-mean
/// profile cannot make this distinction: junk bands and broad luminance
/// trends dominate it and produce confident wrong lags (Sunset).
struct RegistrationBlock {
    data: Vec<f64>,
    width: usize,
}

impl RegistrationBlock {
    fn rows(&self) -> usize {
        self.data.len().checked_div(self.width).unwrap_or(0)
    }

    fn row(&self, r: usize) -> &[f64] {
        &self.data[r * self.width..(r + 1) * self.width]
    }
}

fn registration_block<T: Copy + Into<f64>>(levels: &[T], width: usize) -> RegistrationBlock {
    let cols = content_cols(width);
    let bw = cols.len();
    let mut data = Vec::with_capacity((levels.len() / width) * bw);
    for row in levels.chunks_exact(width) {
        let seg = &row[cols.clone()];
        let mean = seg.iter().map(|&v| v.into()).sum::<f64>() / bw as f64;
        data.extend(seg.iter().map(|&v| v.into() - mean));
    }
    RegistrationBlock { data, width: bw }
}

/// Lag search radius: a third of the shortest plane, capped. Real plane
/// offsets on the 48 kHz rip reach ~170 lines (Human Anatomy), so a
/// tighter cap silently pins the search at its bound.
const REG_MAX_LAG_CAP: i64 = 256;

/// Row offsets of the green and blue planes relative to the red plane,
/// from 2D NCC over the planes' registration blocks.
fn registration_lags<T: Copy + Into<f64>>(planes: [&[T]; 3], width: usize) -> (i64, i64) {
    // The search radius is per pair: bounding it by the third plane's
    // height would shrink the red↔green search just because the blue
    // plane came out short.
    let pair_max_lag = |b: &[T]| {
        let min_height = (planes[0].len() / width).min(b.len() / width);
        ((min_height / 3) as i64).clamp(1, REG_MAX_LAG_CAP)
    };
    let block_r = registration_block(planes[0], width);
    let lag_g = best_block_lag(&block_r, &registration_block(planes[1], width), pair_max_lag(planes[1]));
    let lag_b = best_block_lag(&block_r, &registration_block(planes[2], width), pair_max_lag(planes[2]));
    (lag_g, lag_b)
}

/// How close to the correlation peak a lag must be to count as equivalent.
/// Among equivalent lags the smallest |lag| wins (a tie between ±k
/// resolves to -k): when the correlation curve is flat (featureless
/// slides) the argmax is noise, and a small shift is always the safer
/// choice.
const REG_PEAK_DELTA: f64 = 0.002;

/// Row offset of `b` relative to `a` maximizing the 2D normalized
/// cross-correlation of the registration blocks: a's row `i` aligns with
/// b's row `i + lag`. Correlates only over the central 60% of `a`'s rows —
/// the first/last rows of a segmented frame are inter-image leader junk.
/// Returns 0 for degenerate (flat) blocks.
fn best_block_lag(a: &RegistrationBlock, b: &RegistrationBlock, max_lag: i64) -> i64 {
    debug_assert_eq!(a.width, b.width);
    let a_lo = (a.rows() / 5) as i64;
    let a_hi = (a.rows() * 4 / 5) as i64;
    // NCC noise scales with 1/sqrt(window); a tiny overlap at an extreme
    // lag can out-correlate the true peak by chance, so windows below
    // half the *attainable* window don't compete. The attainable window
    // is the central span capped at the lag-0 window: when `b` is much
    // shorter than `a`, a bare half-span floor would disqualify lag 0
    // (or empty the curve entirely), and a floor sitting at the lag-0
    // window would disqualify every positive lag — both structurally
    // forced misregistrations.
    let window_at_zero = a_hi.min(b.rows() as i64) - a_lo;
    let min_window = ((a_hi - a_lo).min(window_at_zero) / 2).max(16);
    let mut curve: Vec<(i64, f64)> = Vec::new();
    for lag in -max_lag..=max_lag {
        let start = a_lo.max(-lag);
        let end = a_hi.min(b.rows() as i64 - lag);
        if end - start < min_window {
            continue;
        }
        let (mut num, mut va, mut vb) = (0.0, 0.0, 0.0);
        for i in start..end {
            // Rows are zero-mean by construction, so no window demeaning.
            for (da, db) in a.row(i as usize).iter().zip(b.row((i + lag) as usize)) {
                num += da * db;
                va += da * da;
                vb += db * db;
            }
        }
        if va <= f64::EPSILON || vb <= f64::EPSILON {
            continue;
        }
        curve.push((lag, num / (va * vb).sqrt()));
    }
    let Some(peak) = curve.iter().map(|&(_, c)| c).reduce(f64::max) else {
        return 0;
    };
    curve
        .iter()
        .filter(|&&(_, c)| c >= peak - REG_PEAK_DELTA)
        .map(|&(lag, _)| lag)
        .min_by_key(|&lag| (lag.abs(), lag))
        .unwrap_or(0)
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
        // Flat planes: degenerate registration blocks, no shift.
        let r = gray_frame(4, 10, 200);
        let g = gray_frame(4, 8, 100);
        let b = gray_frame(4, 9, 50);
        let img = composite_rgb(&r, &g, &b).unwrap();
        assert_eq!((img.width(), img.height()), (4, 8));
        let rgb = img.to_rgb8();
        assert_eq!(rgb.get_pixel(0, 0), &image::Rgb([200, 100, 50]));
    }

    fn banded_frame(height: u32, band_row: u32) -> PipelineResult {
        // The band has structure *within* the scan line (a single bright
        // pixel in the central columns): registration correlates per-line-
        // demeaned blocks, so a row that is uniform across columns carries
        // no registration signal.
        let width = 8u32;
        let mut pixels = vec![0u8; (width * height) as usize];
        pixels[(band_row * width + 3) as usize] = 255;
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
        // The same bright mark sits at different rows in each plane;
        // registration must line the planes up into one white pixel.
        let r = banded_frame(64, 20);
        let g = banded_frame(64, 26);
        let b = banded_frame(64, 17);
        let img = composite_rgb(&r, &g, &b).unwrap().to_rgb8();
        let white_rows: Vec<u32> = (0..img.height())
            .filter(|&y| img.get_pixel(3, y) == &image::Rgb([255, 255, 255]))
            .collect();
        assert_eq!(white_rows.len(), 1, "expected one fully aligned mark: {white_rows:?}");
        // No partially-colored rows: every other row must be black.
        for y in 0..img.height() {
            if y != white_rows[0] {
                assert_eq!(img.get_pixel(3, y), &image::Rgb([0, 0, 0]), "row {y}");
            }
        }
    }

    #[test]
    fn composite_registers_when_one_plane_is_much_shorter() {
        // A blue plane a third the height of the others: the noise floor
        // on the correlation window must not disqualify the small lags
        // that are the only ones a short plane can support.
        let r = banded_frame(900, 250);
        let g = banded_frame(900, 256);
        let b = banded_frame(300, 240);
        let img = composite_rgb(&r, &g, &b).unwrap().to_rgb8();
        let white_rows: Vec<u32> = (0..img.height())
            .filter(|&y| img.get_pixel(3, y) == &image::Rgb([255, 255, 255]))
            .collect();
        assert_eq!(white_rows.len(), 1, "expected one fully aligned mark: {white_rows:?}");
        for y in 0..img.height() {
            if y != white_rows[0] {
                assert_eq!(img.get_pixel(3, y), &image::Rgb([0, 0, 0]), "row {y}");
            }
        }
    }

    #[test]
    fn composite_registers_positive_lag_into_a_shorter_plane() {
        // Mirror of the test above: the short plane's mark sits *after*
        // the red mark (positive lag). A window floor pinned at the
        // lag-0 window excludes the entire positive lag axis.
        let r = banded_frame(900, 250);
        let g = banded_frame(900, 256);
        let b = banded_frame(300, 260);
        let img = composite_rgb(&r, &g, &b).unwrap().to_rgb8();
        let white_rows: Vec<u32> = (0..img.height())
            .filter(|&y| img.get_pixel(3, y) == &image::Rgb([255, 255, 255]))
            .collect();
        assert_eq!(white_rows.len(), 1, "expected one fully aligned mark: {white_rows:?}");
        for y in 0..img.height() {
            if y != white_rows[0] {
                assert_eq!(img.get_pixel(3, y), &image::Rgb([0, 0, 0]), "row {y}");
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
