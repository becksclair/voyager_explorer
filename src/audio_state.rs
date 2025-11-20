//! Audio playback state management
//!
//! Provides explicit state tracking, error handling, and metrics for audio playback.

use std::fmt;
use std::time::{Duration, Instant};

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
            Self::DeviceDisconnected
                | Self::BufferUnderrun
                | Self::SinkCreationFailed
                | Self::SinkNotAvailable
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

/// Metrics for audio playback monitoring
#[derive(Debug, Clone)]
pub struct AudioMetrics {
    /// Total time spent playing audio
    pub total_playback_time: Duration,
    /// Number of seeks performed
    pub seek_count: u32,
    /// Number of buffer underruns detected
    pub buffer_underruns: u32,
    /// Number of device errors encountered
    pub device_errors: u32,
    /// Last known audio device name
    pub last_device_name: String,
    /// Timestamp of last state change
    pub last_state_change: Option<Instant>,
    /// Number of successful play operations
    pub play_count: u32,
    /// Number of pause operations
    pub pause_count: u32,
    /// Number of stop operations
    pub stop_count: u32,
}

impl Default for AudioMetrics {
    fn default() -> Self {
        Self {
            total_playback_time: Duration::ZERO,
            seek_count: 0,
            buffer_underruns: 0,
            device_errors: 0,
            last_device_name: "Unknown".to_string(),
            last_state_change: None,
            play_count: 0,
            pause_count: 0,
            stop_count: 0,
        }
    }
}

impl AudioMetrics {
    /// Record a seek operation
    pub fn record_seek(&mut self) {
        self.seek_count += 1;
    }

    /// Record a play operation
    pub fn record_play(&mut self) {
        self.play_count += 1;
        self.last_state_change = Some(Instant::now());
    }

    /// Record a pause operation
    pub fn record_pause(&mut self) {
        self.pause_count += 1;
        self.last_state_change = Some(Instant::now());
    }

    /// Record a stop operation
    pub fn record_stop(&mut self) {
        self.stop_count += 1;
        self.last_state_change = Some(Instant::now());
    }

    /// Record a buffer underrun
    pub fn record_buffer_underrun(&mut self) {
        self.buffer_underruns += 1;
    }

    /// Record a device error
    pub fn record_device_error(&mut self) {
        self.device_errors += 1;
    }

    /// Add to total playback time
    pub fn add_playback_time(&mut self, duration: Duration) {
        self.total_playback_time += duration;
    }

    /// Generate a summary string for debugging
    pub fn summary(&self) -> String {
        format!(
            "Audio Metrics: plays={}, pauses={}, stops={}, seeks={}, playback_time={:.1}s, errors={}",
            self.play_count,
            self.pause_count,
            self.stop_count,
            self.seek_count,
            self.total_playback_time.as_secs_f32(),
            self.device_errors + self.buffer_underruns
        )
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
    fn test_metrics_recording() {
        let mut metrics = AudioMetrics::default();
        assert_eq!(metrics.seek_count, 0);

        metrics.record_seek();
        metrics.record_seek();
        assert_eq!(metrics.seek_count, 2);

        metrics.record_play();
        assert_eq!(metrics.play_count, 1);
        assert!(metrics.last_state_change.is_some());
    }

    #[test]
    fn test_status_icons() {
        assert_eq!(AudioPlaybackState::Playing.status_icon(), "▶️");
        assert_eq!(AudioPlaybackState::Paused.status_icon(), "⏸️");
        assert_eq!(
            AudioPlaybackState::Error(AudioError::NoDevice).status_icon(),
            "⚠️"
        );
    }
}
