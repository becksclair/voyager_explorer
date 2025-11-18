//! Comprehensive audio playback tests using synthetic fixtures
//!
//! These tests validate the entire audio playback system without requiring
//! real audio hardware. We use synthetic audio signals with known properties
//! to verify correctness.

use voyager_explorer::audio::WavReader;
use voyager_explorer::audio_state::{AudioError, AudioPlaybackState};
use voyager_explorer::sstv::{DecoderParams, SstvDecoder};
use voyager_explorer::test_fixtures::*;

#[test]
fn test_wav_loading_with_synthetic_tone() {
    // Generate 440Hz A4 tone for 0.5 seconds
    let signal = generate_sine_wave(440.0, 0.5, 44100, 0.6);
    let wav_file = create_test_wav_file(&signal, 44100, 1);

    // Load it via WavReader
    let reader = WavReader::from_file(wav_file.path()).expect("load wav");

    // Verify properties
    assert_eq!(reader.sample_rate, 44100);
    assert_eq!(reader.channels, 1);
    assert_eq!(reader.left_channel.len(), 22050); // 0.5s * 44100
    assert_eq!(reader.right_channel.len(), 22050); // Mono duplicated

    // Verify amplitude is approximately 0.6
    let max_amplitude = reader
        .left_channel
        .iter()
        .map(|&s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(
        (max_amplitude - 0.6).abs() < 0.05,
        "Amplitude should be ~0.6"
    );
}

#[test]
fn test_sync_detection_with_synthetic_pattern() {
    let decoder = SstvDecoder::new();

    // Generate pattern: sync (1200Hz) + silence + sync
    let sync_signal = generate_sync_pattern(44100);

    // Find all sync positions
    let positions = decoder.find_sync_positions(&sync_signal, 44100);

    // Should find at least 2 sync positions
    assert!(
        positions.len() >= 2,
        "Should detect multiple sync signals, found {}",
        positions.len()
    );

    // Syncs should be separated (detection uses chunk-based scanning)
    if positions.len() >= 2 {
        let separation = positions[1] - positions[0];
        // Due to FFT chunk-based detection, separation will be in multiples of chunks
        // Just verify they're reasonably separated (not the same position)
        assert!(
            separation > 1000,
            "Syncs should be separated by more than 1000 samples, got {}",
            separation
        );
    }
}

#[test]
fn test_decoding_produces_consistent_output() {
    let decoder = SstvDecoder::new();
    let params = DecoderParams {
        line_duration_ms: 10.0,
        threshold: 0.3,
    };

    // Generate square wave for clear pattern
    let signal = generate_square_wave(50.0, 0.5, 44100, 0.8);

    // Decode once
    let pixels1 = decoder
        .decode(&signal, &params, 44100)
        .expect("Decode should succeed");

    // Decode again with same input
    let pixels2 = decoder
        .decode(&signal, &params, 44100)
        .expect("Decode should succeed");

    // Should produce identical output
    assert_eq!(pixels1.len(), pixels2.len());
    assert_eq!(pixels1, pixels2, "Decoder should be deterministic");

    // All pixels should be 0 or 255 (binary)
    for &pixel in &pixels1 {
        assert!(pixel == 0 || pixel == 255, "Pixels should be binary");
    }

    // Should have reasonable number of lines
    let num_lines = pixels1.len() / 512;
    assert!(num_lines > 0, "Should produce at least one line");
}

#[test]
fn test_audio_state_transitions() {
    // Test state machine without actual audio hardware
    let mut state = AudioPlaybackState::Uninitialized;

    // Initially uninitialized
    assert!(!state.is_playing());
    assert!(!state.can_play());

    // Transition to Ready (simulating WAV load + device available)
    state = AudioPlaybackState::Ready;
    assert!(state.can_play());
    assert_eq!(state.status_icon(), "🔊");

    // Transition to Playing
    state = AudioPlaybackState::Playing;
    assert!(state.is_playing());
    assert_eq!(state.status_icon(), "▶️");

    // Transition to Paused
    state = AudioPlaybackState::Paused;
    assert!(!state.is_playing());
    assert!(state.can_play());
    assert_eq!(state.status_icon(), "⏸️");

    // Back to Playing
    state = AudioPlaybackState::Playing;
    assert!(state.is_playing());

    // Error state
    state = AudioPlaybackState::Error(AudioError::DeviceDisconnected);
    assert!(state.is_error());
    assert_eq!(state.status_icon(), "⚠️");
    assert!(state.error().unwrap().user_action().contains("Reconnect"));
}

#[test]
fn test_seek_positions_are_valid() {
    let decoder = SstvDecoder::new();

    // Generate composite signal with known structure
    let signal = generate_composite_signal(44100);
    let total_samples = signal.len();

    // Find all sync positions
    let positions = decoder.find_sync_positions(&signal, 44100);

    // All positions should be within bounds
    for &pos in &positions {
        assert!(
            pos < total_samples,
            "Sync position {} out of bounds (total: {})",
            pos,
            total_samples
        );
    }

    // Test find_next_sync from various positions
    for start_pos in [0, total_samples / 4, total_samples / 2] {
        if let Some(next_pos) = decoder.find_next_sync(&signal, start_pos, 44100) {
            assert!(
                next_pos > start_pos,
                "Next sync should be after start position"
            );
            assert!(next_pos < total_samples, "Next sync should be in bounds");
        }
    }
}

#[test]
fn test_chirp_signal_properties() {
    // Generate chirp from 200Hz to 2000Hz
    let chirp = generate_chirp(200.0, 2000.0, 1.0, 44100, 0.7);

    assert_eq!(chirp.len(), 44100, "Should be 1 second at 44.1kHz");

    // Amplitude should stay within expected range
    let max_amp = chirp
        .iter()
        .map(|&s| s.abs())
        .fold::<f32, _>(0.0f32, f32::max);
    assert!(
        (max_amp - 0.7).abs() < 0.05,
        "Chirp amplitude should be ~0.7"
    );

    // Signal should not be constant (it's sweeping frequencies)
    let first_quarter = &chirp[0..11025];
    let last_quarter = &chirp[33075..44100];

    let avg_first: f32 = first_quarter.iter().map(|s| s.abs()).sum::<f32>() / 11025.0;
    let avg_last: f32 = last_quarter.iter().map(|s| s.abs()).sum::<f32>() / 11025.0;

    // Both should have significant energy (not silence)
    assert!(avg_first > 0.1, "First quarter should have energy");
    assert!(avg_last > 0.1, "Last quarter should have energy");
}

#[test]
fn test_stereo_wav_generation() {
    // Create stereo signal (different for each channel)
    let left_signal = generate_sine_wave(440.0, 0.2, 44100, 0.5);
    let right_signal = generate_sine_wave(880.0, 0.2, 44100, 0.5);

    // Interleave for stereo
    let mut stereo_samples = Vec::with_capacity(left_signal.len() * 2);
    for i in 0..left_signal.len() {
        stereo_samples.push((left_signal[i].clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        stereo_samples.push((right_signal[i].clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
    }

    // TODO: Add proper stereo WAV test when we have interleaving helper
    // let _wav_file = create_test_wav_file(&stereo_samples, 44100, 2);

    // For now, test mono → stereo duplication
    let mono_signal = generate_sine_wave(440.0, 0.1, 44100, 0.5);
    let mono_wav = create_test_wav_file(&mono_signal, 44100, 1);

    let reader = WavReader::from_file(mono_wav.path()).expect("load mono wav");
    assert_eq!(reader.channels, 1);
    assert_eq!(reader.left_channel.len(), reader.right_channel.len());
    // For mono, left and right should be identical
    assert_eq!(reader.left_channel, reader.right_channel);
}

#[test]
fn test_white_noise_is_not_silent() {
    let noise = generate_white_noise(0.5, 44100, 0.3);

    // Should have reasonable length
    assert_eq!(noise.len(), 22050); // 0.5s * 44100

    // Calculate RMS (root mean square) energy
    let rms: f32 = (noise.iter().map(|s| s * s).sum::<f32>() / noise.len() as f32).sqrt();

    // RMS should be significant (not silence)
    assert!(rms > 0.05, "Noise should have energy, RMS: {}", rms);

    // Should have both positive and negative values
    let positive_count = noise.iter().filter(|&&s| s > 0.0f32).count();
    let negative_count = noise.iter().filter(|&&s| s < 0.0f32).count();

    assert!(
        positive_count > 1000,
        "Should have many positive samples: {}",
        positive_count
    );
    assert!(
        negative_count > 1000,
        "Should have many negative samples: {}",
        negative_count
    );
}

#[test]
fn test_empty_audio_handling() {
    let decoder = SstvDecoder::new();
    let params = DecoderParams::default();

    // Empty input
    let empty: Vec<f32> = vec![];
    let result = decoder.decode(&empty, &params, 44100);

    // Should return error for empty input
    assert!(result.is_err(), "Empty input should return error");

    // Find sync in empty - should return empty
    let positions = decoder.find_sync_positions(&empty, 44100);
    assert!(positions.is_empty());
}

#[test]
fn test_very_short_audio() {
    let decoder = SstvDecoder::new();

    // Just 100 samples (too short for meaningful decode)
    let short_signal: Vec<f32> = (0..100).map(|_| 0.5).collect();

    let positions = decoder.find_sync_positions(&short_signal, 44100);
    // May or may not find anything, but shouldn't crash
    assert!(positions.len() < 10, "Shouldn't find many syncs in noise");
}

#[test]
fn test_parameter_variation_affects_output() {
    let decoder = SstvDecoder::new();
    let signal = generate_square_wave(100.0, 0.3, 44100, 0.8);

    // Decode with different line durations
    let params_short = DecoderParams {
        line_duration_ms: 5.0,
        threshold: 0.3,
    };
    let params_long = DecoderParams {
        line_duration_ms: 15.0,
        threshold: 0.3,
    };

    let pixels_short = decoder
        .decode(&signal, &params_short, 44100)
        .expect("Short duration decode should succeed");
    let pixels_long = decoder
        .decode(&signal, &params_long, 44100)
        .expect("Long duration decode should succeed");

    // Different line durations should produce different number of lines
    let lines_short = pixels_short.len() / 512;
    let lines_long = pixels_long.len() / 512;

    assert_ne!(
        lines_short, lines_long,
        "Different line durations should produce different outputs"
    );
    assert!(
        lines_short > lines_long,
        "Shorter line duration should produce more lines"
    );
}

#[test]
fn test_error_messages_are_helpful() {
    let errors = [
        AudioError::NoDevice,
        AudioError::DeviceDisconnected,
        AudioError::FormatUnsupported,
        AudioError::BufferUnderrun,
        AudioError::SinkCreationFailed,
        AudioError::StreamInitFailed,
    ];

    for error in errors {
        let message = error.to_string();
        let action = error.user_action();

        // Messages should be non-empty and helpful
        assert!(!message.is_empty(), "Error message should not be empty");
        assert!(!action.is_empty(), "User action should not be empty");
        assert!(
            message.len() > 10,
            "Error message should be descriptive: {}",
            message
        );
        assert!(
            action.len() > 10,
            "User action should be descriptive: {}",
            action
        );
    }
}

#[test]
fn test_composite_signal_structure() {
    let composite = generate_composite_signal(44100);

    // Should be longer than 1 second
    assert!(
        composite.len() > 44100,
        "Composite should be > 1 second: {} samples",
        composite.len()
    );

    // Should not be all zeros
    let non_zero_count = composite.iter().filter(|&&s| s.abs() > 0.01).count();
    assert!(
        non_zero_count > 10000,
        "Composite should have significant non-zero content"
    );

    // Should have both positive and negative samples
    let pos_count = composite.iter().filter(|&&s| s > 0.1f32).count();
    let neg_count = composite.iter().filter(|&&s| s < -0.1f32).count();

    assert!(pos_count > 1000, "Should have positive samples");
    assert!(neg_count > 1000, "Should have negative samples");
}
