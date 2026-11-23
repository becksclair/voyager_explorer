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
use voyager_explorer::pipeline::{composite_triplet_levels, DecodingPipeline};
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
