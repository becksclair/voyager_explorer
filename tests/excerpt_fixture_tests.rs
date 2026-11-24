//! Integration tests against the multi-image excerpt fixture
//! `assets/golden_record_excerpt_48k.wav` — the left channel of the 48 kHz
//! rip, lead-in tone plus 16 frames (calibration circle through the
//! "Home"/Earth color triplet). The asset is gitignored; regenerate it with
//! `just fixtures`. These tests are ignored by default (they need the
//! multi-megabyte asset); run them with:
//!
//! ```text
//! cargo test --test excerpt_fixture_tests -- --ignored
//! ```

use voyager_explorer::analysis::sync::track_line_syncs;
use voyager_explorer::analysis::{find_image_bounds, SegmentImagesParams, SyncParams};
use voyager_explorer::audio::WavReader;
use voyager_explorer::pipeline::{composite_triplet_levels, composite_triplet_levels_with_picture_rows, DecodingPipeline};
use voyager_explorer::sstv::{DecoderMode, DecoderParams, SstvDecoder};

const FIXTURE: &str = "assets/golden_record_excerpt_48k.wav";

/// Segmentation must recover exactly the 16 catalog frames carved into the
/// fixture, and must reject the lead-in calibration tone rather than counting
/// it as a 17th image (the tone sync-locks like an image, so cadence alone
/// cannot drop it — classification does).
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_segments_into_sixteen_frames() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();

    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());

    assert_eq!(
        bounds.len(),
        16,
        "expected 16 image candidates (calibration circle → Earth), got {}",
        bounds.len()
    );
    // The lead-in tone occupies the first few seconds; the first real image
    // (the calibration circle) starts only after it, proving tone rejection.
    assert!(
        bounds[0].start_secs > 3.0,
        "first image should start after the lead-in tone, got {:.2}s",
        bounds[0].start_secs
    );
    // Every candidate is a full image, not a truncated sliver. Real frames on
    // this rip carry ~550-750 sync-locked lines; 400 is a loose floor that
    // still sits well above any inter-image fragment.
    for (i, b) in bounds.iter().enumerate() {
        assert!(b.line_count >= 400, "frame {i} has only {} scan lines", b.line_count);
    }
}

/// Every frame's picture must be found inside its segmented range: the
/// leader lines and start burst around it vary from frame to frame, the
/// picture itself does not. This goes through the pipeline, as the GUI
/// preview does.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_frames_crop_to_their_pictures() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();
    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());
    let pipeline = DecodingPipeline::new();

    let pictures: Vec<_> = bounds
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let result = pipeline
                .process(
                    &samples[b.start_sample..b.end_sample],
                    &DecoderParams::default(),
                    reader.sample_rate,
                )
                .unwrap_or_else(|e| panic!("frame {i} decodes: {e:#}"));
            let rows = result.picture_rows.unwrap_or_else(|| panic!("frame {i}: no picture found"));
            (result.height as usize, rows)
        })
        .collect();

    // Pictures on this rip run 519-534 lines.
    for (i, (_, rows)) in pictures.iter().enumerate() {
        assert!(
            (515..=540).contains(&rows.len()),
            "frame {i}: picture is {} lines ({rows:?})",
            rows.len()
        );
    }

    // Frame 10 (a lunar photograph) is the hard case: uniform borders inside
    // the picture, and about 120 lines of leader and burst before it. The
    // borders stay, the leader goes.
    let (height, rows) = &pictures[10];
    assert!(rows.start >= 100, "frame 10 keeps leader before the picture: {rows:?}");
    assert!(
        height - rows.end >= 20,
        "frame 10 keeps leader after the picture: {rows:?} of {height}"
    );
}

