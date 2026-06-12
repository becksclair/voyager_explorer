//! Time-domain scan-line sync detection.
//!
//! Reference decoders for the Voyager record (foodini/voyager,
//! amazing-rando/voyager-decoder) locate each scan line by finding a large
//! positive spike followed by a falling edge; the bottom of that edge marks
//! the line start. This module implements that approach: peak picking with a
//! minimum-distance constraint, then a forward search for the local minimum.

#[derive(Debug, Clone)]
pub struct SyncParams {
    /// Nominal line duration in milliseconds (Voyager: ~8.32 ms).
    pub expected_line_ms: f32,
    /// Peak threshold as a fraction of the robust signal maximum.
    pub peak_height: f32,
    /// Minimum spacing between accepted peaks, as a fraction of the nominal
    /// line period. Guards against double-triggering on one sync spike.
    pub min_spacing_frac: f32,
    /// How far past the peak to search for the falling-edge minimum, as a
    /// fraction of the nominal line period.
    pub edge_search_frac: f32,
}

impl Default for SyncParams {
    fn default() -> Self {
        Self {
            expected_line_ms: 8.32,
            peak_height: 0.45,
            min_spacing_frac: 0.7,
            edge_search_frac: 0.25,
        }
    }
}

/// Detect scan-line start positions (sample indices into `samples`).
///
/// Returns line starts at the bottom of each sync spike's falling edge.
pub fn detect_line_syncs(samples: &[f32], sample_rate: u32, params: &SyncParams) -> Vec<usize> {
    let period = (params.expected_line_ms / 1000.0 * sample_rate as f32) as usize;
    if period == 0 || samples.len() < period * 2 {
        return Vec::new();
    }
    let min_spacing = ((period as f32 * params.min_spacing_frac) as usize).max(1);
    let edge_search = ((period as f32 * params.edge_search_frac) as usize).max(2);

    // Robust maximum: 99.9th percentile of |sample| avoids a single corrupt
    // sample dominating the threshold.
    let robust_max = percentile_abs(samples, 0.999);
    if robust_max <= 0.0 {
        return Vec::new();
    }
    let threshold = robust_max * params.peak_height;

    // Peak picking: local maxima above threshold, greedily enforcing spacing.
    let mut peaks: Vec<usize> = Vec::new();
    let mut i = 1;
    while i + 1 < samples.len() {
        let s = samples[i];
        if s > threshold && s >= samples[i - 1] && s >= samples[i + 1] {
            match peaks.last() {
                Some(&last) if i - last < min_spacing => {
                    // Within the dead zone: keep whichever peak is taller.
                    if s > samples[last] {
                        *peaks.last_mut().unwrap() = i;
                    }
                }
                _ => peaks.push(i),
            }
        }
        i += 1;
    }

    // Falling edge: line start = minimum within the search window after the peak.
    peaks
        .into_iter()
        .map(|p| {
            let end = (p + edge_search).min(samples.len());
            let mut min_idx = p;
            let mut min_val = samples[p];
            for (j, &v) in samples.iter().enumerate().take(end).skip(p) {
                if v < min_val {
                    min_val = v;
                    min_idx = j;
                }
            }
            min_idx
        })
        .collect()
}

/// Summary of intervals between consecutive sync positions.
#[derive(Debug, Clone)]
pub struct IntervalSummary {
    pub count: usize,
    pub median_samples: f64,
    pub mean_samples: f64,
    pub std_samples: f64,
    pub min_samples: usize,
    pub max_samples: usize,
    /// Median interval expressed in milliseconds.
    pub median_ms: f64,
}

/// Summarize the spacing of sync positions. Returns `None` with fewer than
/// two positions.
pub fn interval_summary(positions: &[usize], sample_rate: u32) -> Option<IntervalSummary> {
    if positions.len() < 2 {
        return None;
    }
    let intervals: Vec<usize> = positions.windows(2).map(|w| w[1] - w[0]).collect();
    let mut sorted = intervals.clone();
    sorted.sort_unstable();
    let median_samples = if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) as f64 / 2.0
    } else {
        sorted[sorted.len() / 2] as f64
    };
    let mean = intervals.iter().sum::<usize>() as f64 / intervals.len() as f64;
    let var = intervals.iter().map(|&i| (i as f64 - mean).powi(2)).sum::<f64>() / intervals.len() as f64;

    Some(IntervalSummary {
        count: intervals.len(),
        median_samples,
        mean_samples: mean,
        std_samples: var.sqrt(),
        min_samples: *sorted.first().unwrap(),
        max_samples: *sorted.last().unwrap(),
        median_ms: median_samples / sample_rate as f64 * 1000.0,
    })
}

fn percentile_abs(samples: &[f32], pct: f64) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut mags: Vec<f32> = samples.iter().map(|s| s.abs()).collect();
    let idx = ((mags.len() as f64 - 1.0) * pct) as usize;
    let (_, nth, _) = mags.select_nth_unstable_by(idx, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    *nth
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic line signal: positive sync spike, falling edge to a dip, then
    /// mid-level "image" content for the rest of the line.
    fn synthetic_lines(n_lines: usize, period: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n_lines * period);
        for line in 0..n_lines {
            for i in 0..period {
                let v = match i {
                    0..=4 => 1.0,                                         // sync spike
                    5..=8 => -0.8,                                        // falling-edge dip = line start
                    _ => 0.2 + 0.2 * ((line * 7 + i) % 13) as f32 / 13.0, // content
                };
                out.push(v);
            }
        }
        out
    }

    #[test]
    fn detects_line_starts_at_nominal_period() {
        let rate = 48_000;
        let period = 400; // 8.33 ms at 48 kHz
        let n_lines = 50;
        let samples = synthetic_lines(n_lines, period);

        let positions = detect_line_syncs(&samples, rate, &SyncParams::default());
        assert!(
            (positions.len() as i64 - n_lines as i64).abs() <= 2,
            "expected ~{n_lines} syncs, got {}",
            positions.len()
        );

        let summary = interval_summary(&positions, rate).unwrap();
        assert!(
            (summary.median_samples - period as f64).abs() < 2.0,
            "median interval {} != {period}",
            summary.median_samples
        );
        assert!((summary.median_ms - 8.33).abs() < 0.1);
    }

    #[test]
    fn line_start_is_after_peak() {
        let samples = synthetic_lines(10, 400);
        let positions = detect_line_syncs(&samples, 48_000, &SyncParams::default());
        // Spike occupies samples 0..=4 of each line; the dip 5..=8. Starts must
        // land in the dip, not on the spike.
        for &p in &positions {
            let offset = p % 400;
            assert!((5..=8).contains(&offset), "line start at offset {offset} not in dip");
        }
    }

    #[test]
    fn silence_yields_no_syncs() {
        let samples = vec![0.0f32; 48_000];
        assert!(detect_line_syncs(&samples, 48_000, &SyncParams::default()).is_empty());
    }

    #[test]
    fn interval_summary_requires_two_positions() {
        assert!(interval_summary(&[100], 48_000).is_none());
    }
}
