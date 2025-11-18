# Voyager Golden Record Explorer

A real-time Rust + Egui desktop application that decodes and visualizes analog image data from NASA's **Voyager Golden Record** with interactive audio playback, waveform visualization, and SSTV-style decoding.

> Transform Voyager Golden Record audio into visible images through real-time SSTV decoding, complete with interactive playback controls, sync signal detection, and comprehensive testing.

![Build Status](https://img.shields.io/badge/build-passing-brightgreen)
![Tests](https://img.shields.io/badge/tests-29%20passing-brightgreen)
![Rust](https://img.shields.io/badge/rust-1.70+-orange)

---

## ✨ Key Features

### 🎵 **Interactive Audio Playback**

_Note: current versions simulate playback visually via position tracking and decoding; audible output via rodio is planned (see Roadmap)._

- **Real-time position tracking** during playback with visual feedback
- **Play/Pause/Stop controls** with state-aware UI
- **Click-to-seek** functionality on waveform visualization
- **Automatic sync detection** with "Skip to Next Sync" navigation
- **Dual-channel support** (Left/Right channel selection for stereo files)

### 📊 **Advanced Waveform Visualization**

- **Real-time waveform rendering** with amplitude scaling
- **Interactive hover line** showing current mouse position
- **Position indicator** showing current playback location
- **Min/max amplitude detection** for optimal visual representation
- **Click-and-drag seeking** for precise position control

### 🖼️ **Real-time SSTV Decoding**

- **Live decoding** during audio playback (2-second sliding window)
- **Automatic image updates** as audio plays
- **Binary threshold decoding** with adjustable parameters
- **512-pixel fixed width** with variable height output
- **Immediate texture updates** for smooth visual feedback

### 🔍 **Enhanced Sync Signal Detection**

- **FFT-based sync detection** at 1200 Hz target frequency
- **Automatic image boundary detection** using Hann windowing
- **Multiple sync position finding** for navigation
- **Smart skip functionality** to jump between image segments

### 🎛️ **Configurable Parameters**

- **Line duration** adjustment (1-100ms per scanline)
- **Amplitude threshold** control (0.0-1.0 sensitivity)
- **Channel selection** for stereo audio files
- **Real-time parameter updates** affecting live decoding

---

## 🏗️ Architecture Overview

### **Modern UI Layout**

```text
┌─────────────────────────────────────────────────────────────┐
│ 🚀 Voyager Golden Record Explorer    [📂 Load] [🧠 Decode]  │
│ 📏 Line Duration: [8.3ms]  🔪 Threshold: [0.2]  📻 [Left]    │
├─────────────────────────────────────┬───────────────────────┤
│                                     │ SSTV Decoder Settings │
│         🖼️ Decoded Image            │                       │
│        (Left Panel - 60%)           │ 📏 Line Duration (ms):│
│                                     │ 🔪 Threshold:        │
│     [Real-time image display]       │                       │
│                                     │   (Central Panel)     │
├─────────────────────────────────────┴───────────────────────┤
│ 📈 Audio Waveform & Controls        (Bottom Panel - 200px) │
│ ▶️ [Play] ⏹️ [Stop] ⏭️ [Skip to Next Sync]              │
│ Position: 01:23.45 / 05:30.12                              │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ ████░░░░██████░░░░░░██░░░█████░░░░░░░░░░░░░░░░░░░░░░░░│ │ │
│ │          ↑ Position    ↑ Hover line                  │ │ │
│ └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### **Core Components**

```rust
// Main Application State
pub struct VoyagerApp {
    // Audio & Playback
    wav_reader: Option<WavReader>,           // WAV file handler
    is_playing: bool,                        // Playback state
    current_position_samples: usize,        // Current position
    selected_channel: WaveformChannel,      // L/R channel

    // SSTV Decoding
    video_decoder: SstvDecoder,              // Core decoder
    params: DecoderParams,                   // Line duration, threshold

    // UI State
    image_texture: Option<TextureHandle>,    // Rendered image
    waveform_hover_position: Option<f32>,   // Mouse hover
    playback_start_time: Option<Instant>,   // Position tracking
}
```

---

## 🔧 Technical Implementation

### **Audio Processing Pipeline**

1. **WAV Loading** (`audio.rs`) - Supports mono/stereo, multiple sample rates
2. **Channel Selection** - User chooses Left or Right for stereo files
3. **Real-time Playback** - Position tracking with precise timing
4. **Sample Processing** - Normalized f32 samples for decoding

### **SSTV Decoding Process**

1. **Sync Detection** - FFT analysis to find 1200Hz sync tones
2. **Line Extraction** - Configurable line duration (samples per scanline)
3. **Amplitude Processing** - Binary threshold conversion
4. **Image Assembly** - 512-pixel width with variable height

### **Interactive Waveform**

1. **Min/Max Rendering** - Efficient amplitude visualization per pixel
2. **Mouse Interaction** - Hover detection and click-to-seek
3. **Position Indicators** - Real-time playback position overlay
4. **Performance Optimization** - Smart redraw and pixel sampling

---

## 🚀 Getting Started

### **Prerequisites**

- Rust 1.70+ with Cargo
- Linux audio libraries: `sudo dnf install alsa-lib-devel`
- WAV audio files from Voyager Golden Record

### **Building & Running**

```bash
# Clone and build
git clone https://github.com/your-username/voyager_explorer
cd voyager_explorer

# Run in development mode
cargo run

# Build optimized release
cargo build --release

# Run comprehensive tests (29 total)
cargo test
```

### **Using the Application**

1. **Load Audio**: Click "📂 Load WAV" to select your audio file
2. **Configure Decoding**: Adjust line duration and threshold parameters
3. **Start Playback**: Click "▶️ Play" to begin real-time decoding
4. **Interactive Navigation**:
   - Click anywhere on waveform to seek
   - Use "⏭️ Skip to Next Sync" for automatic navigation
   - Hover over waveform to see position preview
5. **Real-time Viewing**: Watch images decode as audio plays

---

## 🧪 Comprehensive Testing

### **Test Coverage (29 Tests)**

- **Unit Tests (25)**: Individual component verification
  - Audio processing (WAV loading, channel handling)
  - SSTV decoding (sync detection, parameter variations)
  - Image processing (pixel conversion, grayscale handling)
  - Utility functions (duration formatting, edge cases)

- **Integration Tests (4)**: Full workflow validation
  - Complete WAV-to-image pipeline
  - Stereo channel selection accuracy
  - Parameter variation effects
  - Error handling and edge cases

```bash
# Run all tests
cargo test

# Run specific test categories
cargo test --lib          # Unit tests only
cargo test --test integration_tests  # Integration tests only

# Generate coverage report
cargo tarpaulin --out html
```

---

## 📁 Project Structure

```text
voyager_explorer/
├── src/
│   ├── main.rs           # Application entry point & eframe setup
│   ├── lib.rs            # Library interface for testing
│   ├── app.rs            # Main UI state & interaction logic
│   ├── audio.rs          # WAV file handling (hound integration)
│   ├── sstv.rs           # SSTV decoder with sync detection
│   ├── image_output.rs   # Pixel-to-image conversion
│   └── utils.rs          # Duration formatting utilities
├── tests/
│   └── integration_tests.rs  # Full workflow integration tests
├── assets/               # Sample audio files (gitignored)
│   └── golden_record_*.wav
├── Cargo.toml           # Dependencies & build configuration
├── CLAUDE.md            # Development guidance for AI assistants
└── README.md            # This file
```

---

## 🛠️ Development

### **Key Dependencies**

- **egui + eframe** (0.33.0) - Modern immediate-mode GUI
- **rodio** (0.21.1, optional via `audio_playback` feature) - Audio playback backend for planned sound output
- **hound** (3.5.1) - WAV file reading and processing
- **realfft** (3.5.0) - FFT operations for sync detection
- **rfd** (0.15.4) - Native file dialogs

### **Development Commands**

```bash
# Quick compilation check
cargo check

# Format code
cargo fmt

# Lint with Clippy
cargo clippy

# Debug logging
RUST_LOG=debug cargo run

# Generate documentation
cargo doc --open
```

### **Code Quality Standards**

- Comprehensive error handling with Result types
- Modular architecture with clear separation of concerns
- Extensive testing coverage (unit + integration)
- Performance-optimized real-time processing
- Following Rust best practices and Rebecca's coding standards

---

## 🎯 Roadmap

### **Current Features** ✅

- [x] Real-time playback with visual position tracking (no audio output yet)
- [x] Interactive waveform visualization
- [x] SSTV decoding with configurable parameters
- [x] Enhanced sync signal detection
- [x] Click-to-seek functionality
- [x] Dual-channel audio support
- [x] Comprehensive test suite

### **Planned Enhancements** 🚧

- [ ] Actual rodio audio playback integration
- [ ] Color image decoding support
- [ ] Tiled image paging for high-resolution viewing
- [ ] Parameter presets for different image types
- [ ] Session state saving/loading
- [ ] Export functionality (PNG, TIFF)
- [ ] Advanced signal analysis tools

---

## 🤝 Contributing

Contributions welcome! Areas of focus:
- **Algorithm improvements**: Better sync detection, noise reduction
- **UI enhancements**: Visual polish, accessibility features
- **Performance optimization**: Faster decoding, memory efficiency
- **Feature additions**: Color support, export options
- **Testing**: Additional test scenarios, edge case coverage

### **Getting Started**

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass (`cargo test`)
5. Submit a pull request

---

## 📡 Inspiration & References

- **NASA Voyager Golden Record** - The original interstellar message
- **SSTV (Slow Scan Television)** - Amateur radio image transmission
- **Ham Radio Community** - Digital mode experimentation
- **Analog Signal Processing** - Classic decoding techniques

---

## 📄 License

MIT License - Free to use, modify, and distribute.

_Dedicated to the spirit of curiosity, scientific exploration, and humanity's messages cast into the cosmic ocean._

**"To the makers of music — all worlds, all times."**
