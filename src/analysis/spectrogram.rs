//! STFT spectrogram computation and PNG-ready rendering with labeled
//! frequency/time axes and marker lines (e.g. the 1200 Hz sync candidate).

use image::{Rgb, RgbImage};
use realfft::RealFftPlanner;

use super::font;

#[derive(Debug, Clone)]
pub struct SpectrogramParams {
    /// FFT window size in samples (power of two recommended).
    pub fft_size: usize,
    /// Hop between consecutive windows, in samples.
    pub hop: usize,
    /// Upper frequency bound to display, Hz. `None` = Nyquist.
    pub fmax: Option<f32>,
}

impl Default for SpectrogramParams {
    fn default() -> Self {
        Self {
            fft_size: 1024,
            hop: 256, // 75% overlap
            fmax: None,
        }
    }
}

/// Spectrogram magnitudes in dBFS-ish units (20·log10 of windowed magnitude).
pub struct Spectrogram {
    /// `frames[t][f]` — time-major, `bins` values per frame, in dB.
    pub frames: Vec<Vec<f32>>,
    /// Number of frequency bins per frame (limited by `fmax`).
    pub bins: usize,
    /// Frequency step between bins, Hz.
    pub freq_step: f32,
    /// Time step between frames, seconds.
    pub time_step: f32,
    pub sample_rate: u32,
}

/// Compute an STFT magnitude spectrogram (Hann window).
pub fn compute_spectrogram(samples: &[f32], sample_rate: u32, params: &SpectrogramParams) -> Spectrogram {
    let fft_size = params.fft_size.max(16);
    let hop = params.hop.clamp(1, fft_size);
    let freq_step = sample_rate as f32 / fft_size as f32;
    let total_bins = fft_size / 2 + 1;
    let bins = match params.fmax {
        Some(fmax) => ((fmax / freq_step).ceil() as usize + 1).clamp(2, total_bins),
        None => total_bins,
    };

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(fft_size);
    let mut input = r2c.make_input_vec();
    let mut output = r2c.make_output_vec();

    let window: Vec<f32> = (0..fft_size)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / (fft_size as f32 - 1.0);
            0.5 * (1.0 - phase.cos())
        })
        .collect();

    let mut frames = Vec::new();
    if samples.len() >= fft_size {
        let mut start = 0;
        while start + fft_size <= samples.len() {
            for (dst, (s, w)) in input
                .iter_mut()
                .zip(samples[start..start + fft_size].iter().zip(window.iter()))
            {
                *dst = s * w;
            }
            if r2c.process(&mut input, &mut output).is_ok() {
                let frame: Vec<f32> = output
                    .iter()
                    .take(bins)
                    .map(|c| 20.0 * (c.norm() / fft_size as f32 + 1e-12).log10())
                    .collect();
                frames.push(frame);
            }
            start += hop;
        }
    }

    Spectrogram {
        frames,
        bins,
        freq_step,
        time_step: hop as f32 / sample_rate as f32,
        sample_rate,
    }
}

const MARGIN_LEFT: u32 = 52;
const MARGIN_BOTTOM: u32 = 22;
const MARGIN_TOP: u32 = 8;
const MARGIN_RIGHT: u32 = 8;

const AXIS_COLOR: Rgb<u8> = Rgb([170, 170, 170]);
const GRID_COLOR: Rgb<u8> = Rgb([70, 70, 70]);
const MARKER_COLOR: Rgb<u8> = Rgb([0, 230, 230]);
const BG_COLOR: Rgb<u8> = Rgb([16, 16, 20]);

