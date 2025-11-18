# Changelog

All notable changes to the Voyager Golden Record Explorer project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2024-12-XX - Real-time Interactive Implementation

### 🚀 Major Features Added

#### **Interactive Audio Playback System**
- **Real-time position tracking** with precise sample-level accuracy
- **Play/Pause/Stop controls** with visual state indicators
- **Automatic playback termination** when reaching end of audio
- **Position display** in MM:SS.SS format with live updates
- **Playback state persistence** during UI interactions

#### **Advanced Waveform Visualization**
- **Full interactive waveform display** in bottom panel (200px height)
- **Min/max amplitude rendering** for efficient visualization
- **Real-time hover line** showing mouse position with yellow indicator
- **Click-to-seek functionality** for precise position control
- **Red position indicator** showing current playback location
- **Optimized rendering** with samples-per-pixel calculation

#### **Enhanced Sync Signal Detection**
- **FFT-based sync detection** using RealFFT with 2048-sample chunks
- **Hann windowing** for improved frequency analysis accuracy
- **Target frequency detection** at 1200Hz with configurable thresholds
- **Multiple sync position finding** (`find_sync_positions()`)
- **Smart navigation** with `find_next_sync()` method
- **"Skip to Next Sync" button** for automatic image boundary navigation

#### **Real-time Decoding Capabilities**
- **Live decoding during playback** with 2-second sliding window
- **Automatic image updates** as audio position advances
- **Non-blocking decode operations** maintaining UI responsiveness
- **Immediate texture updates** for smooth visual feedback
- **Position-based decoding** with configurable decode duration

#### **Modern UI Layout Redesign**
- **Three-panel layout**: Top controls, Left image (60%), Bottom waveform (200px)
- **Central decoder settings panel** with parameter controls
- **Interactive UI elements** with immediate visual feedback
- **Responsive design** adapting to window size changes
- **Professional visual styling** with consistent iconography

### 🔧 Technical Improvements

#### **Audio System Enhancements**
- **Dual-channel support** with Left/Right selection for stereo files
- **Improved WAV loading** with comprehensive format support
- **Normalized f32 sample processing** for consistent amplitude handling
- **Channel duplication** for mono files (both channels populated)
- **Robust error handling** for malformed audio files

#### **SSTV Decoder Upgrades**
- **Configurable parameters** (line duration 1-100ms, threshold 0.0-1.0)
- **Binary threshold decoding** with real-time parameter updates
- **512-pixel fixed width** with variable height output
- **Improved line extraction** with accurate sample-per-line calculation
- **Memory-efficient processing** for large audio files

#### **Image Processing Optimization**
- **Efficient pixel-to-texture conversion** using egui::ColorImage
- **Grayscale-to-color mapping** with proper Color32 handling
- **Partial scanline support** for incomplete image data
- **Texture reuse** minimizing GPU memory allocation
- **Variable height handling** for different decoding parameters

### 🧪 Comprehensive Testing Framework

#### **Test Coverage Expansion (29 Tests Total)**
- **25 Unit Tests** covering all core components:
  - Audio processing (mono/stereo, channel selection, format handling)
  - SSTV decoding (sync detection, parameter variations, FFT processing)
  - Image processing (pixel conversion, grayscale handling, boundary conditions)
  - Utility functions (duration formatting, edge cases)

- **4 Integration Tests** validating complete workflows:
  - Full WAV-to-image decoding pipeline
  - Stereo channel selection accuracy
  - Parameter variation effects on output quality
  - Error handling and robustness testing

#### **Test Infrastructure**
- **Synthetic test data generation** for consistent testing
- **Deterministic audio signal creation** with known patterns
- **Comprehensive error scenario coverage** including edge cases
- **Performance validation** with various file sizes and formats
- **Library interface** (`src/lib.rs`) for external test access

### 📚 Documentation Overhaul

#### **Comprehensive README.md**
- **Feature showcase** with visual layout diagram
- **Architecture overview** with component interaction flow
- **Getting started guide** with step-by-step instructions
- **Development documentation** with build commands and testing
- **Technical implementation details** for contributors

#### **Enhanced CLAUDE.md**
- **Complete architecture documentation** for AI assistant guidance
- **Performance considerations** and optimization strategies
- **Testing methodology** and quality standards
- **Development workflow** and debugging guidelines
- **Future roadmap** and scalability considerations

#### **Project Structure Documentation**
- **Clear file organization** with purpose descriptions
- **Dependency explanations** and version specifications
- **Build configuration** with feature flags and targets
- **Library/binary separation** for testing and reusability

### 🔄 Architectural Improvements

#### **State Management**
- **Centralized application state** in VoyagerApp with clear ownership
- **Real-time state synchronization** between playback and UI
- **Efficient state updates** minimizing unnecessary recomputation
- **Clear separation** between UI state and processing state

#### **Performance Optimization**
- **Non-blocking operations** for all real-time features
- **Smart texture management** with update-on-change strategy
- **Efficient waveform rendering** with amplitude caching
- **Memory management** for large audio file handling

