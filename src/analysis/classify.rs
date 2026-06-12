//! Coarse segment classification of record audio: silence, steady tones,
//! image-like line-periodic signal, and broadband content (music/noise).
//!
//! Classification per analysis window combines RMS (silence), spectral
//! flatness (tone vs broadband), and time-domain autocorrelation near the
//! nominal ~8.3 ms line period (image signal). Consecutive windows with the
//! same label are merged into segments.

use super::compute_spectrum;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentLabel {
    Silence,
    Tone,
    /// Line-periodic signal consistent with encoded image scan lines.
    ImagePeriodic,
    Broadband,
}

impl std::fmt::Display for SegmentLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SegmentLabel::Silence => "silence",
            SegmentLabel::Tone => "tone",
            SegmentLabel::ImagePeriodic => "image-periodic",
            SegmentLabel::Broadband => "broadband",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub label: SegmentLabel,
    /// Mean per-window confidence in [0, 1].
    pub confidence: f32,
    /// For `ImagePeriodic` segments: the detected line period in ms.
    pub period_ms: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ClassifyParams {
    /// Analysis window length in seconds.
    pub window_secs: f64,
    /// RMS below this is silence.
    pub silence_rms: f32,
    /// Nominal line duration to search around, ms.
    pub expected_line_ms: f32,
    /// Autocorrelation lag search range around nominal, as a fraction.
    pub period_tolerance: f32,
}

impl Default for ClassifyParams {
    fn default() -> Self {
        Self {
            window_secs: 0.25,
            silence_rms: 0.005,
            expected_line_ms: 8.32,
            period_tolerance: 0.35,
        }
    }
}

/// Classify `samples` into labeled time segments.
pub fn classify_segments(samples: &[f32], sample_rate: u32, params: &ClassifyParams) -> Vec<Segment> {
    let window = ((params.window_secs * sample_rate as f64) as usize).max(256);
    let mut windows: Vec<(f64, SegmentLabel, f32, Option<f32>)> = Vec::new();

    let mut start = 0usize;
    while start + window / 2 <= samples.len() {
        let end = (start + window).min(samples.len());
        let chunk = &samples[start..end];
        let t = start as f64 / sample_rate as f64;
        let (label, confidence, period) = classify_window(chunk, sample_rate, params);
        windows.push((t, label, confidence, period));
        start += window;
    }

    // Merge consecutive same-label windows.
    let mut segments: Vec<Segment> = Vec::new();
    let window_secs = window as f64 / sample_rate as f64;
    for (t, label, confidence, period) in windows {
        match segments.last_mut() {
            Some(seg) if seg.label == label => {
                let n = ((seg.end_secs - seg.start_secs) / window_secs).round() as f32;
                seg.confidence = (seg.confidence * n + confidence) / (n + 1.0);
                if let (Some(p_new), Some(p_old)) = (period, seg.period_ms) {
                    seg.period_ms = Some((p_old * n + p_new) / (n + 1.0));
                } else if seg.period_ms.is_none() {
                    seg.period_ms = period;
                }
                seg.end_secs = t + window_secs;
            }
            _ => segments.push(Segment {
                start_secs: t,
                end_secs: t + window_secs,
                label,
                confidence,
                period_ms: period,
            }),
        }
    }
    if let Some(last) = segments.last_mut() {
        last.end_secs = last.end_secs.min(samples.len() as f64 / sample_rate as f64);
    }
    segments
}

fn classify_window(chunk: &[f32], sample_rate: u32, params: &ClassifyParams) -> (SegmentLabel, f32, Option<f32>) {
    let n = chunk.len() as f64;
    let rms = (chunk.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / n).sqrt() as f32;
    if rms < params.silence_rms {
        let confidence = (1.0 - rms / params.silence_rms).clamp(0.5, 1.0);
        return (SegmentLabel::Silence, confidence, None);
    }

    // Tonality first: a steady tone is also autocorrelation-periodic, so it
    // must be ruled out before the line-period check. A tone concentrates its
    // spectral energy in a few bins around one peak; image signal spreads
    // energy across a harmonic comb at the line rate.
    const MAX_FFT: usize = 1 << 15;
    let spectrum = compute_spectrum(&chunk[..chunk.len().min(MAX_FFT)], sample_rate);
    let powers: Vec<f64> = spectrum.iter().skip(1).map(|&(_, m)| (m * m).max(1e-18)).collect();
    if powers.is_empty() {
        return (SegmentLabel::Broadband, 0.5, None);
    }
    let total: f64 = powers.iter().sum();
    let peak_bin = powers
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let lo = peak_bin.saturating_sub(2);
    let hi = (peak_bin + 3).min(powers.len());
    let peak_energy: f64 = powers[lo..hi].iter().sum();
    let peak_ratio = (peak_energy / total) as f32;
    if peak_ratio > 0.6 {
        return (SegmentLabel::Tone, peak_ratio.clamp(0.5, 1.0), None);
    }

    // Line-periodicity: normalized autocorrelation peak near the nominal period.
    let nominal_lag = (params.expected_line_ms / 1000.0 * sample_rate as f32) as usize;
    let lag_min = ((nominal_lag as f32 * (1.0 - params.period_tolerance)) as usize).max(2);
    let lag_max = (nominal_lag as f32 * (1.0 + params.period_tolerance)) as usize;
    if let Some((lag, corr)) = autocorr_peak(chunk, lag_min, lag_max) {
        if corr > 0.35 {
            let period_ms = lag as f32 / sample_rate as f32 * 1000.0;
            return (SegmentLabel::ImagePeriodic, corr.clamp(0.0, 1.0), Some(period_ms));
        }
    }

    // Remaining: spectral flatness separates noise-like from weakly-tonal
    // content; both land in Broadband, flatness just sets confidence.
    let log_mean = powers.iter().map(|p| p.ln()).sum::<f64>() / powers.len() as f64;
    let arith_mean = total / powers.len() as f64;
    let flatness = (log_mean.exp() / arith_mean) as f32;
    (SegmentLabel::Broadband, flatness.clamp(0.3, 1.0), None)
}

