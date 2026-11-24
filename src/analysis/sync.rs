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
/// Split after coasting when the re-lock moves at least 1% of a line
/// (~4 samples at 48 kHz). This clears the one-sample baseline p90 measured
/// on the 48 kHz rip while keeping smoothing context for smaller corrections.
const JITTER_RELOCK_FRAC: f64 = 0.01;
/// A re-anchor needs a spike-to-dip swing above this fraction of the
/// detector threshold; a full-threshold swing also counts as a strong lock.
const SWING_ANCHOR_FACTOR: f32 = 0.6;
/// Accepted re-anchor intervals must lie within this fraction of the period.
const TRACK_INTERVAL_TOL: f64 = 0.06;
/// Lines folded together to find the sync edge at the seed. Enough to
/// average picture content away, few enough that an error in the period
/// estimate cannot smear the spike.
const SEED_FOLD_LINES: usize = 8;
/// Tone periods of the start burst in one line. The burst precedes every
/// picture on the record and runs about 20 lines.
const BURST_PERIODS_PER_LINE: f64 = 16.0;
/// A line is burst tone when its autocorrelation at half the tone period is
/// below this. Burst lines measure -0.36 to -0.67 on the 48 kHz rip, and
/// leader and picture lines stay above zero.
const BURST_TONE_CORR: f64 = -0.25;
/// Consecutive coherent, non-burst detections required to re-acquire phase
/// across an inter-image boundary. Requiring 8 lines (and waiting until the
/// coasting walk has passed all 8 with no lock at the old phase) keeps brief
/// dark-region weak-sync patches with bright mid-line picture features from
/// hijacking the phase while still catching every real inter-image boundary.
const REACQUIRE_RUN_SYNCS: usize = 8;
/// Consecutive coasted lines before checking `detected` for a phase-shifted
/// coherent run (an inter-image boundary in a multi-frame window).
const REACQUIRE_COAST_LINES: usize = REACQUIRE_RUN_SYNCS;
/// Lines to probe ahead along the tracked phase before replacing it on a
/// phase-shifted candidate run. A weak-sync dark patch within one image
/// resumes locking at the original phase once the patch ends; a real
/// inter-image boundary does not.
const REACQUIRE_OLD_PHASE_LOOKAHEAD: usize = 32;
/// Consecutive strong, non-burst locks at the original phase within the
/// lookahead window needed to confirm the old phase has not ended.
const REACQUIRE_OLD_PHASE_RESUME_LOCKS: usize = 3;
/// Lines over which to compare summed non-burst sync swing between the old
/// phase and a phase-shifted candidate once the old phase appears to resume.
const REACQUIRE_COMPARE_LINES: usize = 16;

