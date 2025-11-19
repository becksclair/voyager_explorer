// Library interface for Voyager Explorer components

pub mod audio;
pub mod audio_state;
pub mod config;
pub mod error;
pub mod image_output;
pub mod metrics;
pub mod sstv;
pub mod utils;

// Test fixtures for synthetic audio generation
pub mod test_fixtures;

// Re-export commonly used types
pub use config::AppConfig;
pub use error::{AudioError, DecoderError, Result, VoyagerError};
