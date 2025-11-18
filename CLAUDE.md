# CLAUDE.md

This file provides comprehensive guidance to Claude Code (claude.ai/code) when working with the Voyager Golden Record Explorer codebase.

## Project Overview

Voyager Golden Record Explorer is a sophisticated Rust + Egui desktop application that provides real-time SSTV-style audio-to-image decoding with comprehensive interactive features. The application decodes analog image data from NASA's Voyager Golden Record with real-time playback, waveform visualization, sync detection, and user interaction capabilities.

### Current vs planned capabilities

**Currently implemented (as of v0.2.x):**
- WAV loading and normalization via `WavReader` with mono/stereo support.
- Visual “playback” using `is_playing`, `playback_start_time`, and `current_position_samples` (no rodio audio output is wired yet).
- Interactive waveform visualization with hover, click-to-seek, and position indicator.
- SSTV-style binary grayscale decoding (`SstvDecoder::decode`) and sync detection (`find_sync_positions`, `find_next_sync`).
- Image rendering via `image_output::image_from_pixels` into `egui::ColorImage`.
- Unit and integration tests covering the WAV → decode → image pipeline.

**Planned / in-progress (see `specs/implementation.md`):**
- Real rodio-based audio playback behind the `audio_playback` feature, connected to `VoyagerApp` playback controls.
- Moving decode work off the UI thread via a background worker and message passing.
- Color decoding modes and decoder presets for different Voyager image types.
- Session persistence (saving/loading parameters, positions, and file paths).
- Exporting decoded images via the `image` crate (for example PNG/TIFF) and optional raw pixel dumps.
- Advanced analysis tools (spectrum view, noise reduction) and batch processing utilities.

`specs/implementation.md` is the canonical implementation roadmap; keep it and this file in sync when making architectural changes.

## Development Commands

### Build and Run

```bash
cargo run                    # Run the application in debug mode
cargo build                  # Build debug version
cargo build --release        # Build optimized release version
```

### Testing and Quality

```bash
cargo test                   # Run all tests (29 total: 25 unit + 4 integration)
cargo test --lib             # Run unit tests only
cargo test --test integration_tests # Run integration tests only
cargo clippy                 # Run linter
cargo fmt                    # Format code
cargo check                  # Quick compilation check
cargo tarpaulin --out html   # Generate code coverage report
```

### Debugging and Development

```bash
RUST_LOG=debug cargo run     # Run with debug logging enabled
cargo doc --open            # Generate and open documentation
```

## Architecture Overview

The application follows a modern, modular design with clear separation of concerns and comprehensive real-time capabilities.

### Application Layout Structure

```text
┌─────────────────────────────────────────────────────────────┐
│ Top Panel: File controls, decode button, parameters         │
├─────────────────────────────────────┬───────────────────────┤
│ Left Panel (60% width)              │ Central Panel         │
│ - Real-time decoded image display   │ - SSTV decoder        │
│ - Auto-updating during playback     │   settings            │
│ - 512px fixed width, variable height│ - Parameter controls  │
├─────────────────────────────────────┴───────────────────────┤
│ Bottom Panel (200px height): Waveform & Audio Controls      │
│ - Interactive waveform visualization                        │
│ - Play/Pause/Stop/Skip controls                             │
│ - Real-time position tracking                               │
│ - Click-to-seek functionality                               │
└─────────────────────────────────────────────────────────────┘
```

### Core Components Deep Dive

#### **VoyagerApp (src/app.rs)**

Main application state managing UI interactions, playback state (and, with the `audio_playback` feature enabled, audio output), and real-time decoding.

**Key Fields:**
```rust
pub struct VoyagerApp {
    // Audio & Playback Management
    wav_reader: Option<WavReader>,                // WAV file handler with dual-channel support
    is_playing: bool,                            // Real-time playback state
    current_position_samples: usize,             // Precise sample-level position
    playback_start_time: Option<Instant>,        // For position tracking calculations
    selected_channel: WaveformChannel,          // Left/Right channel selection

    // SSTV Decoding System
    video_decoder: SstvDecoder,                  // Core decoder with sync detection
    params: DecoderParams,                       // User-configurable parameters
    image_texture: Option<TextureHandle>,        // Current decoded image for display
    last_decoded: Option<Vec<u8>>,              // Cached pixel data

    // Interactive UI State
    waveform_hover_position: Option<f32>,       // Mouse hover position (0.0-1.0)
}
```

