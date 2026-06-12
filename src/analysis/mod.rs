//! Signal analysis and diagnostics: one-shot spectra, spectrograms, rolling
//! statistics, segment classification, and scan-line sync detection.
//!
//! Everything here is pure library code; the CLI subcommands and the GUI
//! diagnostics panel are thin shims over these functions.

pub mod classify;
mod font;
pub mod segment;
pub mod spectrogram;
pub mod stats;
pub mod sync;

pub use classify::{classify_segments, ClassifyParams, Segment, SegmentLabel};
pub use segment::{find_image_bounds, ImageBounds, SegmentImagesParams};
pub use spectrogram::{compute_spectrogram, render_spectrogram, Spectrogram, SpectrogramParams};
pub use stats::{compute_stats, rolling_stats, SignalStats};
pub use sync::{detect_line_syncs, interval_summary, IntervalSummary, SyncParams};

use realfft::RealFftPlanner;

/// Compute the magnitude spectrum of a signal.
///
/// # Arguments
/// * `samples` - The input audio samples.
/// * `sample_rate` - The sample rate of the audio.
///
/// # Returns
/// A vector of (frequency, magnitude) tuples.
pub fn compute_spectrum(samples: &[f32], sample_rate: u32) -> Vec<(f64, f64)> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }

    // Reuse one planner per thread: RealFftPlanner caches plans per length,
    // so repeat callers (the live spectrum panel repaints at frame rate)
    // skip the expensive plan construction.
    thread_local! {
        static PLANNER: std::cell::RefCell<RealFftPlanner<f32>> = std::cell::RefCell::new(RealFftPlanner::new());
    }
    let r2c = PLANNER.with(|p| p.borrow_mut().plan_fft_forward(n));

    // Prepare input and output buffers
    let mut input_vector = samples.to_vec();
    let mut output_vector = r2c.make_output_vec();

    // Apply a Hamming window to reduce spectral leakage
    for (i, sample) in input_vector.iter_mut().enumerate() {
        let window = 0.54 - 0.46 * ((2.0 * std::f32::consts::PI * i as f32) / (n as f32 - 1.0)).cos();
        *sample *= window;
    }

    // Process FFT
    if r2c.process(&mut input_vector, &mut output_vector).is_err() {
        return Vec::new();
    }

    // Compute magnitude and frequency for each bin
    // We only need the first n/2 + 1 bins (Nyquist)
    let output_len = output_vector.len();
    let mut spectrum = Vec::with_capacity(output_len);

    for (i, complex_val) in output_vector.iter().enumerate() {
        let magnitude = complex_val.norm();
        // Normalize magnitude
        let normalized_magnitude = magnitude / n as f32;

        // Return linear magnitude - let the UI decide whether to convert to dB
        // This preserves the user's choice between linear and dB scale views.
        let frequency = (i as f64 * sample_rate as f64) / n as f64;
        spectrum.push((frequency, normalized_magnitude as f64));
    }

    spectrum
}