/// Frames 0 and 5 open on the start burst. The line tracker must take its
/// phase from the sync, not the burst tone: seeded on the tone it runs at
/// the wrong period and shears the first 40 or so picture lines.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_frames_opening_on_a_burst_are_not_sheared() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();
    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());

    for frame in [0, 5] {
        let b = &bounds[frame];
        let positions = track_line_syncs(
            &samples[b.start_sample..b.end_sample],
            reader.sample_rate,
            &SyncParams::default(),
        );
        let intervals: Vec<f64> = positions.windows(2).map(|w| (w[1] - w[0]) as f64).collect();
        let mut sorted = intervals.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let off_cadence = intervals
            .iter()
            .take(80)
            .filter(|&&i| (i - median).abs() > median * 0.0075)
            .count();
        assert_eq!(
            off_cadence, 0,
            "frame {frame}: {off_cadence} of the first 80 lines are off the {median}-sample cadence"
        );
    }
}

/// The "Home"/Earth color triplet must register and composite. On the left
/// channel it is frames 13 (blue), 14 (green), 15 (red); see `catalog.rs`.
/// `composite_triplet_levels` takes the planes in `[red, green, blue]` order
/// on the raw decoded levels and crops to the registered overlap.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_earth_triplet_composites() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();

    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());
    assert_eq!(
        bounds.len(),
        16,
        "fixture must segment to 16 frames first, got {}",
        bounds.len()
    );

    let params = DecoderParams {
        mode: DecoderMode::Grayscale,
        ..DecoderParams::default()
    };
    let decoder = SstvDecoder::new();
    let width = params.effective_width();
    let decode = |idx: usize| {
        let b = &bounds[idx];
        decoder
            .decode_levels(&samples[b.start_sample..b.end_sample], &params, reader.sample_rate)
            .unwrap_or_else(|e| panic!("decode frame {idx}: {e:#}"))
    };
    let (red, grn, blu) = (decode(15), decode(14), decode(13));

    let img = composite_triplet_levels([&red, &grn, &blu], width, false, 1.0).expect("Earth triplet composites");

    // Plane width, cropped to the registered row overlap — a substantial
    // image. Same loose floor as segmentation above: ~550-750 lines per frame
    // on this rip, so 400 rows after registration crop is comfortably a real
    // composite, not a sliver.
    assert_eq!(img.width() as usize, width, "composite width should match the plane width");
    assert!(img.height() >= 400, "composite only {} rows tall", img.height());

    // A real color photo is not a flat fill: the three planes carry distinct
    // values, so some pixels differ across channels.
    let rgb = img.to_rgb8();
    let varied = rgb.pixels().any(|p| p[0] != p[1] || p[1] != p[2]);
    assert!(
        varied,
        "composite is uniform gray — registration/compositing produced no color"
    );
}

/// The Solar Spectrum triplet (left channel frames 7=blue, 8=green, 9=red)
/// carries spectrally distinct absorption bands across its three planes, so
/// global NCC slides the bands onto each other. Anchoring registration to each
/// plane's detected `picture_rows` keeps the full picture intact and preserves
/// the spatial separation of the red, green, and blue spectral bands.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_solar_spectrum_triplet_registers_with_picture_rows() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();

    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());
    assert_eq!(bounds.len(), 16, "fixture must segment to 16 frames");

    let params = DecoderParams {
        mode: DecoderMode::Grayscale,
        ..DecoderParams::default()
    };
    let decoder = SstvDecoder::new();
    let width = params.effective_width();
    let preroll_len = (reader.sample_rate as f64 * voyager_explorer::sstv::DC_RESTORE_PREROLL_SECS) as usize;
    let decode = |idx: usize| {
        let b = &bounds[idx];
        let preroll = &samples[b.start_sample.saturating_sub(preroll_len)..b.start_sample];
        decoder
            .decode_levels_with_metadata(preroll, &samples[b.start_sample..b.end_sample], &params, reader.sample_rate)
            .unwrap_or_else(|e| panic!("decode frame {idx}: {e:#}"))
    };
    let (red, grn, blu) = (decode(9), decode(8), decode(7));
    let pic_rows = [red.picture_rows.clone(), grn.picture_rows.clone(), blu.picture_rows.clone()];

    let img = composite_triplet_levels_with_picture_rows([&red.levels, &grn.levels, &blu.levels], &pic_rows, width, false, 1.0)
        .expect("Solar Spectrum triplet composites");

    // Global NCC shifts blue by ~192 lines and shrinks the overlap to ~386 rows;
    // picture-row-anchored registration preserves almost the full ~578-row frame.
    assert!(
        img.height() >= 550,
        "Solar Spectrum composite truncated to {} rows (likely misregistered)",
        img.height()
    );

    // Inside the picture's central spectrum band (cols 45%..65%), each channel's
    // absorption trough sits in a distinct region along the scan-line axis:
    // red trough < green trough < blue trough, well separated (>40 rows apart).
    let rgb = img.to_rgb8();
    let c0 = (width * 45 / 100) as u32;
    let c1 = (width * 65 / 100) as u32;
    let trough_row = |ch: usize| -> usize {
        (50..img.height() - 50)
            .min_by_key(|&y| (c0..c1).map(|x| u32::from(rgb.get_pixel(x, y)[ch])).sum::<u32>())
            .expect("non-empty row range") as usize
    };
    let r_trough = trough_row(0);
    let g_trough = trough_row(1);
    let b_trough = trough_row(2);
    assert!(
        g_trough > r_trough + 40 && b_trough > g_trough + 40,
        "expected distinct ordered R/G/B spectral troughs, got R={r_trough} G={g_trough} B={b_trough}"
    );
}

