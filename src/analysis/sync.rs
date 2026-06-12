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
    peaks.into_iter().map(|p| falling_edge_min(samples, p, edge_search)).collect()
}

/// Index of the minimum sample in `[peak, peak + edge_search)` — the bottom
/// of the sync spike's falling edge, which marks the line start. First
/// minimum wins on ties.
fn falling_edge_min(samples: &[f32], peak: usize, edge_search: usize) -> usize {
    let end = (peak + edge_search).min(samples.len());
    let mut min_idx = peak;
    for j in peak..end {
        if samples[j] < samples[min_idx] {
            min_idx = j;
        }
    }
    min_idx
}

/// Tracker tuning constants. The detector's analogous knobs live on
/// [`SyncParams`]; these stay module-level because no caller tunes them yet.
///
/// Search radius around each predicted sync, as a fraction of the period.
/// Tight enough that content peaks elsewhere in the line cannot capture the
/// lock (reference decoders use 1-6%).
const TRACK_SEARCH_FRAC: f64 = 0.06;
/// Falling-edge search length after the spike, as a fraction of the period.
/// The dip follows the spike within a few samples; searching further lets
/// dark content masquerade as the falling edge.
const TRACK_EDGE_FRAC: f64 = 0.05;
/// EMA weight for adapting the period from an accepted lock. Gentle, so
/// locks at the tolerance edge cannot drag the cadence estimate.
const PERIOD_EMA_ALPHA: f64 = 0.05;
/// A re-anchor needs a spike-to-dip swing above this fraction of the
/// detector threshold; a full-threshold swing also counts as a strong lock.
const SWING_ANCHOR_FACTOR: f32 = 0.6;
/// Accepted re-anchor intervals must lie within this fraction of the period.
const TRACK_INTERVAL_TOL: f64 = 0.06;

