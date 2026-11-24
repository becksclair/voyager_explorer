use anyhow::Result;
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
    /// The decode used the full sample range of a segmented image.
    pub segmented_image: bool,
    /// The decoder aligned scan lines to detected syncs.
    pub sync_locked: bool,
    /// Rows holding the picture itself, clear of leader lines and the start
    /// burst, when the detector could tell (see
    /// [`crate::analysis::find_picture_rows`]). Trustworthy only together
    /// with `segmented_image`: the detector needs a whole image.
    pub picture_rows: Option<std::ops::Range<usize>>,
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

    /// Build the egui texture image in the *published* (upright) orientation.
    ///
    /// The decoder emits scan lines as rows (line N → row N), but Voyager's
    /// scan lines are vertical in the published images, so the recognizable
    /// orientation is the raw buffer rotated 90° clockwise then mirrored
    /// horizontally. That composition is a pure transpose: a source pixel at
    /// (x, y) maps to (y, x), and the width/height swap. The CLI applies the
    /// same `rotate90 + fliph` to its PNG output (see `cli.rs`); transposing
    /// here keeps the live GUI view consistent with it.
    ///
    /// When `crop_sync` is set, trim a segmented image to its detected
    /// picture rows and a sync-locked decode to the columns between its sync
    /// bands. With no detected picture the rows are left alone. Cropping
    /// affects only the displayed texture.
    pub fn to_egui_image(&self, crop_sync: bool) -> ColorImage {
        let w = self.width as usize;
        let h = self.height as usize;
        let cols = if crop_sync && self.sync_locked && w > 0 && h > 0 {
            display_crop_cols(w)
        } else {
            0..w
        };
        let rows = match &self.picture_rows {
            Some(rows) if crop_sync && self.segmented_image && !rows.is_empty() && rows.end <= h => rows.clone(),
            _ => 0..h,
        };
        let cw = cols.len();
        let ch = rows.len();
        // Transposed output: a cropped cw×ch source becomes a ch×cw image
        // (ColorImage size is [width, height]).
        let mut img = ColorImage::new([ch, cw], vec![egui::Color32::BLACK; cw * ch]);
        match self.mode {
            DecoderMode::Grayscale => {
                for (oy, sy) in rows.clone().enumerate() {
                    for (ox, sx) in cols.clone().enumerate() {
                        let src = sy * w + sx;
                        if src < self.pixels.len() {
                            img.pixels[ox * ch + oy] = egui::Color32::from_gray(self.pixels[src]);
                        }
                    }
                }
            }
            DecoderMode::PseudoColor => {
                for (oy, sy) in rows.clone().enumerate() {
                    for (ox, sx) in cols.clone().enumerate() {
                        let src = (sy * w + sx) * 3;
                        if src + 3 <= self.pixels.len() {
                            let r = self.pixels[src];
                            let g = self.pixels[src + 1];
                            let b = self.pixels[src + 2];
                            img.pixels[ox * ch + oy] = egui::Color32::from_rgb(r, g, b);
                        }
                    }
                }
            }
        }
        img
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
/// cropping to the common overlap. Frames marked `segmented_image` anchor
/// the search to their `picture_rows` (see
/// [`composite_triplet_levels_with_picture_rows`]).
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

    let picture_rows = [red, grn, blu].map(|f| f.segmented_image.then(|| f.picture_rows.clone()).flatten());
    let (lag_g, lag_b) = registration_lags(
        [red.pixels.as_slice(), grn.pixels.as_slice(), blu.pixels.as_slice()],
        &picture_rows,
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
    composite_triplet_levels_with_picture_rows(planes, &[None, None, None], width, invert, gamma)
}

/// [`composite_triplet_levels`], anchored by each plane's detected
/// [`crate::analysis::find_picture_rows`] range when available.
///
/// When two planes share an unclipped picture edge, their boundary offset
/// gives a tight prior (within ~2 lines on the Golden Record) so 2D NCC
/// refines locally instead of sliding spectrally distinct planes onto each
/// other (e.g., Solar Spectrum, where red/green/blue absorption bands sit
/// at different positions across the frame). If the local correlation curve
/// has no interior peak above its search boundary, registration keeps the
/// picture-row prior.
///
/// The prior is trusted: nothing outside `prior ± PRIOR_REFINE_RADIUS` is
/// searched. Pass `picture_rows` only from decodes of whole segmented
/// images (the detector needs the full frame layout), and `None` for any
/// plane whose rows are in doubt; that pair then registers by global NCC.
pub fn composite_triplet_levels_with_picture_rows(
    planes: [&[f32]; 3],
    picture_rows: &[Option<std::ops::Range<usize>>; 3],
    width: usize,
    invert: bool,
    gamma: f32,
) -> Result<DynamicImage> {
    anyhow::ensure!(width > 0, "width must be non-zero");
    let mut joint: Vec<f32> = planes.iter().flat_map(|p| content_levels(p, width)).collect();
    if joint.is_empty() {
        joint = planes.iter().flat_map(|p| p.iter().copied()).collect();
    }
    let (lo, hi) = crate::sstv::percentile_bounds(&joint, 0.01, 0.99);

    let min_height = planes.iter().map(|p| p.len() / width).min().unwrap_or(0);
    anyhow::ensure!(min_height > 0, "empty plane in triplet");
    let (lag_g, lag_b) = registration_lags(planes, picture_rows, width);

    let frames: Vec<PipelineResult> = planes
        .iter()
        .map(|levels| PipelineResult {
            pixels: crate::sstv::normalize_levels(levels, lo, hi, invert, gamma),
            width: width as u32,
            height: (levels.len() / width) as u32,
            mode: DecoderMode::Grayscale,
            segmented_image: false,
            sync_locked: false,
            picture_rows: None,
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

/// Central-row window (15–85%) for compositing: the first/last rows of a
/// segmented frame are inter-image leader bands. Wider than [`content_cols`]'
/// 20–90% column window because the leader bands are thinner than the
/// per-line sync residue. Non-empty and within `0..rows` for every
/// `rows >= 1`.
///
/// The clamp nests the `.min(rows)` *inside* the `.max` (unlike the sibling
/// helpers' outer `.min`) to stay byte-for-byte identical to the original
/// inline window in [`content_levels`]; don't "normalize" it to the outer
/// form without re-checking that equivalence.
fn content_rows(rows: usize) -> std::ops::Range<usize> {
    let lo = rows * 3 / 20;
    let hi = ((rows * 17) / 20).max((lo + 1).min(rows));
    lo..hi
}

/// Within-line crop for the *display* preview — trims only the decoder's
/// sync bands, far tighter than the compositor's [`content_cols`]. The
/// compositor's window is deliberately conservative (it only needs to keep
/// the junk *minority* out of a percentile population, so over-trimming is
/// free); cropping the same 20%/10% off the shown image throws away real
/// picture. Measured on left-channel frames of the 48 kHz rip: the
/// line-start band ends 7-9% into each scan line and the bright line-end
/// sync stripe starts at 88-90%. Fixed fractions work here because sync
/// lock pins every line to the same phase.
///
/// The dark strip that diagram frames show just inside the left bound when DC
/// restoration is off is picture, not margin, and must stay. Those frames have
/// a black background; the recording is AC-coupled, so without DC restoration
/// each line starts at true black and drifts back toward mid-grey over roughly
/// a fifth of the line. The level falls to black in one step at the 9% bound
/// and then recovers smoothly with no second edge, and photographs carry
/// ordinary picture content in the same columns.
/// Non-empty and within `0..width` for every `width >= 1`.
fn display_crop_cols(width: usize) -> std::ops::Range<usize> {
    let lo = width * 9 / 100;
    let hi = ((width * 89) / 100).max(lo + 1).min(width);
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
    let row_window = content_rows(rows);
    let cols = content_cols(width);
    levels
        .chunks_exact(width)
        .skip(row_window.start)
        .take(row_window.len())
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

/// Maximum difference in detected picture lengths between two planes when
/// both edges are unclipped; a larger mismatch means at least one edge is
/// unreliable, so registration falls back to global NCC.
const MAX_PICTURE_LEN_DIFF: usize = 10;

/// Local 2D NCC search radius (in scan lines) around a `picture_rows` prior.
/// Across all 20 Golden Record triplets the prior lands within ±2 lines of
/// the true offset; ±8 gives headroom for jitter while keeping distinct
/// spectral bands (Solar Spectrum, ~80–190 lines apart) out of reach.
/// The outer two lags on each side only guard the boundary (see
/// [`best_block_lag`]), so a refinement lands at most 6 lines from the prior.
const PRIOR_REFINE_RADIUS: i64 = 8;

/// Estimate the row lag of plane `b` relative to plane `a` from their
/// detected picture boundaries, using only unclipped edges (`start > 0`,
/// `end < height`).
fn picture_rows_prior_lag(
    pic_a: Option<&std::ops::Range<usize>>,
    height_a: usize,
    pic_b: Option<&std::ops::Range<usize>>,
    height_b: usize,
) -> Option<i64> {
    let (a, b) = (pic_a?, pic_b?);
    if a.is_empty() || b.is_empty() || a.end > height_a || b.end > height_b {
        return None;
    }
    let start_lag = (a.start > 0 && b.start > 0).then(|| b.start as i64 - a.start as i64);
    let end_lag = (a.end < height_a && b.end < height_b).then(|| b.end as i64 - a.end as i64);
    match (start_lag, end_lag) {
        (Some(s), Some(e)) => {
            if s.abs_diff(e) > MAX_PICTURE_LEN_DIFF as u64 {
                return None;
            }
            let sum = s + e;
            Some(if sum >= 0 { (sum + 1) / 2 } else { (sum - 1) / 2 })
        }
        (Some(s), None) => Some(s),
        (None, Some(e)) => Some(e),
        (None, None) => None,
    }
}

/// Row offsets of the green and blue planes relative to the red plane,
/// from 2D NCC over the planes' registration blocks (anchored locally to
/// `picture_rows` when unclipped picture edges are available).
fn registration_lags<T: Copy + Into<f64>>(
    planes: [&[T]; 3],
    picture_rows: &[Option<std::ops::Range<usize>>; 3],
    width: usize,
) -> (i64, i64) {
    // The search radius is per pair: bounding it by the third plane's
    // height would shrink the red↔green search just because the blue
    // plane came out short.
    let height_r = planes[0].len() / width;
    let pair_max_lag = |b: &[T]| {
        let min_height = height_r.min(b.len() / width);
        ((min_height / 3) as i64).clamp(1, REG_MAX_LAG_CAP)
    };
    let block_r = registration_block(planes[0], width);
    let prior_g = picture_rows_prior_lag(
        picture_rows[0].as_ref(),
        height_r,
        picture_rows[1].as_ref(),
        planes[1].len() / width,
    );
    let prior_b = picture_rows_prior_lag(
        picture_rows[0].as_ref(),
        height_r,
        picture_rows[2].as_ref(),
        planes[2].len() / width,
    );
    let lag_g = best_block_lag(
        &block_r,
        &registration_block(planes[1], width),
        pair_max_lag(planes[1]),
        prior_g,
    );
    let lag_b = best_block_lag(
        &block_r,
        &registration_block(planes[2], width),
        pair_max_lag(planes[2]),
        prior_b,
    );
    (lag_g, lag_b)
}

/// How close to the correlation peak a lag must be to count as equivalent.
/// Among equivalent lags the one nearest the anchor wins: the
/// `picture_rows` prior when there is one, lag 0 otherwise (a tie between
/// anchor ± k resolves to anchor - k). When the correlation curve is flat
/// (featureless slides) the argmax is noise, and the smallest departure
/// from the anchor is the safer choice.
const REG_PEAK_DELTA: f64 = 0.002;

/// Row offset of `b` relative to `a` maximizing the 2D normalized
/// cross-correlation of the registration blocks: a's row `i` aligns with
/// b's row `i + lag`. Correlates only over the central 60% of `a`'s rows —
/// the first/last rows of a segmented frame are inter-image leader junk.
///
/// When `prior` is given, searches only `prior ± PRIOR_REFINE_RADIUS`
/// (`max_lag` bounds the prior-less search alone) and accepts an NCC
/// refinement only when the interior peak rises strictly above the 2-line
/// border on each side of the window (covering both odd and even line
/// parities against 60 Hz hum ripple); monotonic ramps and flat curves keep
/// `prior`. Returns 0 for degenerate blocks without a prior.
fn best_block_lag(a: &RegistrationBlock, b: &RegistrationBlock, max_lag: i64, prior: Option<i64>) -> i64 {
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
    let (search_lo, search_hi) = match prior {
        Some(p) => (p - PRIOR_REFINE_RADIUS, p + PRIOR_REFINE_RADIUS),
        None => (-max_lag, max_lag),
    };
    let mut curve: Vec<(i64, f64)> = Vec::new();
    for lag in search_lo..=search_hi {
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
    let anchor = prior.unwrap_or(0);
    let Some(peak) = curve.iter().map(|&(_, c)| c).reduce(f64::max) else {
        return anchor;
    };
    if let Some(p) = prior {
        // Require a genuine interior peak: the 2-line border at each end of
        // the evaluated window (both odd and even parities) must sit strictly
        // below `peak - REG_PEAK_DELTA`. A monotonic ramp (spectrally distinct
        // planes) or flat curve stays anchored at `p`.
        let min_eval = curve.first().map(|&(l, _)| l).unwrap_or(p);
        let max_eval = curve.last().map(|&(l, _)| l).unwrap_or(p);
        let edge_max = curve
            .iter()
            .filter(|&&(l, _)| l <= min_eval + 1 || l >= max_eval - 1)
            .map(|&(_, c)| c)
            .reduce(f64::max)
            .unwrap_or(f64::NEG_INFINITY);
        if peak <= edge_max + REG_PEAK_DELTA {
            return p;
        }
    }
    curve
        .iter()
        .filter(|&&(_, c)| c >= peak - REG_PEAK_DELTA)
        .map(|&(lag, _)| lag)
        .min_by_key(|&lag| ((lag - anchor).abs(), lag))
        .unwrap_or(anchor)
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
        self.process_with_preroll(&[], samples, params, sample_rate)
    }

    /// [`Self::process`] with preceding audio samples used to warm up the
    /// DC-restoration filter before `samples[0]` so mid-picture windows open
    /// without a start-up transient.
    pub fn process_with_preroll(
        &self,
        preroll: &[f32],
        samples: &[f32],
        params: &DecoderParams,
        sample_rate: u32,
    ) -> Result<PipelineResult> {
        // No context here: every caller names the operation itself, and the
        // decoder's error already says what went wrong.
        let crate::sstv::DecodedFrame {
            pixels,
            sync_locked,
            picture_rows,
        } = self
            .decoder
            .decode_frame_with_preroll(preroll, samples, params, sample_rate)?;

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
            segmented_image: false,
            sync_locked,
            picture_rows,
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
            segmented_image: false,
            sync_locked: false,
            picture_rows: None,
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
            segmented_image: false,
            sync_locked: false,
            picture_rows: None,
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
    fn picture_rows_prior_handles_unclipped_clipped_and_mismatched_edges() {
        // Both edges unclipped: averages start and end offsets (symmetric rounding).
        assert_eq!(
            picture_rows_prior_lag(Some(&(29..555)), 593, Some(&(100..625)), 659),
            Some(71)
        );
        assert_eq!(picture_rows_prior_lag(Some(&(29..555)), 593, Some(&(29..555)), 578), Some(0));
        // Start clipped on `a` (Human Anatomy red plane): falls back to unclipped end.
        assert_eq!(
            picture_rows_prior_lag(Some(&(0..484)), 532, Some(&(124..649)), 707),
            Some(165)
        );
        // End clipped on `b`: falls back to unclipped start.
        assert_eq!(picture_rows_prior_lag(Some(&(20..120)), 140, Some(&(50..130)), 130), Some(30));
        // Both edges clipped on one plane: no prior.
        assert_eq!(picture_rows_prior_lag(Some(&(0..100)), 100, Some(&(20..120)), 140), None);
        // Picture lengths disagree by > MAX_PICTURE_LEN_DIFF: distrust prior.
        assert_eq!(picture_rows_prior_lag(Some(&(20..120)), 150, Some(&(20..140)), 160), None);
    }

    #[test]
    fn picture_rows_prior_anchors_spectrally_distinct_planes() {
        // Mirror Solar Spectrum: each plane carries a wide band at a different
        // position inside its picture, with distinct leader lengths before it.
        let width = 20usize;
        let make_plane = |height: usize, pic_start: usize, band_rel: usize| -> Vec<f32> {
            let mut levels = vec![0.5f32; height * width];
            for r in (pic_start + band_rel)..(pic_start + band_rel + 16) {
                for x in 6..14 {
                    levels[r * width + x] = 0.0;
                }
            }
            levels
        };
        let r = make_plane(160, 20, 20); // picture 20..120, band at picture-rel 20..36
        let g = make_plane(190, 50, 50); // picture 50..150, band at picture-rel 50..66 (true lag +30)
        let b = make_plane(150, 10, 80); // picture 10..110, band at picture-rel 80..96 (true lag -10)
        let pic_rows = [Some(20..120), Some(50..150), Some(10..110)];

        let (lag_g, lag_b) = registration_lags([r.as_slice(), g.as_slice(), b.as_slice()], &pic_rows, width);
        assert_eq!((lag_g, lag_b), (30, -10));

        // Verify the composite keeps the three spectral bands separated rather
        // than stacking them into one neutral band.
        let img = composite_triplet_levels_with_picture_rows([&r, &g, &b], &pic_rows, width, false, 1.0)
            .unwrap()
            .to_rgb8();
        assert_eq!(img.height(), 150);
        // In red-plane coords: red band is at y=40..56 (y_min=10 -> out y=30..46),
        // green band at g=100..116 -> r=70..86 (out y=60..76),
        // blue band at b=90..106 -> r=100..116 (out y=90..106).
        let px_r = img.get_pixel(10, 38);
        let px_g = img.get_pixel(10, 68);
        let px_b = img.get_pixel(10, 98);
        assert!(
            px_r[0] < 30 && px_r[1] > 200 && px_r[2] > 200,
            "red band misaligned: {px_r:?}"
        );
        assert!(
            px_g[1] < 30 && px_g[0] > 200 && px_g[2] > 200,
            "green band misaligned: {px_g:?}"
        );
        assert!(
            px_b[2] < 30 && px_b[0] > 200 && px_b[1] > 200,
            "blue band misaligned: {px_b:?}"
        );
    }

    #[test]
    fn picture_rows_prior_survives_ncc_ramp_toward_window_edge() {
        // Distinct bands 12 lines apart once the planes are aligned: inside
        // the refine window their overlap, and the NCC with it, climbs all the
        // way to the window edge without peaking. Solar Spectrum's planes ramp
        // like this; following the ramp slides the bands together, so the
        // prior must stand.
        let width = 20usize;
        let band_plane = |height: usize, band_start: usize| -> Vec<f32> {
            let mut levels = vec![0.5f32; height * width];
            for r in band_start..band_start + 16 {
                for x in 6..14 {
                    levels[r * width + x] = 0.0;
                }
            }
            levels
        };
        let r = band_plane(160, 40); // picture 20..120
        let g = band_plane(190, 82); // picture 50..150: prior +30, bands coincide at +42
        let b = band_plane(150, 18); // picture 10..110: prior -10, bands coincide at -22
        let pic_rows = [Some(20..120), Some(50..150), Some(10..110)];

        let (lag_g, lag_b) = registration_lags([r.as_slice(), g.as_slice(), b.as_slice()], &pic_rows, width);
        assert_eq!((lag_g, lag_b), (30, -10));
    }

    #[test]
    fn picture_rows_prior_refines_edge_jitter_with_local_ncc() {
        // Shared feature at true lags (+20, -10), while picture_rows carries ±2
        // lines of boundary jitter. Local NCC must refine to the exact feature lag.
        let r = banded_frame(140, 60);
        let g = banded_frame(160, 80); // true lag +20
        let b = banded_frame(130, 50); // true lag -10
        let pic_rows = [Some(20..120), Some(42..142), Some(11..111)]; // priors +22, -9

        let (lag_g, lag_b) = registration_lags(
            [r.pixels.as_slice(), g.pixels.as_slice(), b.pixels.as_slice()],
            &pic_rows,
            r.width as usize,
        );
        assert_eq!((lag_g, lag_b), (20, -10));
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

    #[test]
    fn to_egui_image_transposes_and_crops() {
        let (w, h) = (60u32, 60u32);
        let picture = 8..50usize;

        // Pixel value encodes its source column (x = i % w).
        let by_col = PipelineResult {
            pixels: (0..w * h).map(|i| (i % w) as u8).collect(),
            width: w,
            height: h,
            mode: DecoderMode::Grayscale,
            segmented_image: true,
            sync_locked: true,
            picture_rows: Some(picture.clone()),
        };
        // Uncropped is a pure transpose: source column x becomes output row x,
        // so every pixel in output row r has value r.
        let full = by_col.to_egui_image(false);
        assert_eq!(full.size, [60, 60]);
        assert_eq!(full.pixels[5 * 60 + 7], egui::Color32::from_gray(5));

        // Cropped to the display bounds; output rows map to the kept source columns.
        let cols = display_crop_cols(w as usize);
        let cropped = by_col.to_egui_image(true);
        assert_eq!(cropped.size, [picture.len(), cols.len()]);
        assert_eq!(cropped.pixels[0], egui::Color32::from_gray(cols.start as u8));
        assert_eq!(
            cropped.pixels[(cols.len() - 1) * picture.len()],
            egui::Color32::from_gray((cols.end - 1) as u8)
        );

        // Pixel value encodes its source row (y = i / w): the first output
        // column must map to the first picture row, proving the crop applies
        // the row offset, not just a size change.
        let by_row = PipelineResult {
            pixels: (0..w * h).map(|i| (i / w) as u8).collect(),
            ..by_col.clone()
        };
        assert_eq!(
            by_row.to_egui_image(true).pixels[0],
            egui::Color32::from_gray(picture.start as u8)
        );

        // A window is not a whole image, so its picture rows are not trusted.
        let windowed = PipelineResult {
            segmented_image: false,
            ..by_row.clone()
        }
        .to_egui_image(true);
        assert_eq!(windowed.size, [h as usize, cols.len()]);
        assert_eq!(windowed.pixels[0], egui::Color32::from_gray(0));

        // No picture found, or one that does not fit the frame: rows stay.
        for rows in [None, Some(0..0), Some(10..h as usize + 1)] {
            let kept = PipelineResult {
                picture_rows: rows,
                ..by_row.clone()
            }
            .to_egui_image(true);
            assert_eq!(kept.size, [h as usize, cols.len()]);
        }

        // Nothing known about the decode (a windowed, unlocked one): cropping
        // enabled must leave the image exactly as it is with cropping disabled.
        let plain = PipelineResult {
            segmented_image: false,
            sync_locked: false,
            picture_rows: None,
            ..by_col
        };
        let uncropped = plain.to_egui_image(true);
        assert_eq!(uncropped.size, full.size);
        assert_eq!(uncropped.pixels, full.pixels);
    }
}
