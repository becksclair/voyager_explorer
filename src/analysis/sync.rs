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
/// Half-width (in lines) of the residual moving average in
/// [`smooth_line_jitter`]. The window must be wide enough to average out the
/// per-line argmin noise (±several samples) yet narrow enough to follow the
/// real slant/wow of the cadence.
const JITTER_SMOOTH_HALF: usize = 10;
/// An interval beyond this multiple of the period starts a new run for
/// jitter smoothing — image boundaries and dropouts must not be averaged
/// across (their gaps are real, not noise).
const JITTER_RUN_GAP: f64 = 1.4;
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
    smooth_line_jitter(&positions, period)
}

/// Low-pass the per-line detection jitter out of tracked sync positions while
/// preserving the slow cadence (period, slant, tape wow) and any real gaps.
///
/// The falling-edge argmin jitters ±several samples per line; the tracker's
/// tight window keeps that from drifting the lock, but it still snaps each
/// accepted line start onto the noisy argmin. Used directly as line
/// boundaries those positions render the noise as feathered edges and
/// vertical streaks. Within each gap-free run, fit a line to
/// position-vs-index and replace each residual with its moving average:
/// only structure slower than the window (the true cadence) survives, and
/// the linear-phase average introduces no drift or ringing.
fn smooth_line_jitter(positions: &[usize], period: f64) -> Vec<usize> {
    if positions.len() < 3 || period <= 0.0 {
        return positions.to_vec();
    }
    let gap = period * JITTER_RUN_GAP;
    let mut out = positions.to_vec();
    let mut run_start = 0usize;
    for i in 1..=positions.len() {
        let is_break = i == positions.len() || (positions[i] - positions[i - 1]) as f64 > gap;
        if is_break {
            smooth_run(&positions[run_start..i], &mut out[run_start..i]);
            run_start = i;
        }
    }
    out
}

