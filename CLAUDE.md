# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Voyager Golden Record Explorer is a Rust + Egui desktop application that decodes and visualizes analog image data encoded on NASA's Voyager Golden Record. The app performs real-time SSTV-style audio-to-image decoding with an interactive GUI.

## Development Commands

### Build and Run
```bash
cargo run                    # Run the application in debug mode
cargo build                  # Build debug version
cargo build --release        # Build optimized release version
```

### Testing and Quality
```bash
cargo test                   # Run tests
cargo clippy                 # Run linter
cargo fmt                    # Format code
```

### Debugging
```bash
RUST_LOG=debug cargo run     # Run with debug logging enabled
```

## Architecture Overview

The application follows a modular design with clear separation of concerns:

### Core Components

- **`main.rs`**: Entry point with eframe setup and window configuration (1024x720 default)
- **`app.rs`**: Main application state (`VoyagerApp`) implementing the eframe::App trait
  - Manages UI state, file loading, and decoding operations
  - Handles user interactions and parameter adjustments
- **`audio.rs`**: WAV file handling with `WavReader` struct
  - Supports mono/stereo files, converts to normalized f32 samples
  - Provides channel selection (Left/Right) via `WaveformChannel` enum
- **`sstv.rs`**: SSTV decoding logic with `SstvDecoder`
  - Configurable parameters: line duration (ms) and amplitude threshold
  - Converts audio samples to grayscale pixel data
- **`image_output.rs`**: Image rendering utilities
  - Converts pixel arrays to egui::ColorImage for display
  - Fixed width of 512 pixels, variable height based on data
- **`utils.rs`**: Currently minimal utility functions

### Key Data Flow

1. User loads WAV file → `WavReader::from_file()` processes audio
2. User selects channel (L/R) and adjusts decode parameters
3. Decode button → `SstvDecoder::decode()` processes samples
4. Pixel data → `image_from_pixels()` creates displayable image
5. Image rendered in central panel via egui texture system

### Dependencies

- **egui/eframe**: GUI framework with image loader support
- **hound**: WAV file reading and audio processing
- **rodio**: Audio playback capabilities (integrated but not fully utilized)
- **rfd**: Native file dialog for WAV selection
- **image**: Image format support (JPEG, PNG)

## Development Notes

### Decoder Parameters
- `line_duration_ms`: Controls how many audio samples represent one image line (default: 8.3ms)
- `threshold`: Amplitude threshold for binary pixel conversion (0.0-1.0, default: 0.2)

### Image Format
- Fixed width: 512 pixels
- Variable height based on audio length and line duration
- Grayscale output (binary thresholding currently)

### Asset Files
Sample WAV files in `assets/` directory:
- `golden_record_left.wav`, `golden_record_right.wav`, `golden_record_stereo.wav`
- `voyager-golden-record-cover.jpg`

### UI Layout
- Top panel: File loading, decode button, and parameter controls
- Bottom panel: File info and decode statistics
- Central panel: Decoded image display

## Future Architecture Considerations

The codebase is designed for extensibility with planned features:
- Tiled image paging system for high-resolution viewing
- Color image decoding support
- Real-time streaming decode
- Parameter presets for different image types
- Advanced signal processing tools