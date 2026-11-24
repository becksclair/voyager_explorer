//! Picture-row detection inside one decoded frame.
//!
//! A segmented image range holds more than the picture. The segmenter keeps
//! every sync-locked line around it, so a decoded frame is laid out as
//!
//! ```text
//! leader lines | start burst | blank lines | picture | leader lines
//! ```
//!
//! where any part but the picture may be missing. Leader and blank lines
//! carry sync but no video: they are flat down to the recording's noise
//! floor. Picture lines always carry video noise, even across a uniform
//! border. The start burst is a tone that repeats 16 times per line.
//!
//! How much leader a frame keeps varies from none to a fifth of its lines,
//! so the picture cannot be cropped by a fixed fraction; it is found here
//! per frame. Measured on the 156 frames of the 48 kHz rip: the burst is 20
//! lines, up to 11 blank lines follow it, and the picture is 519–534 lines
//! in the 153 ranges that hold all of it (the segmenter cuts three short).

use std::ops::Range;

/// Share of each line that is measured, in percent of its width. The sync
/// bands at both ends of a line are the same on leader and picture lines.
const MEASURED_COLS: (usize, usize) = (15, 85);

/// Activity is measured on this many block means per line, not on single
/// levels: averaging neighbours removes noise above the video band, which
/// the 384 kHz masters carry on leader and picture lines alike.
const ACTIVITY_BLOCKS_PER_LINE: usize = 256;

/// A line is leader when its activity is under this multiple of the frame's
/// noise floor. Leader lines measure up to 1.75× the floor and the quietest
/// picture lines 3.5× or more, on the 48 kHz rip and the 384 kHz masters.
const LEADER_ACTIVITY_FACTOR: f64 = 2.5;

/// Upper edge of the leader cluster, as a multiple of the noise floor.
const LEADER_SPREAD_FACTOR: f64 = 1.75;

/// A real noise floor leaves the band between the leader cluster and the
/// picture threshold nearly empty: only the few lines where a picture fades
/// in or out land there (at most 8 per frame on the rip). A floor taken
/// from a picture line fills it. More than one line in this many inside the
/// band means the floor is not a leader line.
const MAX_BAND_SHARE: usize = 40;

/// Tone periods of the start burst in one line.
const BURST_PERIODS_PER_LINE: usize = 16;

/// Half-period autocorrelation below which a line is certainly burst. Burst
/// lines measure about -0.75 and no picture line in the rip goes below -0.5.
const BURST_SEED_CORR: f64 = -0.6;

/// Looser test for the other lines of a burst, applied only to lines about
/// as active as the seeds. A burst line cut at the wrong place correlates
/// more weakly, down to about -0.35.
const BURST_ROW_CORR: f64 = -0.2;
const BURST_LEVEL_FRAC: f64 = 0.6;

const MIN_BURST_SEEDS: usize = 2;
const MIN_BURST_ROWS: usize = 3;

/// Lines dropped on each side of a burst: its first and last lines are
/// partly tone and partly something else.
const BURST_MARGIN_ROWS: usize = 2;

/// Leader-like lines inside a picture that do not split it.
const MAX_GAP_ROWS: usize = 2;

const MIN_QUIET_ROWS: usize = 16;

/// Most lines that can sit between a picture and the burst before it, or
/// the frame start when no burst comes before it. The rip has at most 11.
const MAX_BLANK_ROWS: usize = 16;

/// Fewest lines of a picture with a burst before it and leader after it,
/// which is a picture the frame holds whole. The rip has at least 523.
const MIN_WHOLE_PICTURE_ROWS: usize = 500;

/// Most lines a picture can have. The rip has at most 534; anything much
/// longer has leader in it.
const MAX_PICTURE_ROWS: usize = 560;

/// Unusually quiet lines (dropouts) that may be set aside when the noise
/// floor they give lets leader through as picture.
const MAX_QUIET_OUTLIERS: usize = 2;

