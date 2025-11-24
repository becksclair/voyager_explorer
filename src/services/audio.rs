#[cfg(feature = "audio_playback")]
use std::sync::Arc;
#[cfg(feature = "audio_playback")]
use std::time::Duration;

#[cfg(feature = "audio_playback")]
use crate::error::{AudioError, Result};

#[cfg(feature = "audio_playback")]
use rodio::Source;

#[cfg(feature = "audio_playback")]
/// Audio source that plays from a shared buffer of f32 samples with zero-copy seeking.
pub struct AudioBufferSource {
    /// Shared reference to the audio buffer. Arc enables zero-copy sharing.
    buffer: Arc<[f32]>,
    /// Starting position in the buffer (sample index where playback begins).
    offset: usize,
    /// Sample rate in Hz (e.g., 44100, 48000).
    sample_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo).
    channels: u16,
    /// Current read position relative to offset.
    position: usize,
}

#[cfg(feature = "audio_playback")]
impl AudioBufferSource {
    /// Create a new AudioBufferSource with validated parameters.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidParams`] if:
    /// - `offset >= buffer.len()` (offset out of bounds)
    /// - `channels == 0` (invalid channel count)
    /// - `sample_rate == 0` (invalid sample rate)
    pub fn new(
        buffer: Arc<[f32]>,
        offset: usize,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, AudioError> {
        // Validate offset
        if offset >= buffer.len() {
            return Err(AudioError::BufferTooShort {
                needed: offset + 1,
                actual: buffer.len(),
            });
        }

        // Validate channels
        if channels == 0 {
            return Err(AudioError::UnsupportedChannels { channels });
        }

        // Validate sample rate
        if sample_rate == 0 {
            return Err(AudioError::InvalidSampleRate { rate: sample_rate });
        }

        Ok(Self {
            buffer,
            offset,
            sample_rate,
            channels,
            position: 0,
        })
    }
}

#[cfg(feature = "audio_playback")]
impl Iterator for AudioBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let absolute_position = self.offset + self.position;
        if absolute_position < self.buffer.len() {
            let sample = self.buffer[absolute_position];
            self.position += 1;
            Some(sample)
        } else {
            None
        }
    }
}

#[cfg(feature = "audio_playback")]
impl Source for AudioBufferSource {
    fn current_span_len(&self) -> Option<usize> {
        self.buffer
            .len()
            .checked_sub(self.offset)
            .map(|len| len.saturating_sub(self.position))
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let remaining_samples = self.buffer.len().checked_sub(self.offset)? as u64;
        let duration_secs =
            remaining_samples as f64 / (self.sample_rate as f64 * self.channels as f64);
        Some(Duration::from_secs_f64(duration_secs))
    }
}