/// Render a spectrogram to an RGB image with axes, gridlines, and horizontal
/// marker lines at the given frequencies. `start_secs` offsets the time-axis
/// labels so windowed reads show absolute file time.
pub fn render_spectrogram(spec: &Spectrogram, start_secs: f64, mark_freqs: &[f32], target_width: u32) -> RgbImage {
    let n_frames = spec.frames.len().max(1);
    let plot_w = target_width.clamp(256, 4096);
    // Scale bins vertically to a readable height.
    let v_scale = (512 / spec.bins.max(1) as u32).clamp(1, 8);
    let plot_h = (spec.bins as u32 * v_scale).clamp(128, 1024);

    let width = MARGIN_LEFT + plot_w + MARGIN_RIGHT;
    let height = MARGIN_TOP + plot_h + MARGIN_BOTTOM;
    let mut img = RgbImage::from_pixel(width, height, BG_COLOR);

    // Normalization: peak dB over the whole spectrogram, floor = peak - range.
    let peak_db = spec
        .frames
        .iter()
        .flat_map(|f| f.iter().copied())
        .fold(f32::NEG_INFINITY, f32::max)
        .max(-120.0);
    let db_range = 90.0_f32;
    let floor_db = peak_db - db_range;

    // Plot body: x = time (frame, bin-averaged to plot width), y = frequency
    // (low at bottom). Average each column's bins once, then paint rows from
    // that vector — not per output pixel, which would re-walk the frames
    // v_scale times over.
    let mut column = vec![floor_db; spec.bins];
    for px in 0..plot_w {
        let f0 = (px as usize * n_frames) / plot_w as usize;
        let f1 = (((px + 1) as usize * n_frames) / plot_w as usize)
            .max(f0 + 1)
            .min(spec.frames.len());
        let count = f1.saturating_sub(f0);
        if count > 0 {
            column.iter_mut().for_each(|v| *v = 0.0);
            for frame in &spec.frames[f0..f1] {
                for (acc, v) in column.iter_mut().zip(frame.iter()) {
                    *acc += v;
                }
            }
            column.iter_mut().for_each(|v| *v /= count as f32);
        } else {
            column.iter_mut().for_each(|v| *v = floor_db);
        }
        for py in 0..plot_h {
            let bin = (py / v_scale) as usize;
            let db = column.get(bin).copied().unwrap_or(floor_db);
            let t = ((db - floor_db) / db_range).clamp(0.0, 1.0);
            let y = MARGIN_TOP + plot_h - 1 - py;
            img.put_pixel(MARGIN_LEFT + px, y, heat_color(t));
        }
    }

    let nyquist_shown = spec.bins as f32 * spec.freq_step;
    // Frequency gridlines + labels at "nice" steps targeting ~6 lines.
    let freq_stepping = nice_step(nyquist_shown / 6.0);
    let mut f = freq_stepping;
    while f < nyquist_shown {
        let row = (f / nyquist_shown * plot_h as f32) as u32;
        if row < plot_h {
            let y = MARGIN_TOP + plot_h - 1 - row;
            for px in 0..plot_w {
                if px % 2 == 0 {
                    img.put_pixel(MARGIN_LEFT + px, y, GRID_COLOR);
                }
            }
            let label = format_freq(f);
            font::draw_text(
                &mut img,
                &label,
                (MARGIN_LEFT - 4 - font::text_width(&label)) as i64,
                y as i64 - 3,
                AXIS_COLOR,
            );
        }
        f += freq_stepping;
    }

    // Marker frequencies (solid, bright, labeled on the right side of the axis).
    for &mf in mark_freqs {
        if mf <= 0.0 || mf >= nyquist_shown {
            continue;
        }
        let row = (mf / nyquist_shown * plot_h as f32) as u32;
        let y = MARGIN_TOP + plot_h - 1 - row;
        for px in 0..plot_w {
            img.put_pixel(MARGIN_LEFT + px, y, MARKER_COLOR);
        }
        let label = format_freq(mf);
        font::draw_text(
            &mut img,
            &label,
            (MARGIN_LEFT - 4 - font::text_width(&label)) as i64,
            y as i64 - 3,
            MARKER_COLOR,
        );
    }

    // Time axis: ticks at nice intervals, labels offset by start_secs.
    let total_secs = n_frames as f64 * spec.time_step as f64;
    let t_step = nice_step((total_secs / 8.0) as f32).max(1e-3) as f64;
    let mut t = 0.0f64;
    while t <= total_secs {
        let px = ((t / total_secs) * (plot_w - 1) as f64) as u32;
        let x = MARGIN_LEFT + px;
        for dy in 0..4 {
            img.put_pixel(x, MARGIN_TOP + plot_h + dy, AXIS_COLOR);
        }
        let label = format_secs(start_secs + t);
        let lx = (x as i64 - font::text_width(&label) as i64 / 2).max(0);
        font::draw_text(&mut img, &label, lx, (MARGIN_TOP + plot_h + 6) as i64, AXIS_COLOR);
        t += t_step;
    }

    img
}

