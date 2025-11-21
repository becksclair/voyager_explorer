use voyager_explorer::sstv::{DecoderMode, DecoderParams, SstvDecoder};

#[test]
fn test_pseudocolor_decoding_logic() {
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0, // 10ms per line
        threshold: 0.0,         // No threshold for this test
        decode_window_secs: 1.0,
        mode: DecoderMode::PseudoColor,
    };

    let sample_rate = 44100;
    let samples_per_line = (sample_rate as f32 * params.line_duration_ms / 1000.0) as usize;

    // Create 3 lines of data:
    // Line 1 (Red): High intensity (1.0) -> 255
    // Line 2 (Green): Low intensity (0.0) -> 0
    // Line 3 (Blue): Low intensity (0.0) -> 0

    let mut samples = Vec::new();

    // Line 1: Red channel (all 1.0)
    samples.extend(std::iter::repeat_n(1.0, samples_per_line));

    // Line 2: Green channel (all 0.0)
    samples.extend(std::iter::repeat_n(0.0, samples_per_line));

    // Line 3: Blue channel (all 0.0)
    samples.extend(std::iter::repeat_n(0.0, samples_per_line));

    let pixels = decoder
        .decode(&samples, &params, sample_rate)
        .expect("Decode failed");

    assert_eq!(
        pixels.len(),
        512 * 3,
        "Should produce exactly one line of RGB pixels"
    );

    // Verify pixel values
    // R should be 255 (from 1.0 input)
    // G should be 0 (from 0.0 input)
    // B should be 0 (from 0.0 input)

    for i in 0..512 {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];

        assert_eq!(r, 255, "Red channel should be 255");
        assert_eq!(g, 0, "Green channel should be 0");
        assert_eq!(b, 0, "Blue channel should be 0");
    }
}

#[test]
fn test_pseudocolor_partial_lines() {
    // Test with 4 lines (1 full RGB line + 1 extra line)
    // The extra line should be ignored or handled gracefully (implementation detail: currently ignored if not multiple of 3?)
    // Actually, my implementation processes all lines, but image_from_pixels handles the grouping.
    // Wait, SstvDecoder::decode for PseudoColor groups lines.
    // Let's verify what SstvDecoder::decode does.

    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0,
        threshold: 0.0,
        decode_window_secs: 1.0,
        mode: DecoderMode::PseudoColor,
    };

    let sample_rate = 44100;
    let samples_per_line = (sample_rate as f32 * params.line_duration_ms / 1000.0) as usize;

    let mut samples = Vec::new();
    // 4 lines of data
    samples.extend(std::iter::repeat_n(1.0, samples_per_line * 4));

    let pixels = decoder
        .decode(&samples, &params, sample_rate)
        .expect("Decode failed");

    // Should produce 1 RGB line (3 source lines)
    // The 4th line is dropped because it doesn't form a complete RGB triplet?
    // Let's check the implementation logic again.
    // "chunks_exact(3)" was used? No, I implemented it manually.
    // If I implemented it to consume 3 lines at a time, the leftover line is dropped.

    assert_eq!(
        pixels.len(),
        512 * 3,
        "Should produce exactly one line of RGB pixels from 4 input lines"
    );
}
