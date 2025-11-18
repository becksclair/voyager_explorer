//! Application configuration system with TOML persistence.
//!
//! Supports loading from file, environment variables, and sensible defaults.

use crate::error::{ConfigError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level application configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    /// Decoder configuration
    pub decoder: DecoderConfig,

    /// UI configuration
    pub ui: UiConfig,

    /// Audio configuration
    #[cfg(feature = "audio_playback")]
    pub audio: AudioConfig,

    /// Worker thread configuration
    pub worker: WorkerConfig,

    /// Metrics configuration
    pub metrics: MetricsConfig,
}

/// SSTV decoder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoderConfig {
    /// Default line duration in milliseconds (1.0-100.0)
    pub default_line_duration_ms: f32,

    /// Default amplitude threshold (0.0-1.0)
    pub default_threshold: f32,

    /// Decode window duration in seconds
    pub decode_window_secs: f32,

    /// FFT chunk size (must be power of 2)
    pub fft_chunk_size: usize,

    /// Sync detection threshold multiplier
    pub sync_threshold_multiplier: f32,

    /// Target sync frequency in Hz
    pub target_sync_freq_hz: f32,
}

/// UI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// Fixed image width in pixels
    pub image_width: usize,

    /// Maximum image height in pixels (GPU limit)
    pub max_image_height: usize,

    /// Waveform display height in pixels
    pub waveform_height: f32,

    /// Enable debug panel
    pub show_debug_panel: bool,

    /// Frame rate target
    pub target_fps: u32,
}

/// Audio playback configuration
#[cfg(feature = "audio_playback")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Audio buffer size in samples
    pub buffer_size: usize,

    /// Enable audio playback by default
    pub playback_enabled: bool,

    /// Default playback volume (0.0-1.0)
    pub default_volume: f32,
}

/// Worker thread configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Maximum queue size for pending decode requests
    pub max_queue_size: usize,

    /// Health check interval in milliseconds
    pub health_check_interval_ms: u64,

    /// Maximum time without response before restart (ms)
    pub max_unresponsive_ms: u64,

    /// Enable worker auto-restart on panic
    pub auto_restart_on_panic: bool,
}

/// Metrics configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Enable metrics collection
    pub enabled: bool,

    /// Enable metrics UI panel
    pub show_metrics_panel: bool,

    /// Histogram precision (significant value digits)
    pub histogram_precision: u8,

    /// Maximum histogram value in milliseconds
    pub histogram_max_ms: u64,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            default_line_duration_ms: 8.3,
            default_threshold: 0.2,
            decode_window_secs: 2.0,
            fft_chunk_size: 2048,
            sync_threshold_multiplier: 10.0,
            target_sync_freq_hz: 1200.0,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            image_width: 512,
            max_image_height: 16384,
            waveform_height: 200.0,
            show_debug_panel: cfg!(debug_assertions),
            target_fps: 60,
        }
    }
}

#[cfg(feature = "audio_playback")]
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            buffer_size: 4096,
            playback_enabled: true,
            default_volume: 0.5,
        }
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 10,
            health_check_interval_ms: 1000,
            max_unresponsive_ms: 5000,
            auto_restart_on_panic: true,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_metrics_panel: cfg!(debug_assertions),
            histogram_precision: 2,
            histogram_max_ms: 10_000,
        }
    }
}

impl AppConfig {
    /// Load configuration from TOML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::LoadFailed {
            path: path.to_path_buf(),
            source,
        })?;

        toml::from_str(&contents).map_err(|source| ConfigError::InvalidFormat {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Load configuration with fallback to defaults
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        Self::load_from_file(path).unwrap_or_else(|e| {
            tracing::warn!("Failed to load config, using defaults: {}", e);
            Self::default()
        })
    }

    /// Save configuration to TOML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let path = path.as_ref();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::SaveFailed {
                path: path.to_path_buf(),
                source,
            })?;
        }

        let contents =
            toml::to_string_pretty(self).expect("Config serialization should never fail");

        std::fs::write(path, contents).map_err(|source| ConfigError::SaveFailed {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Get default config file path
    pub fn default_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("voyager-explorer");

        config_dir.join("config.toml")
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate decoder config
        if !(1.0..=100.0).contains(&self.decoder.default_line_duration_ms) {
            return Err(ConfigError::ValidationFailed {
                reason: format!(
                    "Line duration {}ms out of range 1-100ms",
                    self.decoder.default_line_duration_ms
                ),
            });
        }

        if !(0.0..=1.0).contains(&self.decoder.default_threshold) {
            return Err(ConfigError::ValidationFailed {
                reason: format!(
                    "Threshold {} out of range 0.0-1.0",
                    self.decoder.default_threshold
                ),
            });
        }

        if !self.decoder.fft_chunk_size.is_power_of_two() {
            return Err(ConfigError::ValidationFailed {
                reason: format!(
                    "FFT chunk size {} must be power of 2",
                    self.decoder.fft_chunk_size
                ),
            });
        }

        // Validate UI config
        if self.ui.image_width == 0 {
            return Err(ConfigError::ValidationFailed {
                reason: "Image width must be > 0".to_string(),
            });
        }

        // Validate worker config
        if self.worker.max_queue_size == 0 {
            return Err(ConfigError::ValidationFailed {
                reason: "Worker queue size must be > 0".to_string(),
            });
        }

        Ok(())
    }
}

// Helper for getting config dir (add dirs crate dependency if needed)
mod dirs {
    use std::path::PathBuf;

    pub fn config_dir() -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".config"))
                })
        }

        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("Library/Application Support"))
        }

        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA").ok().map(PathBuf::from)
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_valid() {
        let config = AppConfig::default();
        config.validate().expect("Default config should be valid");
    }

    #[test]
    fn test_config_serialization() {
        let config = AppConfig::default();
        let toml_str = toml::to_string(&config).expect("Should serialize");
        let _deserialized: AppConfig = toml::from_str(&toml_str).expect("Should deserialize");
    }

    #[test]
    fn test_validation_line_duration() {
        let mut config = AppConfig::default();
        config.decoder.default_line_duration_ms = 0.5; // Too low
        assert!(config.validate().is_err());

        config.decoder.default_line_duration_ms = 150.0; // Too high
        assert!(config.validate().is_err());

        config.decoder.default_line_duration_ms = 8.3; // Valid
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validation_threshold() {
        let mut config = AppConfig::default();
        config.decoder.default_threshold = -0.1; // Too low
        assert!(config.validate().is_err());

        config.decoder.default_threshold = 1.5; // Too high
        assert!(config.validate().is_err());

        config.decoder.default_threshold = 0.5; // Valid
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validation_fft_chunk_size() {
        let mut config = AppConfig::default();
        config.decoder.fft_chunk_size = 2000; // Not power of 2
        assert!(config.validate().is_err());

        config.decoder.fft_chunk_size = 2048; // Power of 2
        assert!(config.validate().is_ok());
    }
}