/// Find the rows of a decoded frame that hold the picture.
///
/// `levels` is row-major, `width` values per scan line, from the raw
/// (un-restored, un-normalized) signal. Returns `None` when no
/// picture can be told apart: too few lines, lines too narrow to resolve the
/// burst, or no run of picture lines covering at least a quarter of the
/// frame.
///
/// Leader is told from picture against the frame's own noise floor (see
/// `leader_floor`). A frame that keeps no leader or blank line has no
/// floor to measure, and a floor read from one of its picture lines makes
/// the quiet parts of the picture look like leader. Two checks catch that:
/// the floor must look like a leader level, and the picture it yields must
/// fit the frame layout (see `fits_frame_layout`). When either fails,
/// every line outside a burst counts as picture.
///
/// One case cannot be told apart from inside the frame: no leader at all,
/// no burst, and a picture that is quiet along one edge only. Those quiet
/// lines are then taken for leader. No frame of the rip is like that.
pub fn find_picture_rows(levels: &[f32], width: usize) -> Option<Range<usize>> {
    let half_period = width / (2 * BURST_PERIODS_PER_LINE);
    if half_period < 2 || levels.is_empty() || !levels.len().is_multiple_of(width) {
        return None;
    }
    let cols = width * MEASURED_COLS.0 / 100..width * MEASURED_COLS.1 / 100;
    let block = (width / ACTIVITY_BLOCKS_PER_LINE).max(1);
    let rows: Vec<RowStats> = levels
        .chunks_exact(width)
        .map(|line| RowStats::measure(&line[cols.clone()], block, half_period))
        .collect();

    let burst = burst_rows(&rows);

    let mut quiet: Vec<f64> = rows
        .iter()
        .zip(&burst)
        .filter(|(_, &b)| !b)
        .map(|(r, _)| r.activity)
        .collect();
    if quiet.len() < MIN_QUIET_ROWS {
        return None;
    }
    quiet.sort_by(f64::total_cmp);

    // The longest run of lines outside any burst and above `threshold`.
    let picture_above = |threshold: f64| {
        let mut picture: Vec<bool> = rows.iter().zip(&burst).map(|(r, &b)| !b && r.activity > threshold).collect();
        bridge_gaps(&mut picture, MAX_GAP_ROWS);
        for (p, &b) in picture.iter_mut().zip(&burst) {
            *p &= !b;
        }
        runs(&picture).into_iter().max_by_key(|r| r.len())
    };
    let mut measured = None;
    for outliers in 0..=MAX_QUIET_OUTLIERS {
        match leader_floor(&quiet[outliers..]).map(|floor| picture_above(LEADER_ACTIVITY_FACTOR * floor)) {
            // Leader and nothing else.
            Some(None) => return None,
            // Too long to be a picture: the floor was read from a line far
            // quieter than the leader, and the leader came through with it.
            // Set that line aside and measure again.
            Some(Some(found)) if found.len() > MAX_PICTURE_ROWS => continue,
            Some(Some(found)) if fits_frame_layout(&found, &burst) => measured = Some(found),
            _ => {}
        }
        break;
    }
    // With no floor to trust, nothing outside a burst is leader.
    let picture = measured.or_else(|| picture_above(f64::NEG_INFINITY))?;
    (picture.len() >= rows.len() / 4).then_some(picture)
}

/// Whether a detected picture fits how a frame is laid out: few lines
/// between it and the burst before it (or the frame start), and a full
/// line count when the frame holds it whole.
fn fits_frame_layout(picture: &Range<usize>, burst: &[bool]) -> bool {
    let burst_end = burst[..picture.start].iter().rposition(|&b| b).map(|last| last + 1);
    let blank = picture.start - burst_end.unwrap_or(0);
    let whole = burst_end.is_some() && picture.end < burst.len();
    blank <= MAX_BLANK_ROWS && (!whole || picture.len() >= MIN_WHOLE_PICTURE_ROWS)
}

/// The activity of a leader line, from the sorted activities of the lines
/// outside any burst. `None` when the frame keeps no leader line.
///
/// Leader lines are the quietest in a frame and sit close together, so the
/// floor is read near the bottom of the distribution: at the 1st percentile
/// when that line still belongs with the quietest one, lower when the frame
/// keeps only a handful of leader lines. Reading it above the very bottom
/// keeps a slightly quieter line from setting it. A line far below the
/// cluster (a dropout) still sets it; the caller notices the over-long
/// picture that results and calls again without that line. The floor is
/// accepted only when the band above the leader cluster is nearly empty
/// (see [`MAX_BAND_SHARE`]).
fn leader_floor(sorted: &[f64]) -> Option<f64> {
    let quietest = sorted.iter().copied().find(|&a| a > 0.0)?;
    let floor = [(sorted.len() / 100).max(2), 2]
        .into_iter()
        .filter_map(|rank| sorted.get(rank).copied())
        .find(|a| (quietest..=LEADER_SPREAD_FACTOR * quietest).contains(a))
        .unwrap_or(quietest);
    let band = LEADER_SPREAD_FACTOR * floor..=LEADER_ACTIVITY_FACTOR * floor;
    let inside = sorted.iter().filter(|a| band.contains(a)).count();
    (inside <= sorted.len() / MAX_BAND_SHARE).then_some(floor)
}