/// Window decodes spanning an inter-image boundary must re-acquire the next
/// image's sync phase on its leader instead of coasting at the previous
/// image's phase until the start burst.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_window_across_image_boundary_reacquires_leader_phase() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();
    let sr = reader.sample_rate as usize;

    // Boundaries 0->1 (9.65s..9.70s) and 3->4 (27.60s..28.37s) both jump phase
    // by >80 samples across the splice, well outside the ±6% tracking window.
    for (start_s, dur_s) in [(9.0, 2.0), (27.0, 2.5)] {
        let s0 = (start_s * sr as f64) as usize;
        let s1 = (s0 + (dur_s * sr as f64) as usize).min(samples.len());
        let window = &samples[s0..s1];

        let tracked = track_line_syncs(window, reader.sample_rate, &SyncParams::default());
        // The next image's leader sits in the middle third of each window;
        // every tracked line start there must sit near a real positive sync
        // spike (within the preceding 25 samples), not in the flat mid-line.
        let max_abs = window.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
        let mid_lines: Vec<usize> = tracked
            .iter()
            .copied()
            .filter(|&p| p > window.len() / 3 && p < window.len() * 2 / 3)
            .collect();
        assert!(mid_lines.len() >= 40, "expected >=40 leader lines, got {}", mid_lines.len());
        let locked_near_spike = mid_lines
            .iter()
            .filter(|&&p| {
                let lo = p.saturating_sub(25);
                let spike = window[lo..=p].iter().copied().fold(f32::NEG_INFINITY, f32::max);
                spike > max_abs * 0.35
            })
            .count();
        assert!(
            locked_near_spike * 10 >= mid_lines.len() * 8,
            "window at {start_s}s: only {locked_near_spike}/{} leader lines locked to a sync spike",
            mid_lines.len()
        );
    }
}

/// DC restoration must flatten the AC-coupling droop on black-background
/// diagrams (Frame 0: Calibration Circle) relative to an un-restored decode.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_dc_restore_flattens_black_diagram_background() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();
    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());
    let f0 = &bounds[0];
    let slice = &samples[f0.start_sample..f0.end_sample];
    let decoder = SstvDecoder::new();

    let restored = decoder
        .decode_levels(slice, &DecoderParams::default(), reader.sample_rate)
        .expect("restored decode");
    let raw = decoder
        .decode_levels(
            slice,
            &DecoderParams {
                dc_restore: false,
                ..DecoderParams::default()
            },
            reader.sample_rate,
        )
        .expect("raw decode");

    // Average pure-black border rows 50..80 (after start burst, before circle).
    let width = 512usize;
    let col_std = |levels: &[f32]| -> f64 {
        let mut avg = vec![0.0f64; width];
        for row in 50..80 {
            for (x, slot) in avg.iter_mut().enumerate() {
                *slot += f64::from(levels[row * width + x]) / 30.0;
            }
        }
        // Content region inside sync bands (12%..86% of line width).
        let sub = &avg[width * 12 / 100..width * 86 / 100];
        let mean = sub.iter().sum::<f64>() / sub.len() as f64;
        (sub.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / sub.len() as f64).sqrt()
    };

    let raw_std = col_std(&raw);
    let restored_std = col_std(&restored);
    assert!(
        restored_std < raw_std * 0.4,
        "DC restoration did not flatten Frame 0 black background: raw_std={raw_std:.5} restored_std={restored_std:.5}"
    );
}

