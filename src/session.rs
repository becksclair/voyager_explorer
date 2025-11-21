//! Session state persistence for saving and loading application state.
//!
//! This module provides serialization/deserialization support for session state,
//! allowing users to save their current decoder configuration, playback position,
//! and loaded file for later restoration.

use crate::audio::WaveformChannel;
use crate::sstv::{DecoderMode, DecoderParams};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Serializable application session state.
///
/// This captures the essential state needed to restore a user's work session,
/// including which file was loaded, playback position, decoder settings, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Path to the loaded WAV file (if any)
    pub wav_path: Option<PathBuf>,

    /// Current playback position in samples
    pub current_position_samples: usize,

    /// Selected audio channel (Left or Right)
    pub selected_channel: WaveformChannel,

    /// Decoder parameters
    pub line_duration_ms: f32,
    pub threshold: f32,
    pub decode_window_secs: f64,
    pub mode: DecoderMode,

    /// Optional preset name (if params match a preset)
    pub current_preset: Option<String>,
}

impl SessionState {
    /// Create a new session state from current application state.
    pub fn from_app(
        wav_path: Option<PathBuf>,
        current_position_samples: usize,
        selected_channel: WaveformChannel,
        params: &DecoderParams,
        current_preset: Option<&str>,
    ) -> Self {
        Self {
            wav_path,
            current_position_samples,
            selected_channel,
            line_duration_ms: params.line_duration_ms,
            threshold: params.threshold,
            decode_window_secs: params.decode_window_secs,
            mode: params.mode,
            current_preset: current_preset.map(|s| s.to_string()),
        }
    }

    /// Convert to DecoderParams
    pub fn to_params(&self) -> DecoderParams {
        DecoderParams {
            line_duration_ms: self.line_duration_ms,
            threshold: self.threshold,
            decode_window_secs: self.decode_window_secs,
            mode: self.mode,
        }
    }

    /// Save session state to a JSON file.
    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        tracing::info!(path = %path.display(), "Session saved successfully");
        Ok(())
    }

    /// Load session state from a JSON file.
    pub fn load_from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state = serde_json::from_str(&json)?;
        tracing::info!(path = %path.display(), "Session loaded successfully");
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_session_serialization_roundtrip() {
        let original = SessionState {
            wav_path: Some(PathBuf::from("/path/to/audio.wav")),
            current_position_samples: 12345,
            selected_channel: WaveformChannel::Left,
            line_duration_ms: 8.3,
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
            current_preset: Some("Voyager Default".to_string()),
        };

        // Serialize to JSON
        let json = serde_json::to_string(&original).unwrap();

        // Deserialize back
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        // Verify fields match
        assert_eq!(original.wav_path, restored.wav_path);
        assert_eq!(
            original.current_position_samples,
            restored.current_position_samples
        );
        assert_eq!(original.selected_channel, restored.selected_channel);
        assert_eq!(original.line_duration_ms, restored.line_duration_ms);
        assert_eq!(original.threshold, restored.threshold);
        assert_eq!(original.decode_window_secs, restored.decode_window_secs);
        assert_eq!(original.mode, restored.mode);
        assert_eq!(original.current_preset, restored.current_preset);
    }

    #[test]
    fn test_save_and_load_from_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let original = SessionState {
            wav_path: Some(PathBuf::from("/test/file.wav")),
            current_position_samples: 5000,
            selected_channel: WaveformChannel::Right,
            line_duration_ms: 10.0,
            threshold: 0.3,
            decode_window_secs: 3.0,
            mode: DecoderMode::PseudoColor,
            current_preset: None,
        };

        // Save to file
        original.save_to_file(path).unwrap();

        // Load from file
        let loaded = SessionState::load_from_file(path).unwrap();

        // Verify
        assert_eq!(original.wav_path, loaded.wav_path);
        assert_eq!(
            original.current_position_samples,
            loaded.current_position_samples
        );
        assert_eq!(original.mode, loaded.mode);
    }

    #[test]
    fn test_to_params_conversion() {
        let session = SessionState {
            wav_path: None,
            current_position_samples: 0,
            selected_channel: WaveformChannel::Left,
            line_duration_ms: 12.5,
            threshold: 0.15,
            decode_window_secs: 2.5,
            mode: DecoderMode::PseudoColor,
            current_preset: Some("Color".to_string()),
        };

        let params = session.to_params();

        assert_eq!(params.line_duration_ms, 12.5);
        assert_eq!(params.threshold, 0.15);
        assert_eq!(params.decode_window_secs, 2.5);
        assert_eq!(params.mode, DecoderMode::PseudoColor);
    }

    #[test]
    fn test_from_app_constructor() {
        let params = DecoderParams {
            line_duration_ms: 8.3,
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        };

        let session = SessionState::from_app(
            Some(PathBuf::from("/test.wav")),
            1000,
            WaveformChannel::Left,
            &params,
            Some("Test Preset"),
        );

        assert_eq!(session.wav_path, Some(PathBuf::from("/test.wav")));
        assert_eq!(session.current_position_samples, 1000);
        assert_eq!(session.selected_channel, WaveformChannel::Left);
        assert_eq!(session.line_duration_ms, 8.3);
        assert_eq!(session.current_preset, Some("Test Preset".to_string()));
    }
}
