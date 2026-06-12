//! Basic signal statistics for diagnosing record audio: levels, DC offset,
//! zero-crossing rate, crest factor, and dominant frequency.

use super::compute_spectrum;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalStats {
    pub rms: f32,
    pub peak: f32,
    pub dc_offset: f32,
    /// Zero crossings per second.
    pub zero_crossing_rate: f32,
    /// Peak-to-RMS ratio in dB.
    pub crest_db: f32,
    /// Frequency of the strongest spectral bin, Hz (0.0 for silence/empty).
    pub dominant_freq_hz: f32,
}

/// Compute statistics over the whole sample slice.
pub fn compute_stats(samples: &[f32], sample_rate: u32) -> SignalStats {
    if samples.is_empty() {
        return SignalStats {
            rms: 0.0,
            peak: 0.0,
            dc_offset: 0.0,
            zero_crossing_rate: 0.0,
            crest_db: 0.0,
            dominant_freq_hz: 0.0,
        };
    }

    let n = samples.len() as f64;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut peak = 0.0f32;
    let mut crossings = 0usize;

    for (i, &s) in samples.iter().enumerate() {
        sum += s as f64;
        sum_sq += (s as f64) * (s as f64);
        peak = peak.max(s.abs());
        if i > 0 && (s >= 0.0) != (samples[i - 1] >= 0.0) {
            crossings += 1;
        }
    }

    let rms = (sum_sq / n).sqrt() as f32;
    let duration_secs = n / sample_rate as f64;
    let zero_crossing_rate = (crossings as f64 / duration_secs) as f32;
    let crest_db = if rms > 0.0 { 20.0 * (peak / rms).log10() } else { 0.0 };

    // Dominant frequency from a one-shot spectrum, skipping the DC bin. Cap the
    // FFT length to bound cost on long inputs.
    const MAX_FFT: usize = 1 << 18;
    let fft_slice = &samples[..samples.len().min(MAX_FFT)];
    let dominant_freq_hz = compute_spectrum(fft_slice, sample_rate)
        .iter()
        .skip(1)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|&(f, _)| f as f32)
        .unwrap_or(0.0);

    SignalStats {
        rms,
        peak,
        dc_offset: (sum / n) as f32,
        zero_crossing_rate,
        crest_db,
        dominant_freq_hz,
    }
}

/// Compute stats over consecutive windows of `window_secs`, returning
/// `(window_start_secs, stats)` rows. The final partial window is included
/// when it is at least a quarter of the window length.
pub fn rolling_stats(samples: &[f32], sample_rate: u32, window_secs: f64) -> Vec<(f64, SignalStats)> {
    let window = ((window_secs * sample_rate as f64) as usize).max(1);
    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < samples.len() {
        let end = (start + window).min(samples.len());
        if end - start >= window / 4 {
            rows.push((
                start as f64 / sample_rate as f64,
                compute_stats(&samples[start..end], sample_rate),
            ));
        }
        start += window;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::generate_sine_wave;

    #[test]
    fn sine_stats_match_theory() {
        let rate = 48_000;
        let amp = 0.5;
        let samples = generate_sine_wave(1000.0, 1.0, rate, amp);
        let stats = compute_stats(&samples, rate);

        // RMS of a sine = amplitude / sqrt(2)
        assert!((stats.rms - amp / 2f32.sqrt()).abs() < 0.01);
        assert!((stats.peak - amp).abs() < 0.01);
        assert!(stats.dc_offset.abs() < 0.001);
        // A 1 kHz sine crosses zero 2000 times per second
        assert!((stats.zero_crossing_rate - 2000.0).abs() < 50.0);
        // Crest factor of a sine = 3.01 dB
        assert!((stats.crest_db - 3.01).abs() < 0.2);
        assert!((stats.dominant_freq_hz - 1000.0).abs() < 5.0);
    }

    #[test]
    fn silence_stats_are_zero() {
        let stats = compute_stats(&vec![0.0; 48_000], 48_000);
        assert_eq!(stats.rms, 0.0);
        assert_eq!(stats.peak, 0.0);
    }

    #[test]
    fn rolling_windows_cover_input() {
        let rate = 48_000;
        let samples = generate_sine_wave(440.0, 2.0, rate, 0.5);
        let rows = rolling_stats(&samples, rate, 0.5);
        assert_eq!(rows.len(), 4);
        assert!((rows[1].0 - 0.5).abs() < 1e-9);
    }
}