/// Track scan-line sync positions with a predictive lock.
///
/// The global detector ([`detect_line_syncs`]) picks the tallest peaks
/// anywhere, which mistimes or drops lines whenever image content rivals
/// the sync spike (dark photographs, dropouts). Reference decoders instead
/// predict each next sync at `previous + period` and search only a narrow
/// window around the prediction, coasting on the prediction when no
/// credible peak appears. This does that: it seeds from the global
/// detector's first coherent position outside a start burst, then walks the
/// buffer line by line in both directions, slowly adapting the period
/// estimate (slant) from accepted peaks on the way forward. The jitter
/// smoother treats a large re-lock offset as a run boundary.
///
/// The seed must not sit on the start burst: the tone repeats every 1/16 of
/// a line, so its peaks also recur one line apart, close enough to pass for
/// a line cadence. A track seeded there follows the tone at the wrong phase
/// and period, and shears the first picture lines until it drifts onto the
/// real sync. The sync spike itself is present all through the burst, so a
/// track that reaches the burst already in phase stays locked.
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

    // Seed near the first detected sync that starts a coherent pair outside a
    // start burst (failing that, the first coherent pair anywhere), then
    // find the sync edge there properly. A detection only says roughly where
    // a line starts: it can sit a tenth of a line late where the picture
    // goes dark right after the sync, or a few percent off around a burst,
    // and a track seeded further off than its search window never locks.
    let coherent = |i: usize| ((detected[i + 1] - detected[i]) as f64 - period).abs() / period < 0.1;
    let outside_burst = |i: usize| !is_burst_tone(samples, detected[i], period);
    let pairs = 0..detected.len() - 1;
    let rough_seed = pairs
        .clone()
        .find(|&i| coherent(i) && outside_burst(i) && outside_burst(i + 1))
        .or_else(|| pairs.clone().find(|&i| coherent(i)))
        .map_or(detected[0], |i| detected[i]);
    let seed = sync_edge_near(samples, rough_seed, period, edge_search);

    // The sync-like event nearest `predict`, if there is one: its line start,
    // whether the spike-to-dip swing clears the full threshold, and the swing.
    let lock_near_swing = |predict: f64, from: f64, period: f64| -> Option<(usize, bool, f32)> {
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
        let peak = (lo..=hi).max_by(|&a, &b| {
            samples[a]
                .partial_cmp(&samples[b])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.abs_diff(center).cmp(&a.abs_diff(center)))
        })?;

        // Falling edge after the peak marks the true line start.
        let min_idx = falling_edge_min(samples, peak, edge_search);
        // Re-anchor only on a sync-like event: a genuine spike-to-dip swing
        // (attenuated syncs in dark lines still swing harder than content
        // noise) at a plausible interval. Anchoring on anything weaker
        // randomizes line starts in dark regions; coasting there is what
        // keeps the image coherent.
        let swing = samples[peak] - samples[min_idx];
        let interval = (min_idx as f64 - from).abs();
        let sync_like = swing > threshold * SWING_ANCHOR_FACTOR && (interval - period).abs() / period < TRACK_INTERVAL_TOL;
        sync_like.then_some((min_idx, swing > threshold, swing))
    };
    let lock_near = |predict: f64, from: f64, period: f64| -> Option<(usize, bool)> {
        lock_near_swing(predict, from, period).map(|(min_idx, strong, _)| (min_idx, strong))
    };

    // Whether `candidate` lies past a full line from `anchor` and outside the
    // narrow tracking window modulo `period` — i.e. a genuine phase jump such
    // as an inter-image boundary, not a dark-region dropout within one frame
    // (where syncs resume at the same phase).
    let is_phase_jump = |anchor: f64, candidate: usize, period: f64| -> bool {
        let dist = (candidate as f64 - anchor).abs();
        if dist < period * 1.1 {
            return false;
        }
        let rem = dist % period;
        let phase_err = rem.min(period - rem);
        phase_err > period * TRACK_SEARCH_FRAC
    };
    let is_coherent_run = |start_idx: usize, period: f64| -> bool {
        if start_idx + REACQUIRE_RUN_SYNCS > detected.len() {
            return false;
        }
        (start_idx..start_idx + REACQUIRE_RUN_SYNCS - 1)
            .all(|k| ((detected[k + 1] - detected[k]) as f64 - period).abs() / period < 0.1)
            && (start_idx..start_idx + REACQUIRE_RUN_SYNCS).all(|k| !is_burst_tone(samples, detected[k], period))
    };

    // Sum the non-burst locked spike-to-dip swings along one phase track over
    // `n_lines` starting at `start_pos`.
    let phase_swing_score = |start_pos: f64, period: f64, n_lines: usize| -> f64 {
        let mut probe = start_pos - period;
        let mut total = 0.0f64;
        for _ in 0..n_lines {
            let p = probe + period;
            if p + period > samples.len() as f64 {
                break;
            }
            if let Some((min_idx, _, swing)) = lock_near_swing(p, probe, period) {
                if !is_burst_tone(samples, min_idx, period) {
                    total += swing.max(0.0) as f64;
                }
                probe = min_idx as f64;
            } else {
                probe = p;
            }
        }
        total
    };

    // Check whether the currently tracked phase resumes ahead AND beats the
    // phase-shifted candidate over the same post-resume window.
    // - Inside one image with a weak-sync dark patch + bright mid-line bar,
    //   the real syncs resume at the old phase with strictly larger swing than
    //   the picture bar, so re-acquisition is suppressed.
    // - Across a real inter-image boundary, even if the second image happens
    //   to put a bright vertical picture feature at the first image's old
    //   phase, the second image's true syncs at `candidate` are at least as
    //   strong over that window, so re-acquisition still fires.
    let old_phase_beats_candidate = |from_pos: f64, candidate: usize, period: f64| -> Option<f64> {
        let mut probe = from_pos;
        let mut streak = 0usize;
        let mut streak_start = 0.0f64;
        let mut resume_end = None;
        for _ in 0..REACQUIRE_OLD_PHASE_LOOKAHEAD {
            let p = probe + period;
            if p + period > samples.len() as f64 {
                break;
            }
            if let Some((min_idx, strong)) = lock_near(p, probe, period) {
                probe = min_idx as f64;
                if strong && !is_burst_tone(samples, min_idx, period) {
                    if streak == 0 {
                        streak_start = probe;
                    }
                    streak += 1;
                    if streak >= REACQUIRE_OLD_PHASE_RESUME_LOCKS {
                        resume_end = Some(probe);
                        break;
                    }
                } else {
                    streak = 0;
                }
            } else {
                probe = p;
                streak = 0;
            }
        }
        let resume_end = resume_end?;

        // Advance the candidate phase to the candidate line that contains
        // `streak_start` (`new_probe <= streak_start < new_probe + period`) so
        // both scores cover the same candidate-line cycles.
        let mut new_probe = candidate as f64;
        let max_steps = REACQUIRE_OLD_PHASE_LOOKAHEAD + REACQUIRE_RUN_SYNCS + 4;
        for _ in 0..max_steps {
            if new_probe + period > streak_start {
                break;
            }
            let p = new_probe + period;
            if p + period > samples.len() as f64 {
                break;
            }
            new_probe = lock_near(p, new_probe, period).map_or(p, |(m, _)| m as f64);
        }

        let max_start = streak_start.max(new_probe);
        let avail = (((samples.len() as f64 - max_start) / period).floor().max(0.0) as usize).min(REACQUIRE_COMPARE_LINES);
        if avail == 0 {
            return None;
        }
        let old_score = phase_swing_score(streak_start, period, avail);
        let new_score = phase_swing_score(new_probe, period, avail);
        (old_score > new_score).then_some(resume_end)
    };

    let mut locked = 0usize;

    // Lines before the seed, walked backward at the seed's period. Without
    // them a range that opens on a burst would lose the burst lines.
    let mut positions = Vec::new();
    let mut pos = seed as f64;
    // Backward lines up to the earliest lock. Anything coasted past it rests
    // on no sync at all (silence or noise ahead of the first line), and
    // keeping it would count against the lock fraction below.
    let mut backed_by_lock = 0;
    // Backward lines that re-locked off the prediction after a coast, by
    // their index in walk order.
    let mut backward_relocks = Vec::new();
    let mut was_coasting = false;
    loop {
        let predict = pos - period;
        // Stop where the search window would run off the buffer: a spike cut
        // by the buffer start cannot be told from a missing one.
        if predict < search as f64 {
            break;
        }
        let lock = lock_near(predict, pos, period).filter(|&(min_idx, _)| (min_idx as f64) < pos);
        let next = match lock {
            Some((min_idx, strong)) => {
                if was_coasting && (min_idx as f64 - predict).abs() >= period * JITTER_RELOCK_FRAC {
                    backward_relocks.push(positions.len());
                }
                locked += usize::from(strong);
                min_idx as f64
            }
            None => predict,
        };
        positions.push(next.round() as usize);
        if lock.is_some() {
            backed_by_lock = positions.len();
        }
        pos = next;
        was_coasting = lock.is_none();
    }
    positions.truncate(backed_by_lock);
    positions.reverse();
    // In time order the step lies just after a backward re-lock: the lines
    // coasted over on the way to it were predicted from the later side, so
    // the later run starts at the first of them.
    let mut run_starts: Vec<usize> = backward_relocks
        .iter()
        .rev()
        .map(|&walked| positions.len() - walked)
        .collect();
    positions.push(seed);

    let mut pos = seed as f64;
    let mut last_locked_pos = seed as f64;
    let mut last_locked_idx = positions.len() - 1;
    let mut coast_len = 0usize;
    let mut was_coasting = false;
    let mut old_phase_resumes_before = 0.0f64;
    let mut detect_cursor = detected.partition_point(|&d| (d as f64) <= seed as f64 + period * 1.1);
    loop {
        let predict = pos + period;
        if predict + period > samples.len() as f64 {
            break;
        }
        let lock = lock_near(predict, pos, period);
        match lock {
            Some((min_idx, strong)) => {
                if was_coasting && (min_idx as f64 - predict).abs() >= period * JITTER_RELOCK_FRAC {
                    run_starts.push(positions.len());
                }
                period = period * (1.0 - PERIOD_EMA_ALPHA) + (min_idx as f64 - pos) * PERIOD_EMA_ALPHA;
                locked += usize::from(strong);
                positions.push(min_idx);
                pos = min_idx as f64;
                last_locked_pos = pos;
                last_locked_idx = positions.len() - 1;
                if was_coasting {
                    // Anchor/period changed after a coast: reset the scan
                    // cursor so any future coast evaluates candidates against
                    // the updated anchor.
                    detect_cursor = detected.partition_point(|&d| (d as f64) <= last_locked_pos + period * 1.1);
                }
                coast_len = 0;
                was_coasting = false;
                old_phase_resumes_before = 0.0;
            }
            None => {
                // No credible sync (dark line, dropout, boundary junk): coast.
                coast_len += 1;
                positions.push(predict.round() as usize);
                pos = predict;
                was_coasting = true;

                // Across an inter-image boundary, the next image's leader syncs
                // arrive at a new phase outside our ±6% tracking window. Wait
                // until the coasting walk has passed an entire coherent
                // non-burst run at a different phase with zero locks at the old
                // phase, and confirm the old phase does not resume ahead, then
                // drop only the shifted coasted lines and re-anchor.
                if coast_len >= REACQUIRE_COAST_LINES && pos >= old_phase_resumes_before {
                    let mut reacquired = None;
                    while detect_cursor + REACQUIRE_RUN_SYNCS <= detected.len()
                        && (detected[detect_cursor + REACQUIRE_RUN_SYNCS - 1] as f64) <= predict
                    {
                        let j = detect_cursor;
                        detect_cursor += 1;
                        if (detected[j] as f64) <= last_locked_pos + period * 1.1 {
                            continue;
                        }
                        if is_coherent_run(j, period) {
                            let candidate = sync_edge_near(samples, detected[j], period, edge_search);
                            if (candidate as f64) > last_locked_pos + period * 1.1
                                && is_phase_jump(last_locked_pos, candidate, period)
                            {
                                if let Some(resume_pos) = old_phase_beats_candidate(pos, candidate, period) {
                                    old_phase_resumes_before = resume_pos;
                                } else {
                                    reacquired = Some(candidate);
                                }
                                break;
                            }
                        }
                    }
                    if let Some(new_seed) = reacquired {
                        // Walk backward from `new_seed` to pick up any opening
                        // lines of the new image that `detected` skipped.
                        let mut lead = Vec::new();
                        let mut back = new_seed as f64;
                        loop {
                            let bp = back - period;
                            if bp < search as f64 || bp <= last_locked_pos + period * 0.7 {
                                break;
                            }
                            let Some((min_idx, strong)) = lock_near(bp, back, period)
                                .filter(|&(m, _)| (m as f64) < back && (m as f64) > last_locked_pos + period * 0.7)
                            else {
                                break;
                            };
                            locked += usize::from(strong);
                            lead.push(min_idx);
                            back = min_idx as f64;
                        }
                        lead.reverse();
                        // Only the coasted lines that run into the new image
                        // are at the wrong phase. Those that end before its
                        // first line are the previous image's own closing
                        // lines (weak syncs, a dropout) and the gap: keep
                        // them, or the previous image loses those rows.
                        let first_new = lead.first().copied().unwrap_or(new_seed) as f64;
                        let kept = positions[last_locked_idx + 1..]
                            .iter()
                            .take_while(|&&p| p as f64 + period * 0.7 <= first_new)
                            .count();
                        positions.truncate(last_locked_idx + 1 + kept);
                        run_starts.retain(|&idx| idx <= last_locked_idx);
                        // The kept lines are extrapolated, not measured, so
                        // they smooth as a run of their own and leave the
                        // locked lines before them as they were.
                        if kept > 0 {
                            run_starts.push(last_locked_idx + 1);
                        }
                        run_starts.push(positions.len());
                        positions.extend(lead);
                        positions.push(new_seed);
                        locked += 1;
                        pos = new_seed as f64;
                        last_locked_pos = pos;
                        last_locked_idx = positions.len() - 1;
                        coast_len = 0;
                        was_coasting = false;
                        detect_cursor = detected.partition_point(|&d| (d as f64) <= last_locked_pos + period * 1.1);
                    }
                }
            }
        }
    }

    // A track that mostly coasted never found real line structure (music,
    // noise): hand back the raw detections so the caller's cadence check
    // can reject sync lock and fall through to fixed-period slicing.
    if locked < positions.len() / 4 {
        return detected;
    }
    smooth_line_jitter(&positions, &run_starts)
}