/// Detrend-smooth-retrend one gap-free run of positions in place. Runs of
/// fewer than three lines (too few to fit a trend) are left as-is.
///
/// On a run long enough to support it the trend is a *quadratic* in line index:
/// a constant period is linear, and analog tape slant (the per-line period
/// drifting) makes the line positions curve. A linear detrend would leave that
/// curvature in the residual, where the moving average flattens it and
/// reintroduces a slant error; the quadratic absorbs it so the residual is the
/// per-line noise the average is meant to remove (plus slow wow, which it
/// preserves). Short runs instead use a linear detrend — see the trend block
/// below for why a quadratic overfits there.
///
/// The residual is averaged over a centered window of half-width
/// `min(JITTER_SMOOTH_HALF, (n-1)/2)`, so a short run (e.g. between dropouts)
/// is still smoothed as much as its length allows rather than skipped, and the
/// window never overruns the run. Out-of-range window indices are mirrored
/// back into the run (reflection padding), so the first and last lines average
/// a full-width window and get the same noise reduction as the interior — a
/// truncated edge window would smooth the run ends less.
fn smooth_run(run: &[usize], out: &mut [usize]) {
    let n = run.len();
    if n < 3 {
        return;
    }
    // Center the index (t = k - m) so the odd moments vanish by symmetry,
    // leaving a well-conditioned 2×2 solve for the even coefficients.
    let m = (n as f64 - 1.0) / 2.0;
    let (mut s2, mut s4) = (0.0f64, 0.0f64);
    let (mut sp, mut spt, mut spt2) = (0.0f64, 0.0f64, 0.0f64);
    for (k, &p) in run.iter().enumerate() {
        let t = k as f64 - m;
        let t2 = t * t;
        let p = p as f64;
        s2 += t2;
        s4 += t2 * t2;
        sp += p;
        spt += p * t;
        spt2 += p * t2;
    }
    if s2 <= 0.0 {
        return; // n >= 3 guarantees s2 > 0; defensive against a degenerate run.
    }
    // The trend order adapts to the run length. A quadratic captures real tape
    // slant (the per-line period drifting makes line positions curve), but
    // separating that curvature from per-line noise needs enough points: on a
    // short run the curvature is both negligible (slant × span² is tiny) and
    // unidentifiable, so a least-squares quadratic overfits the noise and the
    // moving average then amplifies jitter instead of removing it. Use a linear
    // detrend there — it leaves more residual for the average and injects no
    // spurious curvature. Centered indices decouple the fits: `b = spt/s2`
    // either way; only the constant and (for long runs) quadratic terms differ.
    let b = spt / s2;
    let (a, c) = if n > 2 * JITTER_SMOOTH_HALF {
        let det = n as f64 * s4 - s2 * s2;
        if det.abs() <= f64::EPSILON {
            return;
        }
        // Quadratic trend p ≈ a + b·t + c·t².
        ((sp * s4 - spt2 * s2) / det, (n as f64 * spt2 - s2 * sp) / det)
    } else {
        // Linear trend p ≈ a + b·t (centered ⇒ the constant is the mean).
        (sp / n as f64, 0.0)
    };
    let trend = |k: usize| {
        let t = k as f64 - m;
        a + b * t + c * t * t
    };
    let resid: Vec<f64> = run.iter().enumerate().map(|(k, &p)| p as f64 - trend(k)).collect();

    // Centered moving average of the residual; trend + smoothed residual keeps
    // the slow cadence (slant, wow) and drops the per-line argmin noise. The
    // half-width is capped at (n-1)/2 so the window fits the run, and indices
    // that fall off either end are reflected back inside (mirror about the
    // endpoint) so every position averages a full-width `2*half + 1` window.
    // Because `half <= (n-1)/2`, an out-of-range index is at most `half` past
    // an edge, so a single reflection always lands within `0..n`.
    //
    // Force strictly increasing output: these become line boundaries
    // (`positions[i]..positions[i+1]`) downstream, and a non-increasing pair
    // would underflow the interval subtraction. At the real line cadence the
    // trend's per-step rise (~period) dwarfs the residual wiggle so the clamp
    // never fires; it only guards the degenerate small-period case.
    let half = JITTER_SMOOTH_HALF.min((n - 1) / 2);
    let window = (2 * half + 1) as f64;
    let reflect = |idx: isize| -> usize {
        if idx < 0 {
            (-idx) as usize
        } else if idx >= n as isize {
            (2 * (n as isize - 1) - idx) as usize
        } else {
            idx as usize
        }
    };
    let mut prev: Option<usize> = None;
    for (k, slot) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        for j in 0..=2 * half {
            let idx = k as isize + j as isize - half as isize;
            sum += resid[reflect(idx)];
        }
        let rs = sum / window;
        let mut value = (trend(k) + rs).round().max(0.0) as usize;
        if let Some(p) = prev {
            value = value.max(p + 1);
        }
        *slot = value;
        prev = Some(value);
    }
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
    fn smoothing_cuts_jitter_and_keeps_slant() {
        let n = 200usize;
        let period = 399.0;
        // Smooth true cadence with a constant slant.
        let slant = 0.02; // per-line period growth -> quadratic line positions
        let truth = |k: usize| 1000.0 + period * k as f64 + 0.5 * slant * (k as f64).powi(2);
        // Deterministic per-line jitter in {-4..=4} samples on top of the truth.
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let noisy: Vec<usize> = (0..n).map(|k| (truth(k) + jitter(k)).round() as usize).collect();

        let smoothed = smooth_line_jitter(&noisy, period);

        assert_eq!(smoothed.len(), noisy.len());
        assert!(
            smoothed.windows(2).all(|w| w[1] > w[0]),
            "line boundaries must stay increasing"
        );

        let rms = |v: &[usize]| -> f64 {
            (v.iter().enumerate().map(|(k, &p)| (p as f64 - truth(k)).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };
        let before = rms(&noisy);
        let after = rms(&smoothed);
        // Most of the per-line jitter is averaged out...
        assert!(
            after < before * 0.5,
            "jitter not reduced: before={before:.2} after={after:.2}"
        );
        // ...and the slant is preserved (a flattened trend would blow this up
        // to tens of samples over the run, not stay near zero).
        assert!(after < 1.5, "slant not tracked: residual rms {after:.2}");
    }

    #[test]
    fn smooth_run_output_is_strictly_increasing_at_small_period() {
        // Degenerate cadence: ~1 sample/line with ±4 jitter. The trend's
        // per-step rise is tiny, so without the monotonic clamp the residual
        // moving average can produce equal/decreasing boundaries — which would
        // underflow the interval subtraction in segment_lines.
        let n = 60usize;
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let run: Vec<usize> = (0..n).map(|k| (1000.0 + k as f64 + jitter(k)).round() as usize).collect();
        let mut out = run.clone();
        smooth_run(&run, &mut out);
        assert!(
            out.windows(2).all(|w| w[1] > w[0]),
            "smoothed positions must be strictly increasing: {out:?}"
        );
        // Bounded divergence: with a real (slope ~1) trend the clamp aligns
        // with the natural rise, so smoothing must only nudge each boundary by
        // the per-line jitter (±4) plus rounding and the odd +1 clamp step —
        // never drag it far. A regression that let the trend or clamp run away
        // (e.g. the flat-run +1/step accumulation leaking onto a real cadence)
        // would blow well past this. Measured max divergence here is 5.
        let max_div = run.iter().zip(&out).map(|(r, o)| r.abs_diff(*o)).max().unwrap();
        assert!(max_div <= 8, "smoothing dragged a boundary {max_div} samples from input");
    }

    #[test]
    fn smoothing_splits_on_gaps_and_smooths_each_run() {
        // Two gap-free runs separated by a real image-boundary gap (well over
        // JITTER_RUN_GAP × period). Exercises the run-splitting loop in
        // `smooth_line_jitter`: each run must be smoothed independently, the
        // boundary gap must survive (not be averaged across), and the whole
        // sequence must stay strictly increasing.
        let period = 399.0;
        let n = 80usize;
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let run_truth = |base: f64, k: usize| base + period * k as f64;

        let mut noisy: Vec<usize> = (0..n).map(|k| (run_truth(1000.0, k) + jitter(k)).round() as usize).collect();
        // Boundary gap = 5× period, far above the 1.4× run-break threshold.
        let gap_base = *noisy.last().unwrap() as f64 + 5.0 * period;
        noisy.extend((0..n).map(|k| (run_truth(gap_base, k) + jitter(k)).round() as usize));

        let smoothed = smooth_line_jitter(&noisy, period);

        assert_eq!(smoothed.len(), noisy.len());
        assert!(
            smoothed.windows(2).all(|w| w[1] > w[0]),
            "boundaries across both runs must stay increasing"
        );
        // The boundary gap is preserved: the step at the run break stays large,
        // proving the runs were not smoothed across.
        let break_step = smoothed[n] - smoothed[n - 1];
        assert!(
            break_step as f64 > 3.0 * period,
            "image-boundary gap was averaged away: step {break_step} at the break"
        );
        // Each run is independently de-jittered (residual rms well under the
        // ±4-sample raw jitter).
        let run_rms = |off: usize, base: f64| -> f64 {
            (0..n)
                .map(|k| (smoothed[off + k] as f64 - run_truth(base, k)).powi(2))
                .sum::<f64>()
                / n as f64
        };
        assert!(run_rms(0, 1000.0).sqrt() < 2.0, "first run not smoothed");
        assert!(run_rms(n, gap_base).sqrt() < 2.0, "second run not smoothed");
    }

    #[test]
    fn smoothing_reduces_jitter_on_short_runs() {
        // A run shorter than the full 2*JITTER_SMOOTH_HALF+1 window (but at
        // least 3 lines) must still be smoothed via the adaptive window, not
        // skipped, and must never *amplify* jitter. n=4 and n=6 are the lengths
        // where a least-squares quadratic overfit (ate too many degrees of
        // freedom) and the moving average increased rms; the linear detrend for
        // short runs fixes that. n=10 should improve substantially.
        let period = 399.0;
        let truth = |k: usize| 1000.0 + period * k as f64;
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let rms = |v: &[usize]| -> f64 {
            (v.iter().enumerate().map(|(k, &p)| (p as f64 - truth(k)).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };

        for n in [4usize, 6, 10] {
            let noisy: Vec<usize> = (0..n).map(|k| (truth(k) + jitter(k)).round() as usize).collect();
            let smoothed = smooth_line_jitter(&noisy, period);

            assert_eq!(smoothed.len(), n);
            assert!(
                smoothed.windows(2).all(|w| w[1] > w[0]),
                "n={n}: short run must stay strictly increasing"
            );
            let (before, after) = (rms(&noisy), rms(&smoothed));
            // Must reduce, not amplify: the quadratic-overfit regression pushed
            // n=4 to ~1.18x and n=6 to ~1.06x; the linear detrend keeps every
            // length below 1 (measured n=4 0.66, n=6 0.34, n=10 0.26).
            assert!(after < before, "n={n}: jitter not reduced: {before:.2} -> {after:.2}");
        }

        // The longer short run is smoothed substantially, not just nudged.
        let noisy: Vec<usize> = (0..10).map(|k| (truth(k) + jitter(k)).round() as usize).collect();
        let after = rms(&smooth_line_jitter(&noisy, period));
        assert!(after < rms(&noisy) * 0.6, "n=10 weakly smoothed: {after:.2}");
    }

    #[test]
    fn smoothing_tracks_slant_for_odd_and_long_runs() {
        // The quadratic solve and slant preservation must hold regardless of
        // run length or index parity: odd n (half-integer center cancels the
        // odd moments the same way) and a long run (where curvature accumulates
        // and f64 conditioning could in principle degrade) are both covered.
        // The existing slant test uses only even n=200.
        let period = 399.0;
        let slant = 0.02; // per-line period growth -> quadratic line positions
        let truth = |k: usize| 1000.0 + period * k as f64 + 0.5 * slant * (k as f64).powi(2);
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let rms = |v: &[usize]| -> f64 {
            (v.iter().enumerate().map(|(k, &p)| (p as f64 - truth(k)).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };

        for &n in &[201usize, 601] {
            let noisy: Vec<usize> = (0..n).map(|k| (truth(k) + jitter(k)).round() as usize).collect();
            let smoothed = smooth_line_jitter(&noisy, period);

            assert_eq!(smoothed.len(), n);
            assert!(
                smoothed.windows(2).all(|w| w[1] > w[0]),
                "n={n}: boundaries must stay increasing"
            );
            let (before, after) = (rms(&noisy), rms(&smoothed));
            // Most of the per-line jitter is averaged out (measured ~2.6 -> <0.5)
            assert!(after < before * 0.5, "n={n}: jitter not reduced: {before:.2} -> {after:.2}");
            // ...and the slant is preserved (a flattened trend would blow this
            // up to tens of samples over the run).
            assert!(after < 1.5, "n={n}: slant not tracked, residual rms {after:.2}");
        }
    }

    #[test]
    fn interval_summary_requires_two_positions() {
        assert!(interval_summary(&[100], 48_000).is_none());
    }
}
