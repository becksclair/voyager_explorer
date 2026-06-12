//! Round-trip tests against an independent forward model of the record
//! encoding (sync spike + falling edge per line, luminance as signal level).
//! Decodes assert statistical similarity to the source image rather than
//! exact pixel equality — the encoder deliberately does not mirror decoder
//! internals.

use voyager_explorer::sstv::{DecoderParams, SstvDecoder};
use voyager_explorer::test_fixtures::{encode_image_to_audio, encode_image_to_audio_with, EncodeOptions};

/// Pearson correlation between two equal-length pixel slices.
fn correlation(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let n = a.len() as f64;
    let ma = a.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mb = b.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mut cov = 0.0;
    let mut va = 0.0;
    let mut vb = 0.0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let dx = x as f64 - ma;
        let dy = y as f64 - mb;
        cov += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    if va == 0.0 || vb == 0.0 {
        return 0.0;
    }
    cov / (va * vb).sqrt()
}

/// Source image: per-line gradient with a moving bright bar — enough spatial
/// structure that misaligned lines destroy the correlation.
fn test_image(width: usize, n_lines: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * n_lines);
    for line in 0..n_lines {
        let bar_start = (line * width) / n_lines;
        for x in 0..width {
            let gradient = (x * 160 / width) as u8;
            let bar = if x >= bar_start && x < bar_start + width / 8 { 95 } else { 0 };
            pixels.push(gradient + bar);
        }
    }
    pixels
}

/// Compare decoded rows to source rows, ignoring the sync-adjacent edges
/// (the trailing sync spike of the following line lives there).
fn image_correlation(source: &[u8], decoded: &[u8], width: usize) -> f64 {
    let n_lines = source.len() / width;
    let decoded_lines = decoded.len() / width;
    let lines = n_lines.min(decoded_lines);
    assert!(lines > 0);
    let margin = width / 10;
    let mut src_interior = Vec::new();
    let mut dec_interior = Vec::new();
    for line in 0..lines {
        src_interior.extend_from_slice(&source[line * width + margin..(line + 1) * width - margin]);
        dec_interior.extend_from_slice(&decoded[line * width + margin..(line + 1) * width - margin]);
    }
    correlation(&src_interior, &dec_interior)
}

/// `image_correlation` with the decoded buffer shifted down by `offset` lines.
fn offset_correlation(source: &[u8], decoded: &[u8], width: usize, offset: usize) -> f64 {
    image_correlation(&source[offset * width..], decoded, width)
}

#[test]
fn roundtrip_clean_signal() {
    let width = 512;
    let n_lines = 64;
    let sample_rate = 48_000;
    let params = DecoderParams::default();
    let pixels = test_image(width, n_lines);

    let audio = encode_image_to_audio(&pixels, width, sample_rate, params.line_duration_ms);
    let decoded = SstvDecoder::new().decode(&audio, &params, sample_rate).expect("decode");

    let corr = image_correlation(&pixels, &decoded, width);
    assert!(corr > 0.9, "clean roundtrip correlation too low: {corr:.3}");

    // Line alignment must be exact: a systematic ±1-line offset would still
    // correlate fairly well on smooth imagery, so check that offset 0 beats
    // shifted pairings outright.
    let corr_plus = offset_correlation(&pixels, &decoded, width, 1);
    let corr_minus = offset_correlation(&decoded, &pixels, width, 1);
    assert!(
        corr > corr_plus && corr > corr_minus,
        "decoded lines misaligned: corr@0={corr:.3} corr@+1={corr_plus:.3} corr@-1={corr_minus:.3}"
    );
}

#[test]
fn roundtrip_with_slant_requires_sync_lock() {
    let width = 512;
    let n_lines = 64;
    let sample_rate = 48_000;
    let pixels = test_image(width, n_lines);

    // Half-sample-per-line drift accumulates to ~32 samples (8% of a line)
    // across the image — fatal for fixed-period slicing.
    let opts = EncodeOptions {
        slant_samples_per_line: 0.5,
        noise_amplitude: 0.0,
    };
    let audio = encode_image_to_audio_with(&pixels, width, sample_rate, 8.32, &opts);
    let decoder = SstvDecoder::new();

    let locked = decoder
        .decode(&audio, &DecoderParams::default(), sample_rate)
        .expect("decode");
    let corr_locked = image_correlation(&pixels, &locked, width);
    assert!(corr_locked > 0.85, "sync-locked decode under slant too low: {corr_locked:.3}");

    let unlocked = decoder
        .decode(
            &audio,
            &DecoderParams {
                sync_lock: false,
                ..Default::default()
            },
            sample_rate,
        )
        .expect("decode");
    let corr_unlocked = image_correlation(&pixels, &unlocked, width);
    assert!(
        corr_locked > corr_unlocked,
        "sync lock should beat fixed slicing under slant: locked={corr_locked:.3} unlocked={corr_unlocked:.3}"
    );
}

#[test]
fn roundtrip_with_noise() {
    let width = 512;
    let n_lines = 64;
    let sample_rate = 48_000;
    let params = DecoderParams::default();
    let pixels = test_image(width, n_lines);

    let opts = EncodeOptions {
        slant_samples_per_line: 0.2,
        noise_amplitude: 0.05,
    };
    let audio = encode_image_to_audio_with(&pixels, width, sample_rate, params.line_duration_ms, &opts);
    let decoded = SstvDecoder::new().decode(&audio, &params, sample_rate).expect("decode");

    let corr = image_correlation(&pixels, &decoded, width);
    assert!(corr > 0.7, "noisy roundtrip correlation too low: {corr:.3}");
}

/// Gate 1 regression: decode the real record excerpt and assert the output is
/// a structured grayscale image, not a degenerate blob. Ignored by default
/// because it needs the multi-megabyte asset; run with `--ignored` locally.
#[test]
#[ignore = "requires assets/sync_image1.wav"]
fn decode_real_sync_image1() {
    let reader = voyager_explorer::audio::WavReader::from_file("assets/sync_image1.wav").expect("load asset");
    let samples = reader.left_channel.as_ref();
    let params = DecoderParams::default();
    let decoded = SstvDecoder::new()
        .decode(samples, &params, reader.sample_rate)
        .expect("decode");

    let width = params.effective_width();
    let lines = decoded.len() / width;
    assert!(lines >= 400, "expected >=400 scan lines, got {lines}");

    // Non-degenerate histogram: meaningful spread of gray levels
    let distinct: std::collections::BTreeSet<u8> = decoded.iter().copied().collect();
    assert!(distinct.len() > 64, "only {} distinct levels", distinct.len());

    // Both dark and bright populations exist
    let dark = decoded.iter().filter(|&&p| p < 64).count();
    let bright = decoded.iter().filter(|&&p| p > 192).count();
    assert!(dark > decoded.len() / 100, "almost no dark pixels");
    assert!(bright > decoded.len() / 1000, "almost no bright pixels");
}