/// The line start nearest `rough`, in the tracker's sense: the bottom of the
/// sync spike's falling edge.
///
/// The sync spike is the one feature that recurs at the same place in every
/// line, so the spike-to-dip swing of each sample is summed over a few
/// lines folded at the period and the phase with the largest sum is the
/// spike. Picture edges swing less, and anything that is not line-locked
/// (picture detail, burst tone) lands at a different phase in every line.
/// `rough` is returned unchanged when there is too little signal to fold.
fn sync_edge_near(samples: &[f32], rough: usize, period: f64, edge_search: usize) -> usize {
    let line = period.round() as usize;
    let base = rough.saturating_sub(line / 2);
    let lines = ((samples.len().saturating_sub(base + edge_search)) / line.max(1)).min(SEED_FOLD_LINES);
    if line == 0 || lines < 2 {
        return rough;
    }
    let swing_at = |peak: usize| samples[peak] - samples[falling_edge_min(samples, peak, edge_search)];
    let folded = |phase: usize| -> f32 {
        (0..lines)
            .map(|k| swing_at(base + phase + (k as f64 * period).round() as usize))
            .sum()
    };
    let spike = (0..line)
        .map(|phase| (phase, folded(phase)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    match spike {
        Some((phase, _)) => falling_edge_min(samples, base + phase, edge_search),
        None => rough,
    }
}

/// Whether the line starting at `start` is start-burst tone rather than
/// leader or picture: a tone is strongly anti-correlated with itself half a
/// period later, wherever the line happens to start.
fn is_burst_tone(samples: &[f32], start: usize, period: f64) -> bool {
    let end = (start + period.round() as usize).min(samples.len());
    let line = &samples[start.min(end)..end];
    let half_period = period / (2.0 * BURST_PERIODS_PER_LINE);
    let lags = [half_period.floor() as usize, half_period.ceil() as usize];
    if lags[0] == 0 || line.len() <= lags[1] * 2 {
        return false;
    }
    let mean = line.iter().map(|&v| f64::from(v)).sum::<f64>() / line.len() as f64;
    let centered = |i: usize| f64::from(line[i]) - mean;
    let energy: f64 = (0..line.len()).map(|i| centered(i) * centered(i)).sum();
    if energy <= 0.0 {
        return false;
    }
    lags.into_iter().any(|lag| {
        let lagged: f64 = (lag..line.len()).map(|i| centered(i) * centered(i - lag)).sum();
        lagged / energy < BURST_TONE_CORR
    })
}

/// Low-pass the per-line detection jitter out of tracked sync positions while
/// preserving the slow cadence (period, slant, tape wow) and large re-lock steps.
///
/// The falling-edge argmin jitters ±several samples per line; the tracker's
/// tight window keeps that from drifting the lock, but it still snaps each
/// accepted line start onto the noisy argmin. Used directly as line
/// boundaries those positions render the noise as feathered edges and
/// vertical streaks. Each index in `run_starts` marks a substantial re-anchor
/// after coasting. Detrend and average the residual within each run so the
/// coast and the recovered cadence do not pull each other's line starts.
fn smooth_line_jitter(positions: &[usize], run_starts: &[usize]) -> Vec<usize> {
    let mut out = positions.to_vec();
    let mut start = 0;
    for end in run_starts.iter().copied().chain(std::iter::once(positions.len())) {
        smooth_run(&positions[start..end], &mut out[start..end], start > 0, end < positions.len());
        start = end;
    }
    // Downstream slicing requires strictly increasing line boundaries,
    // including where independently smoothed runs meet.
    for i in 1..out.len() {
        out[i] = out[i].max(out[i - 1] + 1);
    }
    out
}

/// Detrend-smooth-retrend one run of positions into `out`. Runs of
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
/// is still smoothed as much as its length allows. At the outer edges of the
/// image, reflect out-of-range indices to keep the window width. At a re-lock
/// boundary, fit a local line inside the run to preserve the residual slope.
fn smooth_run(run: &[usize], out: &mut [usize], starts_at_relock: bool, ends_at_relock: bool) {
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
    for (k, slot) in out.iter_mut().enumerate() {
        let near_start = starts_at_relock && k < half;
        let near_end = ends_at_relock && k + half >= n;
        let rs = if near_start || near_end {
            // Shift the window inside the run. Reflecting a residual with a
            // local slope would fold it back on itself and bias its average.
            let width = (2 * JITTER_SMOOTH_HALF + 1).min(n);
            let start = if near_start { 0 } else { n - width };
            fit_residual_at(&resid[start..start + width], k - start)
        } else {
            let mut sum = 0.0;
            for j in 0..=2 * half {
                let idx = k as isize + j as isize - half as isize;
                sum += resid[reflect(idx)];
            }
            sum / window
        };
        *slot = (trend(k) + rs).round().max(0.0) as usize;
    }
}

/// Evaluate a least-squares line at an index within its sample window.
fn fit_residual_at(residuals: &[f64], index: usize) -> f64 {
    let center = (residuals.len() - 1) as f64 / 2.0;
    let (mut sum, mut weighted, mut squares) = (0.0, 0.0, 0.0);
    for (k, residual) in residuals.iter().enumerate() {
        let t = k as f64 - center;
        sum += residual;
        weighted += t * residual;
        squares += t * t;
    }
    sum / residuals.len() as f64 + weighted / squares * (index as f64 - center)
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
    fn tracker_ignores_a_silent_lead_in() {
        // Four parts silence, then one part weak-sync lines. The backward
        // walk finds no sync in the silence: lines extrapolated there would
        // outnumber the locks and send the whole track back to the raw
        // detections, which mistime this fixture.
        let period = 400usize;
        let n_lines = 60;
        let lead_in = 4 * n_lines * period;
        let mut samples = vec![0.0f32; lead_in];
        samples.extend(weak_sync_lines(n_lines, period));

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        assert!(
            (tracked.len() as i64 - n_lines as i64).abs() <= 3,
            "expected ~{n_lines} lines, got {}",
            tracked.len()
        );
        for &p in &tracked {
            assert!(p >= lead_in, "line start {p} is inside the silence");
            let offset = (p - lead_in) % period;
            assert!((4..=11).contains(&offset), "tracked start at offset {offset} is off-cadence");
        }
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
    fn tracker_preserves_offset_after_relocking() {
        let period = 400;
        let lines = 160;
        let relock = 80;
        let coast_start = relock - 5;
        let truth = |line: usize| 30 + line * period + if line >= relock { 18 } else { 0 };
        let mut samples = vec![0.2; lines * period];
        for line in (0..coast_start).chain(relock..lines) {
            let jitter = [0, -3, 3, -4, 4][line % 5];
            let dip = truth(line).checked_add_signed(jitter).unwrap();
            samples[dip - 4] = 1.0;
            samples[dip] = -0.8;
        }

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        // The fixture omits five spikes before an 18-sample offset. Around
        // the recovery the boundaries should stay within the ordinary jitter
        // on their own side, while the jitter within both runs is reduced.
        for line in [coast_start - 40..coast_start, relock..relock + 40].into_iter().flatten() {
            assert!(
                tracked[line].abs_diff(truth(line)) <= 2,
                "line {line}: expected {}, got {}",
                truth(line),
                tracked[line]
            );
        }
    }

    #[test]
    fn tracker_preserves_cadence_at_relock_with_slope() {
        let lines = 200;
        let relock = 80;
        let coast_start = relock - 3;
        let truth = |line: usize| 30 + 400 * line + 5 * line.saturating_sub(60).min(41);
        let mut samples = vec![0.2; truth(lines - 1) + 400];
        for line in (0..coast_start).chain(relock..lines) {
            let dip = truth(line);
            samples[dip - 4] = 1.0;
            samples[dip] = -0.8;
        }

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        // The cadence changes before the dropout and continues through the
        // re-lock. Smoothing should preserve the accurate locks on both sides.
        for line in (relock..relock + 10).chain(coast_start - 10..coast_start) {
            assert!(
                tracked[line].abs_diff(truth(line)) <= 2,
                "line {line}: expected {}, got {}",
                truth(line),
                tracked[line]
            );
        }
    }

    /// A range that opens partway through a line of start-burst tone, as
    /// eight frames of the 48 kHz rip do. The tone is not line-locked: 16
    /// periods take 409.6 samples against a 400-sample line. Each line
    /// starts with a sync spike (16 samples) and its dip (8 samples).
    fn lines_opening_on_a_burst(burst_lines: usize, n_lines: usize) -> (Vec<f32>, usize) {
        const PERIOD: usize = 400;
        // One line of tone before the first sync. Its first trough falls
        // about a line ahead of that sync, so the pair looks coherent.
        let lead_in = 400;
        // The tone fades a little over the lead-in, so its first peak is its
        // tallest and the detector keeps that one, as it does on the record.
        let tone = |t: usize| {
            let level = 0.75 - 0.05 * (t.min(lead_in) as f32 / lead_in as f32);
            level * (std::f32::consts::TAU * t as f32 / 25.6).sin()
        };
        let mut out: Vec<f32> = (0..lead_in).map(tone).collect();
        for line in 0..n_lines {
            for i in 0..PERIOD {
                out.push(match i {
                    0..=15 => 1.0,
                    16..=23 => -0.8,
                    _ if line < burst_lines => tone(out.len()),
                    _ => 0.2 + 0.2 * ((line * 7 + i) % 13) as f32 / 13.0,
                });
            }
        }
        (out, lead_in)
    }

    #[test]
    fn tracker_keeps_line_phase_when_the_range_opens_on_a_burst() {
        let (samples, lead_in) = lines_opening_on_a_burst(18, 60);
        // The first detection is a tone trough about one line before the
        // first sync: a coherent-looking pair, yet further off the line
        // phase than the tracker's search window reaches. Seeding there
        // follows the tone and shears every line until the track drifts
        // onto the sync.
        let detected = detect_line_syncs(&samples, 48_000, &SyncParams::default());
        let first_gap = (detected[1] - detected[0]) as f64;
        assert!(
            (first_gap - 400.0).abs() / 400.0 < 0.1,
            "setup: first pair {first_gap} is not coherent"
        );
        assert!(
            (first_gap - 400.0).abs() > 400.0 * TRACK_SEARCH_FRAC,
            "setup: first pair {first_gap} is in phase"
        );

        let positions = track_line_syncs(&samples, 48_000, &SyncParams::default());
        for &p in positions.iter().filter(|&&p| p >= lead_in) {
            let offset = (p - lead_in) % 400;
            assert!(
                (16..24).contains(&offset),
                "line start {p} is at offset {offset}, not in the sync dip"
            );
        }
        // The burst lines before the seed are kept, not dropped.
        assert!(
            positions.iter().any(|p| (lead_in..lead_in + 400).contains(p)),
            "first whole line is missing"
        );
        assert!(positions.len() >= 58, "only {} lines tracked", positions.len());
    }

    #[test]
    fn tracker_preserves_offset_at_a_relock_before_the_seed() {
        // A long burst with a five-line dropout, after which every sync
        // sits 18 samples later. The seed lands after the burst, so the
        // step is met on the backward walk.
        let (period, lines, burst_lines) = (400usize, 160usize, 100usize);
        let (dropout, relock) = (75usize, 80usize);
        let truth = |line: usize| 430 + line * period + if line >= relock { 18 } else { 0 };
        let tone = |t: usize| 0.7 * (std::f32::consts::TAU * t as f32 / 25.6).sin();
        let burst_end = truth(burst_lines) - 16;
        let mut samples: Vec<f32> = (0..truth(lines)).map(|t| if t < burst_end { tone(t) } else { 0.2 }).collect();
        samples[truth(dropout) - 16..truth(relock) - 16].fill(0.2);
        for line in (0..dropout).chain(relock..lines) {
            let dip = truth(line);
            samples[dip - 16..dip].fill(1.0);
            samples[dip..dip + 8].fill(-0.8);
        }

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        // Smoothing across the step would pull the lines on both sides of
        // it toward each other by several samples.
        for line in [dropout - 30..dropout, relock..relock + 30].into_iter().flatten() {
            let nearest = tracked.iter().map(|p| p.abs_diff(truth(line))).min().unwrap();
            assert!(
                nearest <= 2,
                "line {line}: nearest tracked start is {nearest} samples from {}",
                truth(line)
            );
        }
    }

    #[test]
    fn tracker_starts_lines_at_the_sync_edge_not_a_dark_picture_edge() {
        // After the sync dip the line sits at a shallow porch level, then
        // drops to picture black. The detector's wide falling-edge search
        // reports the black edge, 36 samples late; the seed must not.
        let mut samples = Vec::new();
        for line in 0..40usize {
            for i in 0..400usize {
                samples.push(match i {
                    0..=15 => 1.0,
                    16..=51 => -0.1,
                    52..=119 => -0.6,
                    _ => -0.4 + 0.3 * ((line * 5 + i) % 11) as f32 / 11.0,
                });
            }
        }
        let detected = detect_line_syncs(&samples, 48_000, &SyncParams::default());
        assert_eq!(detected[0] % 400, 52, "the detector is expected to report the black edge");

        let positions = track_line_syncs(&samples, 48_000, &SyncParams::default());
        for &p in &positions {
            assert!((16..24).contains(&(p % 400)), "line start {p} is at offset {}", p % 400);
        }
    }

    #[test]
    fn burst_tone_is_told_from_picture_and_leader() {
        let (samples, lead_in) = lines_opening_on_a_burst(18, 30);
        // Wherever the window starts inside the burst, it reads as tone.
        for start in [0, 7, 13, lead_in + 24, lead_in + 400 * 5 + 100] {
            assert!(is_burst_tone(&samples, start, 400.0), "burst line at {start}");
        }
        for line in 19..28 {
            assert!(!is_burst_tone(&samples, lead_in + 400 * line, 400.0), "picture line {line}");
        }
        assert!(!is_burst_tone(&[0.25f32; 400], 0, 400.0), "flat leader line");
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

        let smoothed = smooth_line_jitter(&noisy, &[]);

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
    fn smoothing_output_is_strictly_increasing_at_small_period() {
        // Degenerate cadence: ~1 sample/line with ±4 jitter. The trend's
        // per-step rise is tiny, so without the monotonic clamp the residual
        // moving average can produce equal/decreasing boundaries — which would
        // underflow the interval subtraction in segment_lines.
        let n = 60usize;
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let run: Vec<usize> = (0..n).map(|k| (1000.0 + k as f64 + jitter(k)).round() as usize).collect();
        let out = smooth_line_jitter(&run, &[]);
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
    fn smoothing_splits_on_relock_and_smooths_each_run() {
        // Two noisy runs separated by a marked, sub-line re-lock step. The
        // smoother must preserve the offset and de-jitter each run.
        let period = 399.0;
        let n = 80usize;
        let jitter = |k: usize| ((k.wrapping_mul(2_654_435_761) >> 8) % 9) as f64 - 4.0;
        let run_truth = |base: f64, k: usize| base + period * k as f64;

        let mut noisy: Vec<usize> = (0..n).map(|k| (run_truth(1000.0, k) + jitter(k)).round() as usize).collect();
        let offset = 18.0;
        let relock_base = run_truth(1000.0, n) + offset;
        noisy.extend((0..n).map(|k| (run_truth(relock_base, k) + jitter(k)).round() as usize));

        let smoothed = smooth_line_jitter(&noisy, &[n]);

        assert_eq!(smoothed.len(), noisy.len());
        assert!(
            smoothed.windows(2).all(|w| w[1] > w[0]),
            "boundaries across both runs must stay increasing"
        );
        let break_step = (smoothed[n] - smoothed[n - 1]) as f64 - period;
        assert!((break_step - offset).abs() <= 3.0, "smoothed re-lock step {break_step}");
        // Each run is independently de-jittered (residual rms well under the
        // ±4-sample raw jitter).
        let run_rms = |off: usize, base: f64| -> f64 {
            (0..n)
                .map(|k| (smoothed[off + k] as f64 - run_truth(base, k)).powi(2))
                .sum::<f64>()
                / n as f64
        };
        assert!(run_rms(0, 1000.0).sqrt() < 2.0, "first run not smoothed");
        assert!(run_rms(n, relock_base).sqrt() < 2.0, "second run not smoothed");
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
            let smoothed = smooth_line_jitter(&noisy, &[]);

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
        let after = rms(&smooth_line_jitter(&noisy, &[]));
        assert!(after < rms(&noisy) * 0.6, "n=10 weakly smoothed: {after:.2}");
    }

    #[test]
    fn smoothing_handles_short_runs_between_relocks() {
        let start = 21;
        let period = 400;
        let truth = |k: usize| 1000 + k * period + 18;
        let jitter = [3, -4, 2, -2, 4, -3];
        for n in [1, 2, 3, 4, 6, 10, 20] {
            let end = start + n;
            let mut positions: Vec<usize> = (0..start).map(|k| 1000 + k * period).collect();
            positions.extend((start..end).map(|k| truth(k).checked_add_signed(jitter[(k - start) % jitter.len()]).unwrap()));
            positions.extend((end..end + 21).map(|k| 1000 + k * period + 36));

            let smoothed = smooth_line_jitter(&positions, &[start, end]);
            assert!(smoothed.windows(2).all(|w| w[0] < w[1]));
            if n < 3 {
                assert_eq!(smoothed[start..end], positions[start..end]);
            } else {
                let squared_error =
                    |v: &[usize]| -> usize { v.iter().enumerate().map(|(k, &p)| p.abs_diff(truth(start + k)).pow(2)).sum() };
                assert!(
                    squared_error(&smoothed[start..end]) < squared_error(&positions[start..end]),
                    "n={n}: jitter was not reduced"
                );
            }
        }
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
            let smoothed = smooth_line_jitter(&noisy, &[]);

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
    fn tracker_reacquires_phase_across_an_inter_image_boundary() {
        // Two 60-line images separated by a 2.4-line gap (160-sample phase
        // shift, well outside the ±6% tracking window). Without phase
        // re-acquisition the tracker coasts through the second image at the
        // first image's phase and shifts its leader sideways.
        let period = 400usize;
        let first_lines = 60usize;
        let second_lines = 60usize;
        let gap = period * 2 + 160;
        let mut samples = synthetic_lines(first_lines, period);
        samples.extend(std::iter::repeat_n(0.0f32, gap));
        let second_start = samples.len();
        samples.extend(synthetic_lines(second_lines, period));

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        let first_half: Vec<usize> = tracked.iter().copied().filter(|&p| p < first_lines * period).collect();
        let second_half: Vec<usize> = tracked.iter().copied().filter(|&p| p >= second_start).collect();

        assert!(first_half.len() >= first_lines - 3, "first image: {} lines", first_half.len());
        assert!(
            second_half.len() >= second_lines - 3,
            "second image: {} lines",
            second_half.len()
        );
        for &p in &first_half {
            assert!((4..=10).contains(&(p % period)), "first image start {p} off-phase");
        }
        for &p in &second_half {
            let offset = (p - second_start) % period;
            assert!(
                (4..=10).contains(&offset),
                "second image start {p} shifted sideways (offset {offset})"
            );
        }
    }

    #[test]
    fn tracker_reacquires_phase_across_multiple_image_boundaries() {
        // Three images separated by two different non-integer gaps (170-sample
        // and 210-sample phase shifts). Every image's lines must lock to its
        // own sync dip without carrying the earlier phase forward.
        let period = 400usize;
        let n_lines = 50usize;
        let mut samples = synthetic_lines(n_lines, period);
        samples.extend(std::iter::repeat_n(0.0f32, period * 2 + 170));
        let second_start = samples.len();
        samples.extend(synthetic_lines(n_lines, period));
        samples.extend(std::iter::repeat_n(0.0f32, period * 3 + 210));
        let third_start = samples.len();
        samples.extend(synthetic_lines(n_lines, period));

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        for (label, base) in [("first", 0), ("second", second_start), ("third", third_start)] {
            let end = base + n_lines * period;
            let group: Vec<usize> = tracked.iter().copied().filter(|&p| p >= base && p < end).collect();
            assert!(group.len() >= n_lines - 3, "{label} image: {} lines", group.len());
            for &p in &group {
                let offset = (p - base) % period;
                assert!(
                    (4..=10).contains(&offset),
                    "{label} image start {p} off-phase (offset {offset})"
                );
            }
        }
    }

    #[test]
    fn reacquiring_keeps_the_lines_coasted_before_the_new_run() {
        // A five-line dropout, after which every sync sits 60 samples (15%
        // of a line) later: outside the tracking window, so the tracker
        // re-acquires. The lines it coasted across the dropout end before
        // the new run starts. They are lines of the first image and must
        // stay in the track, or that image loses five rows.
        let period = 400usize;
        let (lines, dropout, resume) = (120usize, 60usize, 65usize);
        let truth = |line: usize| 30 + line * period + if line >= resume { 60 } else { 0 };
        let mut samples = vec![0.2f32; truth(lines)];
        for line in (0..dropout).chain(resume..lines) {
            let dip = truth(line);
            samples[dip - 4] = 1.0;
            samples[dip] = -0.8;
        }

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        let nearest = |target: usize| tracked.iter().map(|p| p.abs_diff(target)).min().unwrap();
        // The dropout lines stay, at the cadence of the syncs before them.
        for line in dropout..resume {
            let coasted = 30 + line * period;
            assert!(nearest(coasted) <= 2, "dropout line {line} is missing from the track");
        }
        // The line coasted into the new run at the old phase is dropped.
        let stale = 30 + resume * period;
        assert!(nearest(stale) > 20, "a line at the old phase survives inside the new run");
        // Both sides stay on their own syncs.
        for line in (dropout - 20..dropout).chain(resume..resume + 20) {
            assert!(nearest(truth(line)) <= 2, "line {line} is off its sync");
        }
    }

    #[test]
    fn tracker_keeps_phase_past_weak_syncs_with_a_bright_mid_line_bar() {
        // A dark region with a 12-line weak-sync fade (longer than
        // REACQUIRE_RUN_SYNCS) and a bright vertical picture feature at a fixed
        // mid-line offset. The weak syncs have swing 0.20 (< 0.27 anchor
        // threshold), forcing the tracker to coast through all 12 lines while
        // the global detector picks up 12 coherent detections on the bright
        // feature. Because the original phase resumes after the fade, the
        // tracker must keep the original phase instead of hijacking onto the
        // picture feature.
        let period = 400usize;
        let lines = 180usize;
        let (fade_start, fade_end) = (80usize, 92usize);
        let mut samples = vec![-0.1f32; lines * period];
        for line in 0..lines {
            let base = line * period;
            if !(fade_start..fade_end).contains(&line) {
                samples[base + 26..=base + 29].fill(1.0);
                samples[base + 30..=base + 33].fill(-0.8);
            } else {
                // Weak sync: swing 0.20, strictly below the 0.27 anchor threshold.
                samples[base + 26..=base + 29].fill(0.10);
                samples[base + 30..=base + 33].fill(-0.10);
            }
            // Bright vertical feature mid-line throughout the central region.
            if (50..140).contains(&line) {
                samples[base + 216..=base + 220].fill(0.75);
                samples[base + 221..=base + 225].fill(-0.5);
            }
        }

        // Confirm the global detector actually produces a >=8-line coherent run
        // on the bright mid-line feature during the fade so the test genuinely
        // exercises the re-acquisition guard.
        let raw = detect_line_syncs(&samples, 48_000, &SyncParams::default());
        let bar_hits = raw
            .iter()
            .filter(|&&p| {
                let line = p / period;
                let off = p % period;
                (fade_start..fade_end).contains(&line) && (215..=230).contains(&off)
            })
            .count();
        assert!(
            bar_hits >= REACQUIRE_RUN_SYNCS,
            "fixture should produce >= {REACQUIRE_RUN_SYNCS} bright-bar detections during fade, got {bar_hits}"
        );

        let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
        assert!(
            (tracked.len() as i64 - lines as i64).abs() <= 3,
            "expected ~{lines} lines, got {}",
            tracked.len()
        );
        for &p in &tracked {
            let offset = p % period;
            assert!(
                (26..=35).contains(&offset),
                "line start {p} drifted onto picture feature (offset {offset})"
            );
        }
    }

    #[test]
    fn tracker_reacquires_across_boundary_despite_bright_feature_at_old_phase() {
        // Two 60-line images separated by a 2.4-line gap (160-sample phase
        // shift), with a bright vertical picture feature in the second image
        // starting at line 12 that happens to land at the first image's old
        // phase (offset 245, since 400 - 160 + 5 = 245). Even though the old
        // phase finds strong locks on that future picture feature, the second
        // image's true syncs at offset 5 carry equal or greater swing over the
        // same window, so the tracker must re-acquire onto offset 5 instead of
        // letting future picture content veto the phase change.
        let period = 400usize;
        let first_lines = 60usize;
        let second_lines = 60usize;
        let gap = period * 2 + 160;
        for &(bar_peak, bar_dip, gain_drop_line) in &[
            (0.75f32, -0.50f32, None),
            (1.0f32, -0.80f32, None),
            (1.0f32, -0.80f32, Some(28usize)),
        ] {
            let mut samples = synthetic_lines(first_lines, period);
            samples.extend(std::iter::repeat_n(0.0f32, gap));
            let second_start = samples.len();
            samples.extend(synthetic_lines(second_lines, period));
            for line in 12..second_lines {
                let base = second_start + line * period;
                samples[base + 241..=base + 244].fill(bar_peak);
                samples[base + 245..=base + 248].fill(bar_dip);
            }
            if let Some(drop_line) = gain_drop_line {
                let drop_start = second_start + drop_line * period;
                for s in &mut samples[drop_start..] {
                    *s *= 0.5;
                }
            }

            let tracked = track_line_syncs(&samples, 48_000, &SyncParams::default());
            let second_half: Vec<usize> = tracked.iter().copied().filter(|&p| p >= second_start).collect();
            assert!(
                second_half.len() >= second_lines - 3,
                "bar=({bar_peak},{bar_dip}) drop={gain_drop_line:?}: second image got {} lines",
                second_half.len()
            );
            for &p in &second_half {
                let offset = (p - second_start) % period;
                assert!(
                    (4..=10).contains(&offset),
                    "bar=({bar_peak},{bar_dip}) drop={gain_drop_line:?}: second image start {p} vetoed onto old phase (offset {offset})"
                );
            }
        }
    }

    #[test]
    fn interval_summary_requires_two_positions() {
        assert!(interval_summary(&[100], 48_000).is_none());
    }
}
