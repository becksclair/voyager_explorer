use voyager_explorer::sstv::{DecoderMode, DecoderParams, SstvDecoder};

#[test]
fn test_pseudocolor_decoding_logic() {
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0,
        sync_lock: false, // plain level lines, no sync structure
        mode: DecoderMode::PseudoColor,
        ..Default::default()
    };

    let sample_rate = 44100;
    let samples_per_line = (sample_rate as f32 * params.line_duration_ms / 1000.0) as usize;

    // Three lines: R=high level, G=low, B=low -> red pixels after the
    // percentile stretch maps the level range to 0..255.
    let mut samples = Vec::new();
    samples.extend(std::iter::repeat_n(1.0, samples_per_line));
    samples.extend(std::iter::repeat_n(0.0, samples_per_line));
    samples.extend(std::iter::repeat_n(0.0, samples_per_line));

    let pixels = decoder.decode(&samples, &params, sample_rate).expect("Decode failed");

    assert_eq!(pixels.len(), 512 * 3, "Should produce exactly one line of RGB pixels");

    for i in 0..512 {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];

        assert!(r > 200, "Red channel should be bright, got {r}");
        assert!(g < 50, "Green channel should be dark, got {g}");
        assert!(b < 50, "Blue channel should be dark, got {b}");
    }
}

#[test]
fn test_pseudocolor_partial_lines() {
    // 4 lines = 1 complete RGB triplet + 1 leftover line that must be dropped.
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0,
        sync_lock: false,
        mode: DecoderMode::PseudoColor,
        ..Default::default()
    };

    let sample_rate = 44100;
    let samples_per_line = (sample_rate as f32 * params.line_duration_ms / 1000.0) as usize;

    let samples = vec![1.0; samples_per_line * 4];
    let pixels = decoder.decode(&samples, &params, sample_rate).expect("Decode failed");

    assert_eq!(pixels.len(), 512 * 3, "Should produce exactly one line of RGB pixels");
}

#[test]
fn test_pseudocolor_preserves_gray_levels() {
    // Intermediate levels must yield intermediate channel intensities — the
    // old binary decoder could only produce 8 colors.
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0,
        sync_lock: false,
        mode: DecoderMode::PseudoColor,
        ..Default::default()
    };

    let sample_rate = 44100;
    let samples_per_line = (sample_rate as f32 * params.line_duration_ms / 1000.0) as usize;

    let mut samples = Vec::new();
    samples.extend(std::iter::repeat_n(1.0, samples_per_line)); // R full
    samples.extend(std::iter::repeat_n(0.5, samples_per_line)); // G mid
    samples.extend(std::iter::repeat_n(0.0, samples_per_line)); // B none

    let pixels = decoder.decode(&samples, &params, sample_rate).expect("Decode failed");
    let g = pixels[1];
    assert!((90..=170).contains(&g), "mid-level green should land mid-range, got {g}");
}