struct RowStats {
    /// Mean absolute step between neighbouring block means, after removing
    /// the line's overall slope. Mains hum tilts alternate lines by
    /// different amounts and would otherwise count as activity.
    activity: f64,
    /// Autocorrelation at half the burst's tone period, in [-1, 1].
    half_period_corr: f64,
}

impl RowStats {
    fn measure(line: &[f32], block: usize, half_period: usize) -> Self {
        let n = line.len();
        let means: Vec<f64> = line
            .chunks_exact(block)
            .map(|b| b.iter().map(|&v| f64::from(v)).sum::<f64>() / block as f64)
            .collect();
        if n <= half_period || means.len() < 3 {
            return Self {
                activity: 0.0,
                half_period_corr: 0.0,
            };
        }
        let steps = (means.len() - 1) as f64;
        let slope = (means[means.len() - 1] - means[0]) / steps;
        let activity = means.windows(2).map(|w| (w[1] - w[0] - slope).abs()).sum::<f64>() / steps;
        let mean = line.iter().map(|&v| f64::from(v)).sum::<f64>() / n as f64;
        let centered = |i: usize| f64::from(line[i]) - mean;
        let energy: f64 = (0..n).map(|i| centered(i) * centered(i)).sum();
        let lagged: f64 = (half_period..n).map(|i| centered(i) * centered(i - half_period)).sum();
        Self {
            activity,
            half_period_corr: if energy > 0.0 { lagged / energy } else { 0.0 },
        }
    }
}

/// Mark the rows of every start burst, with a margin on each side.
fn burst_rows(rows: &[RowStats]) -> Vec<bool> {
    let mut burst = vec![false; rows.len()];
    let mut seed_levels: Vec<f64> = rows
        .iter()
        .filter(|r| r.half_period_corr < BURST_SEED_CORR)
        .map(|r| r.activity)
        .collect();
    if seed_levels.len() < MIN_BURST_SEEDS {
        return burst;
    }
    seed_levels.sort_by(f64::total_cmp);
    let level = seed_levels[seed_levels.len() / 2];

    let mut candidate: Vec<bool> = rows
        .iter()
        .map(|r| r.activity >= BURST_LEVEL_FRAC * level && r.half_period_corr < BURST_ROW_CORR)
        .collect();
    bridge_gaps(&mut candidate, 1);
    for run in runs(&candidate) {
        if run.len() >= MIN_BURST_ROWS {
            let start = run.start.saturating_sub(BURST_MARGIN_ROWS);
            let end = (run.end + BURST_MARGIN_ROWS).min(rows.len());
            burst[start..end].fill(true);
        }
    }
    burst
}

/// Maximal runs of `true`, in order.
fn runs(mask: &[bool]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &set) in mask.iter().enumerate() {
        match (set, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..mask.len());
    }
    out
}

