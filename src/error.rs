//! Comprehensive error types for the Voyager Explorer application.
//!
//! This module provides structured error handling with context chains,
//! user-friendly messages, and recovery strategies.

use std::path::PathBuf;

use thiserror::Error;

/// Top-level error type for all Voyager Explorer operations.
///
/// Each variant is transparent: it displays as the error it wraps and adds
/// no link of its own to the source chain. Printing the wrapped error here
/// as well would repeat the cause wherever a whole chain is shown (`{:#}`).
#[derive(Error, Debug)]
pub enum VoyagerError {
    /// Audio file loading or processing errors
    #[error(transparent)]
    Audio(#[from] AudioError),

    /// SSTV decoding errors
    #[error(transparent)]
    Decoder(#[from] DecoderError),

    /// Configuration errors
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// I/O errors
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Audio-related errors
#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Failed to load WAV file '{path}'")]
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
    #[error("Failed to load config file '{path}'")]
    LoadFailed { path: Box<PathBuf>, source: std::io::Error },

    #[error("Invalid config format in '{path}'")]
    InvalidFormat { path: Box<PathBuf>, source: toml::de::Error },

    #[error("Config validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("Failed to save config to '{path}'")]
    SaveFailed { path: Box<PathBuf>, source: std::io::Error },

    #[error("Config serialization failed")]
    SerializationFailed { source: toml::ser::Error },
}

/// Render an error followed by its causes, outermost first, the way `{:#}`
/// prints an `anyhow::Error`. For the error types here, which are logged
/// and shown without being converted to one.
///
/// A message never repeats its own cause: variants with a `source` field
/// leave it to the chain, so the cause is printed exactly once.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut cause = err.source();
    while let Some(err) = cause {
        out.push_str(": ");
        out.push_str(&err.to_string());
        cause = err.source();
    }
    out
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
    fn cause_appears_once_in_a_printed_chain() {
        let err = VoyagerError::from(DecoderError::InsufficientSamples { needed: 400, actual: 1 });
        let chain = format!("{:#}", anyhow::Error::from(err).context("Decode failed"));
        assert_eq!(chain, "Decode failed: Insufficient samples for decoding: needed 400, got 1");
    }

    #[test]
    fn load_failure_names_its_cause_once() {
        let err = VoyagerError::from(ConfigError::LoadFailed {
            path: Box::new(PathBuf::from("settings.toml")),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
        });
        let expected = "Failed to load config file 'settings.toml': permission denied";
        assert_eq!(error_chain(&err), expected);
        assert_eq!(format!("{:#}", anyhow::Error::from(err)), expected);
    }

    #[test]
    fn test_recovery_hints() {
        let err = DecoderError::InvalidLineDuration { duration_ms: 0.5 };
        assert!(err.recovery_hint().is_some());
        assert!(err.recovery_hint().unwrap().contains("1-100ms"));
    }
}
