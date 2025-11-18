use std::f32::consts::PI;
use std::io::Write;
use tempfile::NamedTempFile;
use voyager_explorer::audio::{WavReader, WaveformChannel};
use voyager_explorer::image_output::image_from_pixels;
use voyager_explorer::sstv::{DecoderParams, SstvDecoder};
use voyager_explorer::utils::format_duration;

/// Create a test WAV file with synthetic SSTV-like data
fn create_test_sstv_wav(sample_rate: u32, duration_secs: f32) -> NamedTempFile {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let mut samples = Vec::new();

    // Generate synthetic SSTV signal with sync tones and image data
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;

        // Add sync tone at regular intervals (every 0.5 seconds)
        let sync_interval = 0.5;
        let is_sync_time = (t % sync_interval) < 0.05; // 50ms sync signal

        let sample = if is_sync_time {
            // Sync tone at 1200 Hz
            (2.0 * PI * 1200.0 * t).sin() * 0.8
        } else {
            // Image data: alternating pattern that creates visible lines
            let pattern = ((t * 1000.0) as i32 % 100) as f32 / 100.0;
            if pattern > 0.5 {
                0.4
            } else {
                -0.4
            }
        };

        let sample_i16 = (sample * i16::MAX as f32) as i16;
        samples.push(sample_i16);
    }

    create_wav_file(&samples, sample_rate, 1)
}

fn create_wav_file(samples: &[i16], sample_rate: u32, channels: u16) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();

    let data_size = (samples.len() * 2) as u32;
    let file_size = data_size + 36;

    // RIFF header
    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();

    // fmt chunk
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes())
        .unwrap();
    file.write_all(&(channels * 2).to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();

    // data chunk
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    for &sample in samples {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }

    file.flush().unwrap();
    file
}

#[test]
fn test_full_workflow_wav_to_image() {
    // Create a test WAV file with synthetic SSTV data
    let temp_wav = create_test_sstv_wav(44100, 2.0); // 2 seconds of audio

    // Load the WAV file
    let wav_reader = WavReader::from_file(temp_wav.path()).expect("Failed to load test WAV");

    // Verify WAV properties
    assert_eq!(wav_reader.sample_rate, 44100);
    assert_eq!(wav_reader.channels, 1);
    assert!(!wav_reader.left_channel.is_empty());

    // Test duration formatting
    let duration_secs = wav_reader.left_channel.len() as f32 / wav_reader.sample_rate as f32;
    let formatted_duration = format_duration(duration_secs);
    assert!(formatted_duration.contains(":"));

    // Create SSTV decoder and test decoding
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 20.0, // Longer lines for test data
        threshold: 0.3,
    };

    // Get samples from left channel
    let samples = wav_reader.get_samples(WaveformChannel::Left);
    assert!(!samples.is_empty());

    // Test sync detection
    let sync_positions = decoder.find_sync_positions(samples, wav_reader.sample_rate);
    println!("Found {} sync positions", sync_positions.len());
    // We should find at least one sync signal
    assert!(
        !sync_positions.is_empty(),
        "Should detect at least one sync signal"
    );

    // Test decoding
    let decoded_pixels = decoder
        .decode(samples, &params, wav_reader.sample_rate)
        .expect("Decode should succeed");
    assert!(!decoded_pixels.is_empty(), "Should decode some pixels");

    // Verify pixel data is valid (all values should be 0 or 255 for binary decoding)
    for &pixel in &decoded_pixels {
        assert!(
            pixel == 0 || pixel == 255,
            "Pixel value should be 0 or 255, got {}",
            pixel
        );
    }

    // Test image creation
    let image = image_from_pixels(&decoded_pixels);
    assert_eq!(image.size[0], 512); // Width should be 512
    assert!(image.size[1] > 0); // Height should be positive
    assert_eq!(image.pixels.len(), image.size[0] * image.size[1]); // Correct pixel count

    // Test seeking functionality
    if sync_positions.len() > 1 {
        let next_sync =
            decoder.find_next_sync(samples, sync_positions[0] + 1000, wav_reader.sample_rate);
        assert!(next_sync.is_some(), "Should find next sync after first one");
        assert!(
            next_sync.unwrap() > sync_positions[0],
            "Next sync should be after first sync"
        );
    }

    println!(
        "Integration test passed: {} pixels decoded from {:.2}s of audio",
        decoded_pixels.len(),
        duration_secs
    );
}

