use voyager_explorer::sstv::{DecoderMode, DecoderParams, SstvDecoder};
use voyager_explorer::test_fixtures::encode_image_to_audio;

// NOTE: Use `encode_image_to_audio` for any future presets/export/color roundtrip tests.
// It mirrors the decoder's timing so images survive a full SSTV image->audio->image cycle.

#[test]
fn test_image_to_audio_and_back_single_line() {
    let width = 512;
    let sample_rate = 51_200; // 10ms line -> 512 samples_per_line
    let line_duration_ms = 10.0;
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms,
        threshold: 0.1,
        decode_window_secs: 2.0,
        mode: DecoderMode::BinaryGrayscale,
    };

    // Line pattern: first half black, second half white.
    let mut line = vec![0u8; width];
    for value in line.iter_mut().skip(width / 2) {
        *value = 255;
    }
    let pixels = line.clone(); // single-line image

    let audio = encode_image_to_audio(&pixels, width, sample_rate, line_duration_ms);
    let decoded = decoder
        .decode(&audio, &params, sample_rate)
        .expect("Decode should succeed");

    assert_eq!(decoded.len(), width, "Should decode one line");
    assert_eq!(decoded, pixels, "Round-trip pixels must match input line");
}

#[test]
fn test_image_to_audio_and_back_two_lines() {
    let width = 512;
    let sample_rate = 51_200;
    let line_duration_ms = 10.0;
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms,
        threshold: 0.1,
        decode_window_secs: 2.0,
        mode: DecoderMode::BinaryGrayscale,
    };

    // Two lines: top white, bottom checker of 8-pixel bars.
    let mut pixels = Vec::with_capacity(width * 2);

    // Line 1: all white
    pixels.extend(std::iter::repeat_n(255u8, width));

    // Line 2: 8-pixel alternating bars to avoid interpolation blur
    let bar = 8;
    for x in 0..width {
        let is_white = (x / bar) % 2 == 0;
        pixels.push(if is_white { 255 } else { 0 });
    }

    let audio = encode_image_to_audio(&pixels, width, sample_rate, line_duration_ms);
    let decoded = decoder
        .decode(&audio, &params, sample_rate)
        .expect("Decode should succeed");

    assert_eq!(
        decoded.len(),
        pixels.len(),
        "Decoded size should match input"
    );
    assert_eq!(decoded, pixels, "Round-trip pixels must match both lines");
}