/// Track scan-line sync positions with a predictive lock.
///
/// The global detector ([`detect_line_syncs`]) picks the tallest peaks
/// anywhere, which mistimes or drops lines whenever image content rivals
/// the sync spike (dark photographs, dropouts). Reference decoders instead
/// predict each next sync at `previous + period` and search only a narrow
/// window around the prediction, coasting on the prediction when no
/// credible peak appears. This does that: it seeds from the global
/// detector's first coherent position, then walks the buffer line by line,
/// slowly adapting the period estimate (slant) from accepted peaks.
pub fn track_line_syncs(samples: &[f32], sample_rate: u32, params: &SyncParams) -> Vec<usize> {
    let detected = detect_line_syncs(samples, sample_rate, params);
    let Some(summary) = interval_summary(&detected, sample_rate) else {
        return detected;
    };

    let nominal = (params.expected_line_ms / 1000.0 * sample_rate as f32) as f64;
    // Start from the detected cadence when it is plausibly the line cadence.
    // When it is not — weak-sync lines get swallowed by the detector's
    // min-spacing dedup, doubling the apparent median — trust the nominal
    // period and let per-line locking prove itself (the lock-fraction guard
    // below rejects the result if it can't).
    let mut period = if (summary.median_samples - nominal).abs() / nominal < 0.3 {
        summary.median_samples
    } else {
        nominal
    };

    let robust_max = percentile_abs(samples, 0.999);
    if robust_max <= 0.0 {
        return detected;
    }
    let threshold = robust_max * params.peak_height;
    let search = ((period * TRACK_SEARCH_FRAC) as usize).max(2);
    let edge_search = ((period * TRACK_EDGE_FRAC) as usize).max(4);

    // Seed at the first detected sync that starts a coherent pair.
    let seed = detected
        .windows(2)
        .find(|w| ((w[1] - w[0]) as f64 - period).abs() / period < 0.1)
        .map(|w| w[0])
        .unwrap_or(detected[0]);

    let mut positions = vec![seed];
    let mut pos = seed as f64;
    let mut locked = 0usize;
    loop {
        let predict = pos + period;
        if predict + period > samples.len() as f64 {
            break;
        }
        let center = predict.round() as usize;
        let lo = center.saturating_sub(search);
        let hi = (center + search).min(samples.len() - 1);

        // Always take the window maximum as the spike candidate — real sync
        // spikes persist even in dark lines, just attenuated, and the tight
        // window is what keeps content peaks from capturing the lock.
        // Thresholding here would force long coasts through dark regions,
        // and the accumulated period error shears the image.
        // Ties (flat plateaus) break toward the prediction so featureless
        // stretches don't walk the anchor to the window edge.
        let peak = (lo..=hi)
            .max_by(|&a, &b| {
                samples[a]
                    .partial_cmp(&samples[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.abs_diff(center).cmp(&a.abs_diff(center)))
            })
            .expect("window is non-empty");

        // Falling edge after the peak marks the true line start.
        let min_idx = falling_edge_min(samples, peak, edge_search);
        // Re-anchor only on a sync-like event: a genuine spike-to-dip swing
        // (attenuated syncs in dark lines still swing harder than content
        // noise) at a plausible interval. Anchoring on anything weaker
        // randomizes line starts in dark regions; coasting there is what
        // keeps the image coherent.
        let swing = samples[peak] - samples[min_idx];
        let interval = min_idx as f64 - pos;
        let next = if swing > threshold * SWING_ANCHOR_FACTOR && (interval - period).abs() / period < TRACK_INTERVAL_TOL {
            period = period * (1.0 - PERIOD_EMA_ALPHA) + interval * PERIOD_EMA_ALPHA;
            if swing > threshold {
                locked += 1;
            }
            min_idx as f64
        } else {
            // No credible sync (dark line, dropout, boundary junk): coast.
            predict
        };
        positions.push(next.round() as usize);
        pos = next;
    }

    // A track that mostly coasted never found real line structure (music,
    // noise): hand back the raw detections so the caller's cadence check
    // can reject sync lock and fall through to fixed-period slicing.
    if locked < positions.len() / 4 {
        return detected;
    }
    positions
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
    let median_samples = median_of_sorted(&sorted);
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

/// Median of an already-sorted slice (even-length average, odd-length pick).
/// Caller guarantees `sorted` is non-empty.
pub(crate) fn median_of_sorted(sorted: &[usize]) -> f64 {
    if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) as f64 / 2.0
    } else {
        sorted[sorted.len() / 2] as f64
    }
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

    /// Lines where some sync spikes are attenuated and the content holds a
    /// bright distractor peak mid-line: the global detector mistimes these,
    /// the tracker must hold the cadence.
    fn weak_sync_lines(n_lines: usize, period: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n_lines * period);
        for line in 0..n_lines {
            let weak = line % 3 == 1;
            for i in 0..period {
                let v = match i {
                    0..=4 => {
                        if weak {
                            0.25
                        } else {
                            1.0
                        }
                    }
                    5..=8 => -0.8,
                    200..=210 if weak => 0.95, // bright content distractor
                    _ => 0.2,
                };
                out.push(v);
            }
        }
        out
    }

    #[test]
    fn tracker_holds_cadence_through_weak_syncs() {
        let period = 400usize;
        let n_lines = 60;
        let samples = weak_sync_lines(n_lines, period);

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        assert!(
            (tracked.len() as i64 - n_lines as i64).abs() <= 3,
            "expected ~{n_lines} lines, got {}",
            tracked.len()
        );
        // Every tracked start must sit at the line cadence (dip region of
        // its line), within a couple of samples of coasting error.
        for &p in &tracked {
            let offset = p % period;
            assert!((4..=11).contains(&offset), "tracked start at offset {offset} is off-cadence");
        }

        // Sanity: the global detector alone fails this fixture — its
        // min-spacing dedup swallows the weak lines (taller distractor or
        // neighbor wins), losing roughly every third line. If this starts
        // passing, the fixture stopped stressing the tracker.
        let detected = detect_line_syncs(&samples, 48_000, &SyncParams::default());
        let mistimed = detected.iter().filter(|&&p| !(4..=11).contains(&(p % period))).count();
        assert!(
            mistimed > 0 || detected.len() < n_lines - 5,
            "global detector unexpectedly perfect on weak-sync fixture ({} positions)",
            detected.len()
        );
    }

    #[test]
    fn tracker_matches_detector_on_clean_signal() {
        let samples = synthetic_lines(50, 400);
        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        let summary = interval_summary(&tracked, 48_000).unwrap();
        assert!((summary.median_samples - 400.0).abs() < 2.0);
        assert!(summary.std_samples < 3.0, "jitter {}", summary.std_samples);
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
