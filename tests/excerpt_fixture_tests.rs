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

use voyager_explorer::analysis::{find_image_bounds, SegmentImagesParams};
use voyager_explorer::audio::WavReader;
use voyager_explorer::pipeline::composite_triplet_levels;
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