#### **Error Handling**
- **Comprehensive error propagation** with Result types throughout
- **Graceful degradation** for file loading failures
- **User feedback** for error conditions with clear messages
- **Safe bounds checking** for all array/vector access

### 🔧 Development Tooling

#### **Build System**
- **Library target** addition for external testing
- **Binary target** specification for application builds
- **Dev dependencies** separation (tempfile for testing)
- **Feature flags** for optional functionality (audio playback)

#### **Code Quality**
- **Rust best practices** following Rebecca's coding standards
- **Clippy compliance** with all warnings addressed
- **Consistent formatting** with cargo fmt standards
- **Documentation standards** for all public APIs

### 🐛 Bug Fixes

- **Fixed compilation errors** with rodio API changes
- **Resolved borrowing issues** in waveform drawing code
- **Corrected pixel boundary handling** in image processing
- **Fixed amplitude scaling** for proper waveform visualization
- **Resolved texture coordinate mapping** for accurate mouse interaction

### 🗂️ Project Structure Changes

```
Added:
├── src/lib.rs                    # Library interface for testing
├── tests/integration_tests.rs    # Comprehensive workflow testing
└── CHANGELOG.md                  # This file

Modified:
├── src/main.rs                   # Public module declarations
├── src/app.rs                    # Complete UI redesign and state management
├── src/audio.rs                  # Enhanced WAV handling and testing
├── src/sstv.rs                   # Advanced sync detection and testing
├── src/image_output.rs           # Optimized rendering and testing
├── src/utils.rs                  # Added comprehensive testing
├── Cargo.toml                    # Build targets and dev dependencies
├── README.md                     # Complete documentation rewrite
└── CLAUDE.md                     # Comprehensive architecture documentation
```

---

## [0.1.0] - 2024-XX-XX - Initial Release

### 🎉 Initial Implementation

#### **Core Features**
- Basic egui desktop application with 1024x720 window
- WAV file loading with hound library integration
- Simple SSTV decoding with binary threshold conversion
- Basic sync tone detection using FFT analysis
- Grayscale image output with 512-pixel fixed width
- Parameter controls for line duration and amplitude threshold

#### **Basic Architecture**
- Modular design with separate audio, decoding, and image modules
- Simple UI with top panel controls and central image display
- Basic error handling for file operations
- Minimal testing framework

#### **Dependencies**
- egui + eframe for GUI framework
- hound for WAV file reading
- realfft for frequency analysis
- rfd for file dialogs
- image crate for format support

#### **Project Setup**
- Initial Cargo.toml configuration
- Basic README with project goals
- Simple build and run instructions
- MIT license specification

---

## Future Releases

### [0.3.0] - In Progress

#### **Milestone 1: Real Audio Playback (COMPLETED)**

**Added:**
- **Real rodio audio playback** with actual sound output (feature-gated)
- Feature-gated `audio_stream` field in `VoyagerApp` for `OutputStream` and `OutputStreamHandle`
- `ensure_audio_stream()` helper for lazy initialization of rodio stream
- `make_buffer_source_from_current_position()` to create playback sources from current position
- `restart_audio_from_current_position()` for seamless seeking during playback
- Proper play/pause/resume support with rodio `Sink` integration
- Audio restart on seek operations (waveform click, skip to next sync)

**Changed:**
- `toggle_playback()` now integrates with rodio for real audio output when feature is enabled
- `stop_playback()` properly cleans up rodio resources
- All rodio-related code properly feature-gated with `#[cfg(feature = "audio_playback")]`
- Application builds and runs correctly both with and without `audio_playback` feature
- Visual-only playback simulation when audio_playback feature is disabled

**Technical:**
- `AudioBufferSource` implements `rodio::Source` trait for custom audio playback
- Mono channel playback (after user channel selection)
- Position tracking synchronized between visual and audio playback
- All 29 tests pass with and without audio_playback feature

**Fixed:**
- Deprecated `screen_rect()` replaced with `input(|i| i.viewport().inner_rect...)`
- `samples.len() > 0` changed to `!samples.is_empty()` for clippy compliance
- Added `Default` implementation for `SstvDecoder`

---

### [0.4.0] - Planned (Future Milestones 2-6)
- **Non-blocking decoding** with background worker thread
- **Sync detection logging fixes** and code cleanup
- **Color image decoding** with YUV/RGB channel support
- **Parameter presets** for different Voyager image types
- **Session state persistence** with project file support
- **Export functionality** (PNG, TIFF, raw pixel data)

### [0.5.0] - Planned
- **Tiled image system** for high-resolution viewing beyond GPU limits
- **Advanced signal analysis** tools (spectrum analyzer, noise reduction)
- **Batch processing** capabilities for multiple files

### [1.0.0] - Long-term
- **Complete Voyager Golden Record** decoding support
- **Advanced audio processing** with noise reduction and enhancement
- **Professional UI polish** with themes and accessibility
- **Cross-platform distribution** packages and installers

---

**Note**: This project is dedicated to the spirit of scientific curiosity and humanity's interstellar message contained within NASA's Voyager Golden Record.