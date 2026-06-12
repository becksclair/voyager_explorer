//! Playback position tracking anchored to the audio device, not a UI timer.
//!
//! rodio's `Sink::get_pos()` reports how much of the current source has been
//! played. Sources are appended starting at a base sample offset (seeks
//! rebuild the source at the new offset), so the true playhead is
//! `base + get_pos · sample_rate`. The previous frame-clocked
//! `Instant::elapsed()` approach drifted from the device under UI load and
//! desynchronized the live decode window; this math cannot drift because the
//! device itself is the clock.

use std::time::Duration;

/// Absolute playhead position in samples for a source that was appended at
/// `base_samples` and has played for `sink_pos` according to the sink.
pub fn position_samples(base_samples: usize, sink_pos: Duration, sample_rate: u32) -> usize {
    base_samples + (sink_pos.as_secs_f64() * sample_rate as f64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_at_start_is_base() {
        assert_eq!(position_samples(1000, Duration::ZERO, 48_000), 1000);
    }

    #[test]
    fn position_advances_with_sink_time() {
        // 0.5 s at 48 kHz = 24000 samples past base
        assert_eq!(position_samples(1000, Duration::from_millis(500), 48_000), 25_000);
    }

    #[test]
    fn position_is_exact_at_high_rates() {
        // 384 kHz master rate, 2.25 s
        assert_eq!(position_samples(0, Duration::from_millis(2250), 384_000), 864_000);
    }

    #[test]
    fn fractional_durations_truncate() {
        // Sub-sample remainders truncate rather than round up past the playhead
        let pos = position_samples(0, Duration::from_nanos(20_833), 48_000); // ~1 sample
        assert_eq!(pos, 0); // 20.833 µs < 1/48000 s (~20.83 µs boundary edge)
    }
}