/// Fill gaps of at most `max_gap` between two runs of `true`.
fn bridge_gaps(mask: &mut [bool], max_gap: usize) {
    let found = runs(mask);
    for pair in found.windows(2) {
        if pair[1].start - pair[0].end <= max_gap {
            mask[pair[0].end..pair[1].start].fill(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 512;
    const LEADER_NOISE: f32 = 0.0001;
    const PICTURE_NOISE: f32 = 0.0006;

    /// Deterministic noise in [-1, 1].
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.0
        }
    }

    #[derive(Clone, Copy)]
    enum Line {
        Leader,
        Burst,
        Picture,
    }

    /// Build a frame from `(kind, line count)` sections.
    fn frame(sections: &[(Line, usize)]) -> Vec<f32> {
        let mut noise = Noise(1);
        let mut levels = Vec::new();
        let mut row = 0usize;
        for &(kind, count) in sections {
            for _ in 0..count {
                for x in 0..WIDTH {
                    let t = x as f32 / WIDTH as f32;
                    levels.push(match kind {
                        Line::Leader => 0.1 + LEADER_NOISE * noise.next(),
                        // The tone is not locked to the line, so its phase
                        // drifts from one line to the next.
                        Line::Burst => {
                            let phase = std::f32::consts::TAU * (BURST_PERIODS_PER_LINE as f32 * t + 0.13 * row as f32);
                            0.1 + 0.05 * phase.sin()
                        }
                        // A uniform border at the line ends, a ramp between.
                        Line::Picture => 0.1 + 0.05 * (t - 0.5).clamp(-0.2, 0.2) + PICTURE_NOISE * noise.next(),
                    });
                }
                row += 1;
            }
        }
        levels
    }

    /// Lines in a whole picture, as on the record.
    const PICTURE: usize = 525;

    #[test]
    fn finds_the_picture_between_burst_and_trailing_leader() {
        let levels = frame(&[
            (Line::Leader, 40),
            (Line::Burst, 20),
            (Line::Leader, 10),
            (Line::Picture, PICTURE),
            (Line::Leader, 60),
        ]);
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(70..70 + PICTURE));
    }

    #[test]
    fn drops_a_burst_that_runs_straight_into_the_picture() {
        let levels = frame(&[
            (Line::Leader, 40),
            (Line::Burst, 20),
            (Line::Picture, PICTURE),
            (Line::Leader, 60),
        ]);
        // The burst margin takes the first picture lines with it.
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(60 + BURST_MARGIN_ROWS..60 + PICTURE));
    }

    #[test]
    fn frame_without_a_burst_starts_at_the_first_picture_line() {
        // The range opens a few blank lines before a picture cut short.
        let levels = frame(&[(Line::Leader, 8), (Line::Picture, 300), (Line::Leader, 60)]);
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(8..308));
    }

    #[test]
    fn picture_cut_off_by_the_frame_end_runs_to_it() {
        let levels = frame(&[(Line::Burst, 20), (Line::Leader, 10), (Line::Picture, 300)]);
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(30..330));
    }

    /// Overwrite `rows` with uniform lines that carry only picture noise.
    fn flatten(levels: &mut [f32], rows: impl Iterator<Item = usize>) {
        let mut noise = Noise(7);
        for row in rows {
            for v in &mut levels[row * WIDTH..(row + 1) * WIDTH] {
                *v = 0.1 + PICTURE_NOISE * noise.next();
            }
        }
    }

    #[test]
    fn uniform_picture_border_lines_are_kept() {
        // Border lines are flat apart from picture noise; they must not be
        // mistaken for leader.
        let mut levels = frame(&[
            (Line::Leader, 40),
            (Line::Burst, 20),
            (Line::Leader, 10),
            (Line::Picture, PICTURE),
            (Line::Leader, 60),
        ]);
        flatten(&mut levels, (70..90).chain(70 + PICTURE - 20..70 + PICTURE));
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(70..70 + PICTURE));
    }

    #[test]
    fn quiet_picture_lines_are_not_leader_when_the_frame_keeps_none() {
        // A frame cut to the picture alone: its quietest lines set the
        // "noise floor", and the busier middle would be all that is left.
        let mut levels = frame(&[(Line::Picture, PICTURE)]);
        for v in &mut levels[150 * WIDTH..400 * WIDTH] {
            *v *= 3.0;
        }
        flatten(&mut levels, (0..150).chain(400..PICTURE));
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(0..PICTURE));
    }

    #[test]
    fn a_few_blank_lines_are_enough_to_find_the_floor() {
        // Burst, three blank lines, picture to the frame end: the only
        // leader in the frame is those three lines.
        let levels = frame(&[(Line::Burst, 20), (Line::Leader, 3), (Line::Picture, PICTURE)]);
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(23..23 + PICTURE));
    }

    #[test]
    fn one_dead_quiet_leader_line_does_not_let_the_leader_through() {
        let mut levels = frame(&[
            (Line::Leader, 40),
            (Line::Burst, 20),
            (Line::Leader, 10),
            (Line::Picture, PICTURE),
            (Line::Leader, 60),
        ]);
        // A dropout: one leader line ten times quieter than the others.
        for v in &mut levels[5 * WIDTH..6 * WIDTH] {
            *v = 0.1 + (*v - 0.1) * 0.1;
        }
        assert_eq!(find_picture_rows(&levels, WIDTH), Some(70..70 + PICTURE));
    }

    #[test]
    fn frame_with_nothing_but_leader_has_no_picture() {
        let levels = frame(&[(Line::Leader, 200)]);
        assert_eq!(find_picture_rows(&levels, WIDTH), None);
    }

    #[test]
    fn rejects_input_it_cannot_measure() {
        assert_eq!(find_picture_rows(&[], WIDTH), None);
        // Not a whole number of lines.
        assert_eq!(find_picture_rows(&[0.0; WIDTH + 1], WIDTH), None);
        // Too few lines to find a noise floor.
        assert_eq!(find_picture_rows(&frame(&[(Line::Picture, 8)]), WIDTH), None);
        // Lines too narrow to resolve the burst.
        assert_eq!(find_picture_rows(&[0.0; 32 * 400], 32), None);
    }
}