/// Round a raw step to the 1/2/5 decade ladder.
fn nice_step(raw: f32) -> f32 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f32.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

fn format_freq(hz: f32) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}kHz", k.round() as i64)
        } else {
            format!("{k:.1}kHz")
        }
    } else {
        format!("{}Hz", hz.round() as i64)
    }
}

fn format_secs(s: f64) -> String {
    if s >= 10.0 {
        format!("{s:.1}s")
    } else {
        format!("{s:.2}s")
    }
}

/// Dark-to-bright heat colormap (black → purple → orange → near-white).
fn heat_color(t: f32) -> Rgb<u8> {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [10.0, 8.0, 18.0]),
        (0.3, [70.0, 15.0, 110.0]),
        (0.6, [200.0, 55.0, 70.0]),
        (0.85, [250.0, 160.0, 40.0]),
        (1.0, [255.0, 250.0, 210.0]),
    ];
    let t = t.clamp(0.0, 1.0);
    for w in STOPS.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return Rgb([
                (c0[0] + (c1[0] - c0[0]) * f) as u8,
                (c0[1] + (c1[1] - c0[1]) * f) as u8,
                (c0[2] + (c1[2] - c0[2]) * f) as u8,
            ]);
        }
    }
    Rgb([255, 250, 210])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::generate_sine_wave;

    #[test]
    fn spectrogram_dimensions_and_peak_bin() {
        let rate = 48_000;
        let samples = generate_sine_wave(1200.0, 1.0, rate, 0.8);
        let params = SpectrogramParams::default();
        let spec = compute_spectrogram(&samples, rate, &params);

        let expected_frames = (samples.len() - params.fft_size) / params.hop + 1;
        assert_eq!(spec.frames.len(), expected_frames);
        assert_eq!(spec.bins, params.fft_size / 2 + 1);

        // Energy concentrates at ~1200 Hz in every frame
        let target_bin = (1200.0 / spec.freq_step).round() as usize;
        for frame in &spec.frames {
            let max_bin = frame
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            assert!((max_bin as i64 - target_bin as i64).unsigned_abs() <= 1);
        }
    }

    #[test]
    fn fmax_limits_bins() {
        let rate = 48_000;
        let samples = generate_sine_wave(440.0, 0.5, rate, 0.5);
        let params = SpectrogramParams {
            fmax: Some(4000.0),
            ..Default::default()
        };
        let spec = compute_spectrogram(&samples, rate, &params);
        assert!(spec.bins as f32 * spec.freq_step <= 4000.0 + 2.0 * spec.freq_step);
    }

    #[test]
    fn render_produces_image() {
        let rate = 48_000;
        let samples = generate_sine_wave(1200.0, 0.5, rate, 0.8);
        let spec = compute_spectrogram(&samples, rate, &SpectrogramParams::default());
        let img = render_spectrogram(&spec, 0.0, &[1200.0], 800);
        assert!(img.width() >= 800);
        assert!(img.height() >= 128);
    }

    #[test]
    fn empty_input_yields_no_frames() {
        let spec = compute_spectrogram(&[], 48_000, &SpectrogramParams::default());
        assert!(spec.frames.is_empty());
        // Render must not panic on empty data
        let _ = render_spectrogram(&spec, 0.0, &[], 800);
    }
}