/// A window opening mid-picture (70% into Frame 6) must match the full-frame
/// DC-restored decode from line 0 onward when given real pre-roll, and stay
/// close via 2-line periodic warm-up when decoded standalone without pre-roll.
#[test]
#[ignore = "requires assets/golden_record_excerpt_48k.wav (just fixtures)"]
fn excerpt_mid_picture_window_has_no_dc_restore_startup_transient() {
    let reader = WavReader::from_file(FIXTURE).expect("load fixture");
    let samples = reader.left_channel.as_ref();
    let bounds = find_image_bounds(samples, reader.sample_rate, &SegmentImagesParams::default());
    let f6 = &bounds[6];
    let params = DecoderParams::default();
    let decoder = SstvDecoder::new();
    let width = params.effective_width();

    // Full-frame DC-restored reference (settled well before 70% into the frame).
    let full_levels = decoder
        .decode_levels(&samples[f6.start_sample..f6.end_sample], &params, reader.sample_rate)
        .expect("full frame decode");
    let full_rows = full_levels.len() / width;

    // 2 s window starting 70% into Frame 6.
    let win_start = f6.start_sample + (f6.end_sample - f6.start_sample) * 70 / 100;
    let win_end = (win_start + 2 * reader.sample_rate as usize).min(samples.len());
    let preroll_len = (reader.sample_rate as f64 * voyager_explorer::sstv::DC_RESTORE_PREROLL_SECS) as usize;
    let preroll = &samples[win_start.saturating_sub(preroll_len)..win_start];
    let window = &samples[win_start..win_end];

    let with_preroll = decoder
        .decode_levels_with_preroll(preroll, window, &params, reader.sample_rate)
        .expect("preroll window decode");
    let standalone = decoder
        .decode_levels(window, &params, reader.sample_rate)
        .expect("standalone window decode");

    // Align window's line 0 to the matching row in `full_levels` using row 10
    // (well past any possible transient).
    let c0 = width * 12 / 100;
    let c1 = width * 86 / 100;
    let row_err = |a: &[f32], ra: usize, b: &[f32], rb: usize| -> f64 {
        (c0..c1)
            .map(|x| f64::from((a[ra * width + x] - b[rb * width + x]).abs()))
            .sum::<f64>()
            / (c1 - c0) as f64
    };
    let approx_row = (full_rows * 70 / 100).saturating_sub(10);
    let search_end = (approx_row + 40).min(full_rows - 15);
    let best_offset = (approx_row..search_end)
        .min_by(|&r1, &r2| {
            row_err(&with_preroll, 10, &full_levels, r1 + 10).total_cmp(&row_err(&with_preroll, 10, &full_levels, r2 + 10))
        })
        .expect("matching row offset");

    let ref_slice = &full_levels[best_offset * width..(best_offset + 10) * width];
    let min_v = ref_slice.iter().copied().fold(f32::INFINITY, f32::min);
    let max_v = ref_slice.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = f64::from((max_v - min_v).max(1e-6));

    let preroll_err0 = row_err(&with_preroll, 0, &full_levels, best_offset) / span;
    let standalone_err0 = row_err(&standalone, 0, &full_levels, best_offset) / span;
    assert!(
        preroll_err0 < 0.02,
        "pre-roll window line 0 error too high: {:.2}%",
        preroll_err0 * 100.0
    );
    assert!(
        standalone_err0 < 0.05,
        "standalone 2-line warmup line 0 error too high: {:.2}%",
        standalone_err0 * 100.0
    );
}