**Core Methods:**
- `toggle_playback()` - Manages play/pause state with time tracking
- `stop_playback()` - Resets position and stops playback
- `seek_to_next_sync()` - Automatic navigation using sync detection
- `decode_at_position()` - Real-time decoding during playback
- `draw_waveform_internal()` - Interactive waveform rendering with mouse handling

#### **Audio System (src/audio.rs)**

Robust WAV file handling with comprehensive format support.

```rust
pub struct WavReader {
    pub left_channel: Vec<f32>,      // Normalized f32 samples (-1.0 to 1.0)
    pub right_channel: Vec<f32>,     // Always populated (duplicated for mono)
    pub sample_rate: u32,            // Original sample rate preserved
    pub channels: u16,               // 1 (mono) or 2 (stereo)
}

pub enum WaveformChannel {
    Left,    // Primary channel or mono
    Right,   // Secondary channel for stereo
}
```

**Features:**
- Automatic mono-to-stereo duplication
- Precision f32 sample normalization
- Channel selection for stereo files
- Robust error handling for invalid files

#### **SSTV Decoder (src/sstv.rs)**

Advanced signal processing with FFT-based sync detection and configurable decoding.

```rust
pub struct SstvDecoder;

pub struct DecoderParams {
    pub line_duration_ms: f32,    // Scanline duration (1-100ms)
    pub threshold: f32,           // Binary threshold (0.0-1.0)
}
```

**Key Methods:**
- `detect_sync_tone()` - FFT-based 1200Hz sync detection with Hann windowing
- `find_sync_positions()` - Locates all sync signals in audio stream
- `find_next_sync()` - Navigation helper for automatic seeking
- `decode()` - Core SSTV decoding with configurable parameters

**Signal Processing Pipeline:**
1. **FFT Analysis** - RealFFT with 2048-sample chunks and Hann windowing
2. **Frequency Detection** - Target 1200Hz with 10x average threshold
3. **Line Extraction** - Configurable samples-per-line based on duration
4. **Amplitude Processing** - Binary threshold conversion to 0/255 pixels
5. **Image Assembly** - 512-pixel fixed width with variable height

#### **Image Processing (src/image_output.rs)**

Efficient pixel-to-texture conversion for real-time display.

```rust
pub fn image_from_pixels(pixels: &[u8]) -> ColorImage {
    let width = 512;  // Fixed width for consistent display
    // Variable height based on pixel data length
    // Grayscale conversion to egui::Color32 format
}
```

**Features:**
- Fixed 512-pixel width for consistency
- Variable height based on decoded data
- Efficient grayscale-to-color conversion
- Proper handling of partial scanlines

#### **Interactive Waveform Visualization**

Real-time waveform rendering with full mouse interaction support.

**Rendering Pipeline:**
1. **Sample Processing** - Min/max amplitude detection per pixel column
2. **Visual Mapping** - Samples-per-pixel calculation for efficient display
3. **Drawing Operations** - Vertical lines from min to max amplitude
4. **Overlay Graphics** - Position indicators and hover lines

**Mouse Interaction:**
- **Click-to-seek** - Convert pixel position to sample position
- **Hover tracking** - Real-time hover line with position preview
- **Visual feedback** - Immediate UI updates for all interactions

### Data Flow Architecture

```text
┌─────────────┐    ┌──────────────┐    ┌─────────────────┐
│ WAV File    │───▶│ Audio Reader │───▶│ Channel Samples │
└─────────────┘    └──────────────┘    └─────────────────┘
                                                │
┌─────────────────────────────────────────────────────────┐
│                Real-time Playback Loop                  │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │Position     │  │Waveform      │  │Live Decoding   │  │
│  │Tracking     │─▶│Visualization │─▶│& Image Update  │  │
│  └─────────────┘  └──────────────┘  └────────────────┘  │
└─────────────────────────────────────────────────────────┘
                                │
┌─────────────────┐    ┌──────────────┐    ┌─────────────┐
│ SSTV Decoder    │◀───│ Sync Signal  │───▶│ UI Display  │
│ (Binary Output) │    │ Detection    │    │ (Texture)   │
└─────────────────┘    └──────────────┘    └─────────────┘
```

## Development Guidelines

### Performance Considerations