/// Strongest normalized autocorrelation in the lag range, computed over a
/// bounded prefix of the chunk for cost control.
fn autocorr_peak(chunk: &[f32], lag_min: usize, lag_max: usize) -> Option<(usize, f32)> {
    const MAX_SAMPLES: usize = 1 << 15;
    let data = &chunk[..chunk.len().min(MAX_SAMPLES)];
    if data.len() < lag_max * 2 {
        return None;
    }
    // Remove DC before correlating: a constant offset correlates perfectly at
    // every lag and would mask true periodicity.
    let mean = data.iter().sum::<f32>() / data.len() as f32;
    let energy: f64 = data.iter().map(|&s| ((s - mean) as f64).powi(2)).sum();
    if energy <= 0.0 {
        return None;
    }

    let mut best: Option<(usize, f32)> = None;
    for lag in lag_min..=lag_max.min(data.len() / 2) {
        let m = data.len() - lag;
        let mut acc = 0.0f64;
        for i in 0..m {
            acc += ((data[i] - mean) as f64) * ((data[i + lag] - mean) as f64);
        }
        // Normalize by full energy scaled to overlap length.
        let norm = energy * m as f64 / data.len() as f64;
        let corr = (acc / norm) as f32;
        if best.map(|(_, c)| corr > c).unwrap_or(true) {
            best = Some((lag, corr));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{generate_sine_wave, generate_white_noise};

    #[test]
    fn classifies_silence() {
        let samples = vec![0.0f32; 48_000];
        let segments = classify_segments(&samples, 48_000, &ClassifyParams::default());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].label, SegmentLabel::Silence);
    }

    #[test]
    fn classifies_pure_tone() {
        let samples = generate_sine_wave(440.0, 1.0, 48_000, 0.5);
        let segments = classify_segments(&samples, 48_000, &ClassifyParams::default());
        assert!(segments.iter().all(|s| s.label == SegmentLabel::Tone), "{segments:?}");
    }

    #[test]
    fn classifies_noise_as_broadband() {
        let samples = generate_white_noise(1.0, 48_000, 0.5);
        let segments = classify_segments(&samples, 48_000, &ClassifyParams::default());
        assert!(segments.iter().all(|s| s.label == SegmentLabel::Broadband), "{segments:?}");
    }

    #[test]
    fn classifies_line_periodic_signal() {
        // Repeating 400-sample scan-line shape at 48 kHz (8.33 ms period)
        let period = 400usize;
        let mut samples = Vec::new();
        for line in 0..240 {
            for i in 0..period {
                let v = match i {
                    0..=4 => 1.0,
                    5..=8 => -0.8,
                    _ => 0.3 * ((i as f32 / period as f32) * std::f32::consts::PI).sin() + 0.05 * ((line % 7) as f32 / 7.0),
                };
                samples.push(v);
            }
        }
        let segments = classify_segments(&samples, 48_000, &ClassifyParams::default());
        assert!(
            segments.iter().any(|s| s.label == SegmentLabel::ImagePeriodic),
            "{segments:?}"
        );
        let seg = segments.iter().find(|s| s.label == SegmentLabel::ImagePeriodic).unwrap();
        let period_ms = seg.period_ms.unwrap();
        assert!((period_ms - 8.33).abs() < 0.5, "period {period_ms} ms");
    }

    #[test]
    fn merges_consecutive_windows() {
        let mut samples = vec![0.0f32; 24_000];
        samples.extend(generate_sine_wave(440.0, 0.5, 48_000, 0.5));
        let segments = classify_segments(&samples, 48_000, &ClassifyParams::default());
        assert_eq!(segments.len(), 2, "{segments:?}");
        assert_eq!(segments[0].label, SegmentLabel::Silence);
        assert_eq!(segments[1].label, SegmentLabel::Tone);
    }
}
