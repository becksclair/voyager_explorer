//! Audio playback state management
//!
//! Provides explicit state tracking and error handling for audio playback.

use std::fmt;

/// Explicit audio playback state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPlaybackState {
    /// No audio device available or no WAV loaded
    Uninitialized,
    /// Audio device ready, WAV loaded, not playing
    Ready,
    /// Active playback in progress
    Playing,
    /// Playback paused, position retained
    Paused,
    /// Error state with specific error type
    Error(AudioError),
}

impl fmt::Display for AudioPlaybackState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uninitialized => write!(f, "Uninitialized"),
            Self::Ready => write!(f, "Ready"),
            Self::Playing => write!(f, "Playing"),
            Self::Paused => write!(f, "Paused"),
            Self::Error(e) => write!(f, "Error: {}", e),
        }
    }
}

impl AudioPlaybackState {
    /// Returns true if audio is actively playing
    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing)
    }

    /// Returns true if audio can be started
    pub fn can_play(&self) -> bool {
        matches!(self, Self::Ready | Self::Paused)
    }

    /// Returns true if in an error state
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Returns the error if in error state
    pub fn error(&self) -> Option<AudioError> {
        if let Self::Error(e) = self {
            Some(*e)
        } else {
            None
        }
    }

    /// Get a user-friendly status icon
    pub fn status_icon(&self) -> &'static str {
        match self {
            Self::Uninitialized => "⚪",
            Self::Ready => "🔊",
            Self::Playing => "▶️",
            Self::Paused => "⏸️",
            Self::Error(_) => "⚠️",
        }
    }

    /// Get a user-friendly status message
    pub fn status_message(&self) -> String {
        match self {
            Self::Uninitialized => "No audio".to_string(),
            Self::Ready => "Audio ready".to_string(),
            Self::Playing => "Playing".to_string(),
            Self::Paused => "Paused".to_string(),
            Self::Error(e) => format!("Audio error: {}", e),
        }
    }
}

/// Specific audio error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioError {
    /// No audio output device detected
    NoDevice,
    /// Audio device was disconnected during playback
    DeviceDisconnected,
    /// Audio format is not supported by the device
    FormatUnsupported,
    /// Buffer underrun occurred (audio stuttering)
    BufferUnderrun,
    /// Failed to create audio sink
    SinkCreationFailed,
    /// Failed to initialize audio stream
    StreamInitFailed,
    /// Audio sink not available or lost
    SinkNotAvailable,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => write!(f, "No audio device available"),
            Self::DeviceDisconnected => write!(f, "Audio device disconnected"),
            Self::FormatUnsupported => write!(f, "Audio format not supported"),
            Self::BufferUnderrun => write!(f, "Audio buffer underrun"),
            Self::SinkCreationFailed => write!(f, "Failed to create audio sink"),
            Self::StreamInitFailed => write!(f, "Failed to initialize audio stream"),
            Self::SinkNotAvailable => write!(f, "Audio sink not available"),
        }
    }
}

impl AudioError {
    /// Returns true if the error is recoverable (user can retry)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::DeviceDisconnected | Self::BufferUnderrun | Self::SinkCreationFailed | Self::SinkNotAvailable
        )
    }

    /// Get suggested user action
    pub fn user_action(&self) -> &'static str {
        match self {
            Self::NoDevice => "Check audio device connection",
            Self::DeviceDisconnected => "Reconnect audio device and retry",
            Self::FormatUnsupported => "Audio format incompatible with device",
            Self::BufferUnderrun => "Playback may stutter; try reducing system load",
            Self::SinkCreationFailed => "Retry playback",
            Self::StreamInitFailed => "Restart application or check audio settings",
            Self::SinkNotAvailable => "Retry playback or restart application",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_predicates() {
        assert!(AudioPlaybackState::Playing.is_playing());
        assert!(!AudioPlaybackState::Ready.is_playing());

        assert!(AudioPlaybackState::Ready.can_play());
        assert!(AudioPlaybackState::Paused.can_play());
        assert!(!AudioPlaybackState::Playing.can_play());

        assert!(AudioPlaybackState::Error(AudioError::NoDevice).is_error());
        assert!(!AudioPlaybackState::Ready.is_error());
    }

    #[test]
    fn test_error_recovery() {
        assert!(AudioError::DeviceDisconnected.is_recoverable());
        assert!(!AudioError::NoDevice.is_recoverable());
        assert!(!AudioError::FormatUnsupported.is_recoverable());
    }

    #[test]
    fn test_state_display() {
        assert_eq!(AudioPlaybackState::Ready.to_string(), "Ready");
        assert_eq!(
            AudioPlaybackState::Error(AudioError::NoDevice).to_string(),
            "Error: No audio device available"
        );
    }

    #[test]
    fn test_status_icons() {
        assert_eq!(AudioPlaybackState::Playing.status_icon(), "▶️");
        assert_eq!(AudioPlaybackState::Paused.status_icon(), "⏸️");
        assert_eq!(AudioPlaybackState::Error(AudioError::NoDevice).status_icon(), "⚠️");
    }
}