**Real-time Requirements:**
- Waveform updates at 60fps during playback
- Non-blocking UI operations during audio processing
- Efficient memory usage for large audio files
- Smart texture updates to avoid GPU stalls

**Optimization Strategies:**
- Min/max amplitude caching for waveform rendering
- Incremental position updates vs. full recalculation
- Texture reuse for decoded images
- Sample-rate adaptive processing

### Error Handling Patterns

**Robust File Loading:**
```rust
// Pattern: Graceful degradation with user feedback
match WavReader::from_file(&path) {
    Ok(reader) => {
        self.wav_reader = Some(reader);
        // Reset UI state for new file
    },
    Err(e) => {
        eprintln!("Failed to load WAV file: {}", e);
        // UI remains in previous state
    }
}
```

**Audio Processing Safety:**
- Bounds checking for all sample access
- Graceful handling of malformed audio data
- Safe channel selection for mono/stereo files
- Position clamping to prevent overruns

### Testing Strategy

**Comprehensive Coverage (29 Tests):**

**Unit Tests (25):**
- `audio.rs` - WAV loading, channel processing, format handling
- `sstv.rs` - Sync detection, decoding parameters, FFT processing
- `image_output.rs` - Pixel conversion, grayscale handling, sizing
- `utils.rs` - Duration formatting, edge cases, boundary conditions

**Integration Tests (4):**
- Full workflow from WAV to decoded image
- Stereo channel selection accuracy
- Parameter variation effects on output
- Error handling and edge case robustness

**Test Data Generation:**
- Synthetic SSTV signals with known patterns
- Stereo test files with different channel content
- Edge cases: empty files, malformed headers, extreme parameters

### Code Organization Principles

**Module Responsibilities:**
- `app.rs` - UI state, user interactions, real-time coordination
- `audio.rs` - File I/O, format conversion, channel management
- `sstv.rs` - Signal processing, sync detection, decoding algorithms
- `image_output.rs` - Pixel processing, texture generation
- `utils.rs` - Helper functions, formatting, utilities

**State Management:**
- Centralized state in `VoyagerApp` with clear ownership
- Immutable data structures where possible
- Explicit lifetime management for audio data
- Clear separation between UI state and processing state

### Future Architecture Considerations

**Scalability Features:**
- Tiled image system for high-resolution support (beyond 16K pixel height limit)
- Color image decoding with YUV/RGB channel separation
- Parameter preset system for different image types
- Session state persistence and project files

**Performance Enhancements:**
- Background decoding thread with message passing
- Audio streaming for very large files
- GPU-accelerated image processing
- Adaptive quality scaling based on zoom level

**Advanced Features:**
- Real-time rodio audio playback integration
- Signal analysis tools (spectrum analyzer, noise reduction)
- Export functionality (PNG, TIFF, raw pixel data)
- Batch processing capabilities

### Development Workflow

**Code Quality Checklist:**
1. All new features must include comprehensive tests
2. Run `cargo clippy` and address all warnings
3. Ensure `cargo fmt` formatting compliance
4. Verify real-time performance with large audio files
5. Test interactive features with mouse/keyboard input
6. Document all public APIs and complex algorithms
7. Update this CLAUDE.md file for architectural changes

**Performance Testing:**
- Test with various audio file sizes (1MB to 100MB+)
- Verify smooth playback with different sample rates
- Measure memory usage during long playback sessions
- Profile waveform rendering performance
- Test seek operations with precise timing

### Debugging Guidelines

**Common Issues and Solutions:**

**Audio Playback Problems:**
- Check ALSA library installation on Linux
- Verify audio file format compatibility (16-bit PCM WAV)
- Ensure sample rate is supported (44.1kHz, 48kHz tested)

**Waveform Display Issues:**
- Verify mouse event handling in interactive regions
- Check amplitude scaling calculations
- Ensure proper coordinate system mapping (screen to sample)

**Decoding Quality Problems:**
- Adjust line duration parameters for different image types
- Fine-tune amplitude threshold for signal characteristics
- Verify sync detection frequency alignment (1200Hz)

**Real-time Performance:**
- Monitor frame rates during playback (`RUST_LOG=debug`)
- Check for blocking operations in UI update loop
- Verify efficient texture update patterns

This architecture provides a solid foundation for real-time audio visualization and decoding, with clear separation of concerns, comprehensive testing, and excellent performance characteristics.
