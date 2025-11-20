# Audio Playback System Design Document

## Overview

Real-time audio playback system for Voyager Golden Record Explorer with seamless seeking, robust error handling, and cross-platform support.

## Architecture

### State Machine

```text
┌─────────────────────────────────────────────────────────────────┐
│                     Audio Playback State Machine                │
└─────────────────────────────────────────────────────────────────┘

States:
  • Uninitialized: No audio device, no WAV loaded
  • Ready: Audio device available, WAV loaded, not playing
  • Playing: Active playback with position advancement
  • Paused: Playback suspended, position retained
  • Error: Audio device failure or other error state

┌──────────────┐
│Uninitialized │
└──────┬───────┘
       │ load_wav()
       ↓
┌──────────────┐     toggle_play()      ┌─────────┐
│    Ready     │───────────────────────→│ Playing │
└──────┬───────┘                        └────┬────┘
       ↑                                     │
       │                                     │ toggle_play()
       │          ┌─────────┐                │
       └──────────┤ Paused  │←───────────────┘
       │          └────┬────┘
       │               │
       │ stop()        │ stop()
       ↓               ↓
┌──────────────┐     seek()
│    Ready     │───────────────────→ (restart Playing if was playing)
└──────────────┘

       any state
           │ device_error()
           ↓
      ┌────────┐
      │ Error  │──→ (UI shows error, attempts recovery)
      └────────┘

Transitions:
  1. Uninitialized → Ready: load_wav() succeeds + audio device available
  2. Ready → Playing: toggle_play() when ready
  3. Playing → Paused: toggle_play() during playback
  4. Paused → Playing: toggle_play() when paused
  5. Playing/Paused → Ready: stop()
  6. Ready → Playing (seek): seek() restarts if was playing
  7. Any → Error: Device failure, format incompatibility
  8. Error → Ready: User intervention (device reconnect, file reload)
```

## Components

### AudioPlaybackState

Explicit state tracking (not implicit via Option checks):

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioPlaybackState {
    Uninitialized,
    Ready,
    Playing,
    Paused,
    Error(AudioError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioError {
    NoDevice,
    DeviceDisconnected,
    FormatUnsupported,
    BufferUnderrun,
}
```

### AudioPlaybackSystem

Encapsulates all audio logic:

```rust
pub struct AudioPlaybackSystem {
    state: AudioPlaybackState,
    stream: Option<(OutputStream, OutputStreamHandle)>,
    sink: Option<Sink>,
    buffer_source: Option<Arc<[f32]>>, // Zero-copy sharing
    position_samples: usize,
    sample_rate: u32,
    last_error: Option<String>,
}
```

## Key Design Decisions

### 1. **Explicit State Tracking**

- Replace implicit state (checking Option<`Sink`>) with explicit AudioPlaybackState enum
- UI can query state directly: `system.state()`
- Clear error reporting: state includes error variant

### 2. **Zero-Copy Buffer Management**

- Use `Arc<[f32]>` instead of `Vec<f32>` clones on every seek
- AudioBufferSource holds Arc and offset, no cloning
- Reduces memory pressure for large files

### 3. **Graceful Degradation**

- If audio device unavailable: visual-only mode
- If device disconnects: automatic fallback, user notification
- Format incompatibility: clear error message

### 4. **User Feedback for All Errors**

- Status bar indicator: 🔊 Audio Ready | ⚠️ Audio Unavailable | ⏸️ Paused
- Toast notifications for transient errors
- Detailed error in debug panel

### 5. **Testability**

- AudioPlaybackSystem is fully testable via state queries
- Mock audio devices for CI/CD environments
- Synthetic audio fixtures with known properties (440Hz tone, chirp)

## Error Handling Matrix

| Error Condition | Detection | User Feedback | Recovery |
|----------------|-----------|---------------|----------|
| No audio device | On startup | Status: ⚠️ Audio Unavailable | Visual mode auto-enabled |
| Device disconnect | During playback | Toast: "Audio device disconnected" | Pause, await reconnect |
| Format unsupported | On play | Toast: "Audio format incompatible" | Visual mode fallback |
| Buffer underrun | During playback | Log warning (not user-facing) | Continue, monitor |
| Large file OOM | On buffer create | Toast: "File too large for audio" | Visual mode only |

## Performance Considerations

### Memory Usage
- **Before**: `samples[pos..].to_vec()` = O(n) copy on every seek
- **After**: `Arc::clone(&samples)` + offset = O(1) on every seek

### CPU Usage
- Background decoding (Milestone 2) won't block audio thread
- Position tracking uses wall-clock time, not polling

### Latency
- Seek latency: < 50ms (stop old sink + start new)
- Device initialization: lazy (only when first needed)

## Cross-Platform Support

### Platform-Specific Backends (via cpal/rodio)

| Platform | Backend | Notes |
|----------|---------|-------|
| Linux | ALSA | Default, requires libasound2-dev |
| Linux | PulseAudio | Alternative, more common on desktop |
| Linux | JACK | Pro audio, low latency |
| Windows | WASAPI | Native, no extra deps |
| macOS | CoreAudio | Native, no extra deps |

### Testing Strategy

**Synthetic Audio Fixtures**:
```rust
// 440Hz tone (A4) for 1 second - easily recognizable
fn generate_test_tone() -> Vec<f32> { ... }

// Chirp from 200Hz to 2000Hz - verify frequency response
fn generate_chirp() -> Vec<f32> { ... }

// Sync + silence + sync pattern - test seeking
fn generate_sync_pattern() -> Vec<f32> { ... }
```

**CI/CD Strategy**:

- GitHub Actions: Build with `--no-default-features` (no audio dependencies for sandboxed environments)
- Local dev: Build with default features (audio_playback enabled automatically)
- Integration tests: Use synthetic fixtures, verify state transitions

## Observability

### Metrics to Track

```rust
pub struct AudioMetrics {
    pub total_playback_time: Duration,
    pub seek_count: u32,
    pub buffer_underruns: u32,
    pub device_errors: u32,
    pub last_device_name: String,
}
```

### Logging Strategy

```rust
// Structured logging for debugging
log::debug!(
    target: "audio_playback",
    "State transition: {:?} -> {:?}, position: {}",
    old_state, new_state, position_samples
);

// Performance tracking
log::info!(
    target: "audio_perf",
    "Seek latency: {:?}, buffer_size: {}",
    seek_duration, buffer_len
);
```

## Future Enhancements

1. **Volume Control**: rodio Sink supports `set_volume(0.0..=1.0)`
2. **Device Selection**: Query available devices, let user choose
3. **Drift Correction**: Monitor actual vs expected position, adjust
4. **Buffering Strategy**: Streaming for gigabyte files
5. **Position Callback**: rodio doesn't expose position - track via elapsed time

## Implementation Checklist

- [ ] Define AudioPlaybackState enum
- [ ] Refactor AudioBufferSource to use Arc<[f32]>
- [ ] Add status bar indicator widget
- [ ] Implement toast notifications for errors
- [ ] Create synthetic audio test fixtures
- [ ] Add AudioMetrics tracking
- [ ] Add structured logging
- [ ] Test state machine transitions
- [ ] Test cross-platform (Windows/Mac/Linux)
- [ ] Document user-facing features

## References

- [rodio documentation](https://docs.rs/rodio/latest/rodio/)
- [cpal (Cross-Platform Audio Library)](https://docs.rs/cpal/latest/cpal/)
- [egui immediate mode GUI patterns](https://docs.rs/egui/latest/egui/)