#[test]
fn test_stereo_channel_selection() {
    // Create stereo test data with different content in each channel
    let sample_rate = 44100;
    let duration_secs = 1.0;
    let num_samples = (duration_secs * sample_rate as f32) as usize;

    let mut stereo_samples = Vec::new();
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;

        // Left channel: sync tone
        let left_sample = (2.0 * PI * 1200.0 * t).sin() * 0.5;

        // Right channel: different frequency
        let right_sample = (2.0 * PI * 800.0 * t).sin() * 0.5;

        stereo_samples.push((left_sample * i16::MAX as f32) as i16);
        stereo_samples.push((right_sample * i16::MAX as f32) as i16);
    }

    let temp_wav = create_wav_file(&stereo_samples, sample_rate, 2);
    let wav_reader = WavReader::from_file(temp_wav.path()).unwrap();

    assert_eq!(wav_reader.channels, 2);

    // Get samples from both channels
    let left_samples = wav_reader.get_samples(WaveformChannel::Left);
    let right_samples = wav_reader.get_samples(WaveformChannel::Right);

    assert_eq!(left_samples.len(), right_samples.len());
    assert_ne!(left_samples, right_samples); // Should be different

    // Test sync detection on left channel (should find sync)
    let decoder = SstvDecoder::new();
    let left_sync = decoder.find_sync_positions(left_samples, wav_reader.sample_rate);
    let right_sync = decoder.find_sync_positions(right_samples, wav_reader.sample_rate);

    // Left channel should have more sync detections than right
    assert!(
        left_sync.len() >= right_sync.len(),
        "Left channel should have more sync signals (left: {}, right: {})",
        left_sync.len(),
        right_sync.len()
    );
}

#[test]
fn test_empty_audio_handling() {
    // Test handling of empty or very short audio files
    let empty_samples: Vec<i16> = Vec::new();
    let temp_wav = create_wav_file(&empty_samples, 44100, 1);

    let result = WavReader::from_file(temp_wav.path());
    // Should handle empty file gracefully (might error, which is acceptable)
    match result {
        Ok(reader) => {
            assert!(reader.left_channel.is_empty());

            // Test decoder with empty samples
            let decoder = SstvDecoder::new();
            let params = DecoderParams::default();
            let result = decoder.decode(&reader.left_channel, &params, reader.sample_rate);
            // Should return error for empty samples
            assert!(result.is_err(), "Empty samples should return error");
        }
        Err(_) => {
            // Empty files might legitimately fail to load
            println!("Empty WAV file failed to load (acceptable)");
        }
    }
}

#[test]
fn test_parameter_variations() {
    // Test different decoder parameters
    let temp_wav = create_test_sstv_wav(44100, 1.0);
    let wav_reader = WavReader::from_file(temp_wav.path()).unwrap();
    let samples = wav_reader.get_samples(WaveformChannel::Left);
    let decoder = SstvDecoder::new();

    // Test with different line durations
    let params_fast = DecoderParams {
        line_duration_ms: 5.0,
        threshold: 0.3,
    };
    let params_slow = DecoderParams {
        line_duration_ms: 50.0,
        threshold: 0.3,
    };

    let pixels_fast = decoder
        .decode(samples, &params_fast, wav_reader.sample_rate)
        .expect("Fast decode should succeed");
    let pixels_slow = decoder
        .decode(samples, &params_slow, wav_reader.sample_rate)
        .expect("Slow decode should succeed");

    // Fast decoding should produce more lines (more pixels)
    assert!(
        pixels_fast.len() > pixels_slow.len(),
        "Fast decoding should produce more pixels than slow decoding"
    );

    // Test with different thresholds
    let params_low_thresh = DecoderParams {
        line_duration_ms: 10.0,
        threshold: 0.1,
    };
    let params_high_thresh = DecoderParams {
        line_duration_ms: 10.0,
        threshold: 0.9,
    };

    let pixels_low = decoder
        .decode(samples, &params_low_thresh, wav_reader.sample_rate)
        .expect("Low threshold decode should succeed");
    let pixels_high = decoder
        .decode(samples, &params_high_thresh, wav_reader.sample_rate)
        .expect("High threshold decode should succeed");

    assert_eq!(pixels_low.len(), pixels_high.len()); // Same number of pixels

    // But different distributions of 0s and 255s
    let white_pixels_low = pixels_low.iter().filter(|&&p| p == 255).count();
    let white_pixels_high = pixels_high.iter().filter(|&&p| p == 255).count();

    // Low threshold should produce more white pixels
    assert!(
        white_pixels_low >= white_pixels_high,
        "Low threshold should produce more white pixels"
    );
}
