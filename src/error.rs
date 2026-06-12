//! Comprehensive error types for the Voyager Explorer application.
//!
//! This module provides structured error handling with context chains,
//! user-friendly messages, and recovery strategies.

use std::path::PathBuf;

use thiserror::Error;

/// Top-level error type for all Voyager Explorer operations.
#[derive(Error, Debug)]
pub enum VoyagerError {
    /// Audio file loading or processing errors
    #[error("Audio error: {0}")]
    Audio(#[from] AudioError),

    /// SSTV decoding errors
    #[error("Decoder error: {0}")]
    Decoder(#[from] DecoderError),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Audio-related errors
#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Failed to load WAV file '{path}': {source}")]
    LoadFailed { path: PathBuf, source: hound::Error },

    #[error("Invalid sample rate: {rate} Hz (must be at least 8 kHz)")]
    InvalidSampleRate { rate: u32 },

    #[error("Seek offset {start_secs:.1}s exceeds the seekable range for this file")]
    SeekOutOfRange { start_secs: f64 },

    #[error("Unsupported channel count: {channels} (only mono/stereo supported)")]
    UnsupportedChannels { channels: u16 },

    #[error("Empty audio file: {path}")]
    EmptyFile { path: PathBuf },

    #[error("Audio buffer too short: needed {needed} samples, got {actual}")]
    BufferTooShort { needed: usize, actual: usize },

    #[error("Audio playback initialization failed: {reason}")]
    PlaybackInitFailed { reason: String },

    #[error("Audio stream error: {0}")]
    StreamError(String),
}

/// SSTV decoder errors
#[derive(Error, Debug)]
pub enum DecoderError {
    #[error("Invalid decoder parameters: {reason}")]
    InvalidParams { reason: String },

    #[error("Line duration out of range: {duration_ms}ms (must be 1-100ms)")]
    InvalidLineDuration { duration_ms: f32 },

    #[error("Insufficient samples for decoding: needed {needed}, got {actual}")]
    InsufficientSamples { needed: usize, actual: usize },
}

/// Configuration errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config file '{path}': {source}")]
    LoadFailed { path: Box<PathBuf>, source: std::io::Error },

    #[error("Invalid config format in '{path}': {source}")]
    InvalidFormat { path: Box<PathBuf>, source: toml::de::Error },

    #[error("Config validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("Failed to save config to '{path}': {source}")]
    SaveFailed { path: Box<PathBuf>, source: std::io::Error },

    #[error("Config serialization failed: {source}")]
    SerializationFailed { source: toml::ser::Error },
}

/// Result type alias for Voyager operations
pub type Result<T, E = VoyagerError> = std::result::Result<T, E>;

impl AudioError {
    /// Check if error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            AudioError::BufferTooShort { .. } | AudioError::StreamError(_) | AudioError::PlaybackInitFailed { .. }
        )
    }

    /// Get user-friendly message
    pub fn user_message(&self) -> String {
        match self {
            AudioError::LoadFailed { path, .. } => {
                format!("Could not open audio file '{}'", path.display())
            }
            AudioError::InvalidSampleRate { rate } => {
                format!("Audio file has unsupported sample rate: {} Hz", rate)
            }
            AudioError::SeekOutOfRange { start_secs } => {
                format!("Start offset {start_secs:.1}s is beyond the seekable range of this file")
            }
            AudioError::UnsupportedChannels { channels } => {
                format!("Audio file has unsupported {} channels", channels)
            }
            AudioError::EmptyFile { path } => {
                format!("Audio file '{}' is empty", path.display())
            }
            AudioError::BufferTooShort { .. } => "Audio segment is too short for decoding".to_string(),
            AudioError::PlaybackInitFailed { .. } => "Could not initialize audio playback device".to_string(),
            AudioError::StreamError(_) => "Audio playback error occurred".to_string(),
        }
    }
}

impl DecoderError {
    /// Get suggested recovery action
    pub fn recovery_hint(&self) -> Option<&str> {
        match self {
            DecoderError::InvalidLineDuration { .. } => Some("Try adjusting line duration between 1-100ms"),
            DecoderError::InsufficientSamples { .. } => Some("Load a longer audio file or adjust decode window"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AudioError::InvalidSampleRate { rate: 999 };
        assert!(err.to_string().contains("999"));
        assert!(err.to_string().contains("8 kHz"));
    }

    #[test]
    fn test_recoverable_errors() {
        let recoverable = AudioError::BufferTooShort { needed: 100, actual: 50 };
        assert!(recoverable.is_recoverable());

        let unrecoverable = AudioError::InvalidSampleRate { rate: 999 };
        assert!(!unrecoverable.is_recoverable());
    }

    #[test]
    fn test_user_messages() {
        let err = AudioError::InvalidSampleRate { rate: 1000 };
        let msg = err.user_message();
        assert!(msg.contains("1000"));
        assert!(!msg.contains("Error")); // User-friendly, not technical
    }

    #[test]
    fn test_recovery_hints() {
        let err = DecoderError::InvalidLineDuration { duration_ms: 0.5 };
        assert!(err.recovery_hint().is_some());
        assert!(err.recovery_hint().unwrap().contains("1-100ms"));
    }
}
