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

    // Create a planner
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(n);

    // Prepare input and output buffers
    let mut input_vector = samples.to_vec();
    let mut output_vector = r2c.make_output_vec();

    // Apply a Hamming window to reduce spectral leakage
    for (i, sample) in input_vector.iter_mut().enumerate() {
        let window =
            0.54 - 0.46 * ((2.0 * std::f32::consts::PI * i as f32) / (n as f32 - 1.0)).cos();
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

        // Convert to dB scale (optional, but usually better for visualization)
        // let db = 20.0 * normalized_magnitude.log10();

        let frequency = (i as f64 * sample_rate as f64) / n as f64;
        spectrum.push((frequency, normalized_magnitude as f64));
    }

    spectrum
}
