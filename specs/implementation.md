---
description: Multi-phase implementation plan to complete Voyager Golden Record Explorer
---

# Implementation Plan

This document describes a multi-phase, verifiable implementation plan to:

- Complete currently missing core functionality.
- Fix known inconsistencies and rough edges.
- Implement roadmap features in staged milestones.

It is written for a coding agent working on this project **for the first time**. Follow the phases in order and use the verification steps at the end of each milestone.

## 0. Context and Ground Rules

### 0.1. Project Overview

- **Name:** Voyager Golden Record Explorer
- **Tech stack:** Rust 2021, `eframe` + `egui`, `realfft`, `hound`, `rodio` (optional), `rfd`, `image`.
- **Goal:**
  - Load Voyager Golden Record WAV audio.
  - Provide interactive audio playback and waveform visualization.
  - Perform SSTV-style decoding in real time to render a 512px-wide image.

### 0.2. Key Modules

- `src/main.rs`
  - eframe entry point; creates window and runs `VoyagerApp`.
  - Declares public modules: `audio`, `image_output`, `sstv`, `utils`.

- `src/app.rs`
  - Defines `VoyagerApp` state and implements `eframe::App`.
  - Handles:
    - WAV loading via `WavReader`.
    - Playback state (`is_playing`, `current_position_samples`, `playback_start_time`).
    - Real-time decoding (`decode_at_position`).
    - UI layout and waveform drawing.
  - Contains **rodio-related scaffolding**:
    - `AudioBufferSource` implementing `rodio::Source`.
    - `audio_sink: Option<Sink>` field **currently unused**.

- `src/audio.rs`
  - `WavReader` wraps `hound::WavReader`.
  - Normalizes `i16` samples to `f32` in `[-1.0, 1.0]`.
  - Handles mono → dual-channel duplication and stereo splitting.
  - `WaveformChannel` enum (`Left`, `Right`) and `get_samples` API.

- `src/sstv.rs`
  - `SstvDecoder` and `DecoderParams` (line duration and threshold).
  - FFT-based sync detection at 1200 Hz using `realfft`.
  - `find_sync_positions` and `find_next_sync` for navigation.
  - `decode` transforms audio samples into binary (0/255) grayscale pixels of width 512.

- `src/image_output.rs`
  - `image_from_pixels` converts grayscale `[u8]` to `egui::ColorImage`.

- `src/utils.rs`
  - `format_duration` formats seconds as `MM:SS.SS`.

- `tests/integration_tests.rs`
  - End-to-end tests for WAV → decode → image.
  - Tests channel selection, decoder parameter effects, and empty audio handling.

### 0.3. Documentation & Style References

Agents should read these before making non-trivial changes:

- `README.md` – feature list, architecture overview, commands, roadmap.
- `CLAUDE.md` – detailed architecture & development guidelines.
- `AGENTS.md` – project-specific rules (prototype/MVP bias, commands to run after changes).

### 0.4. Commands & Quality Gates

From `README.md`, `CLAUDE.md`, and `AGENTS.md`:

- Build / run:
  - `cargo run`
  - `cargo build`
  - `cargo build --release`
- Tests:
  - `cargo test` (all tests)
  - `cargo test --lib` (unit)
  - `cargo test --test integration_tests` (integration)
- Lint / format / check:
  - `cargo check`
  - `cargo fmt`
  - `cargo clippy`
- Debug / coverage:
  - `RUST_LOG=debug cargo run`
  - `cargo tarpaulin --out html`

**Quality expectation for each milestone:** After code changes, run at least:

- `cargo test`
- `cargo fmt`
- `cargo clippy`
- `cargo check`
- Manual smoke via `cargo run` on a sample WAV.

### 0.5. External Crate Documentation

When implementing, consult official docs on docs.rs:

- `eframe`, `egui`, `egui_extras` – GUI & textures.
- `rodio` – audio playback (`OutputStream`, `Sink`, `Source`).
- `realfft` – FFT planning and processing.
- `hound` – WAV reading.
- `rfd` – file dialogs.
- `image` – image encoding/export.

Search pattern: `"<crate name> <version> docs.rs"`.

### 0.6. Synthetic audio fixtures

All milestones that require audio for tests or manual QA should use **synthetic audio fixtures generated in code**, not large binary `.wav` files committed to the repo.

- See `research/synthetic_audio_fixtures.md` for concrete patterns and helpers (e.g. `create_test_sstv_wav`, `create_wav_file`).
- Prefer short, deterministic signals (sync tones, noise+sync, alternating stripe patterns, stereo differentiation) so tests remain fast and self-explanatory.
- If a fixed "golden" signal is needed, embed raw `i16` samples as a `const` array and wrap them into a temporary WAV via the existing helpers.

---

## Milestone 1 – Real Audio Playback via rodio

### 1.1. Problem Statement

Currently, playback in `VoyagerApp` is **visual only**:

- `is_playing`, `playback_start_time`, and `current_position_samples` drive waveform position and real-time decoding.
- No actual audio is played.
- `AudioBufferSource` and `audio_sink: Option<Sink>` exist but are unused.

Goal: Implement **real audio playback** using `rodio` while preserving existing visual behavior and respecting the `audio_playback` feature.

### 1.2. Design Overview

- Use `rodio` to create an `OutputStream` and `OutputStreamHandle` once per app instance.
- Use `AudioBufferSource` as a `rodio::Source` over a slice of samples for the currently selected channel.
- Store playback objects inside `VoyagerApp` (behind `#[cfg(feature = "audio_playback")]`):
  - `audio_stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>`
  - `audio_sink: Option<Sink>` (already present)
- Map UI actions to rodio calls:
  - Play / Pause → `Sink::play()` / `Sink::pause()`.
  - Stop → `Sink::stop()` and reset state.
  - Seek (waveform click) and "Skip to Next Sync" → rebuild `AudioBufferSource` starting at new sample index.

### 1.3. Implementation Steps

1. **Add state fields (guarded by feature flag)**
   - In `VoyagerApp` struct, add under `#[cfg(feature = "audio_playback")]`:
     - `audio_stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>`.
   - Keep `audio_sink: Option<Sink>` as existing, also feature-gated if needed.

2. **Initialize rodio stream lazily**
   - Create a helper method in `VoyagerApp` (feature-gated):
     - Example signature: `fn ensure_audio_stream(&mut self) -> Option<rodio::OutputStreamHandle>`.
     - If `audio_stream` is `None`, build a new stream using `OutputStreamBuilder` or `OutputStream::try_default()`/similar API from rodio docs.
     - Store `(OutputStream, OutputStreamHandle)` in `audio_stream` and return a handle reference.

3. **Connect `AudioBufferSource` to rodio**
   - `AudioBufferSource` currently stores:
     - `samples: Vec<f32>`
     - `sample_rate: u32`
     - `channels: u16`
   - To support seeking:
     - Either extend `AudioBufferSource::new` to accept an initial `position` parameter, or
     - Continue using `position = 0` but pass a **sliced** sample buffer starting at `current_position_samples`.
   - Add a helper on `VoyagerApp`:
     - Example: `fn make_buffer_source_from_current_position(&self) -> Option<AudioBufferSource>`.
     - Reads `wav_reader`, `selected_channel`, and `current_position_samples`.
     - Slices samples: `samples[current_position_samples..]`.

4. **Implement `toggle_playback` with rodio integration**
   - When toggling from **stopped/paused** to **playing**:
     - Ensure a loaded `wav_reader`; if none, do nothing.
     - Get an audio stream handle via `ensure_audio_stream`.
     - Build an `AudioBufferSource` from the current channel and position.
     - Create a `Sink` from the handle and append the source.
     - Store the `Sink` in `audio_sink`.
     - Set `is_playing = true` and `playback_start_time = Some(Instant::now())`.
   - When toggling from **playing** to **paused**:
     - Call `pause()` on `audio_sink` if present.
     - Maintain `current_position_samples` by using the elapsed time since `playback_start_time` (as currently done) or by computing from known sample count.
     - Set `is_playing = false`.

5. **Implement `stop_playback` with rodio integration**
   - Stop audio using `Sink::stop()` if `audio_sink` is `Some`.
   - Set `audio_sink = None`.
   - Reset `current_position_samples = 0` and `playback_start_time = None`.
   - Ensure waveform and position display reset accordingly.

6. **Integrate seeking and "Skip to Next Sync"**
   - Waveform click handler (inside `update`):
     - Already updates `current_position_samples`.
     - If audio is playing and rodio is enabled:
       - Stop current `Sink` and rebuild an `AudioBufferSource` from the new position.
       - Create a new `Sink` and start playback.
       - Reset `playback_start_time = Some(Instant::now())`.
   - `seek_to_next_sync`:
     - Already computes `current_position_samples` from `video_decoder.find_next_sync`.
     - Mirror the same logic as for waveform click to restart audio at the new position when `is_playing` is true.

7. **Keep visual playback logic consistent**
   - The existing `update` logic uses `playback_start_time` + sample rate to advance `current_position_samples` and trigger `decode_at_position`.
   - After integrating rodio, keep this as the **single source of truth** for position:
     - Always update `current_position_samples` based on elapsed time (even though rodio is also playing).
     - Treat rodio as the audio backend that is assumed to run in sync with wall-clock time.

8. **Feature flag behavior**
   - Ensure the app builds and runs when `audio_playback` is disabled:
     - Guard rodio imports, fields, and methods with `#[cfg(feature = "audio_playback")]`.
     - Provide a no-op implementation of `toggle_playback` and `stop_playback` when the feature is off, preserving existing visual playback simulation if desired.

### 1.4. Verification

- **Compile & tests:**
  - `cargo check`
  - `cargo test`
  - `cargo fmt`
  - `cargo clippy`

- **Manual QA:**
  1. Run `cargo run`.
  2. Load a synthetic WAV generated using the test helpers (see 0.6); optionally also try a real Voyager snippet for manual comparison.
  3. Press `▶ Play`:
     - Observe audio playback on system speakers.
     - Waveform position and decoded image update in real-time.
  4. Press `⏸ Pause`:
     - Audio pauses.
     - Waveform position stops advancing.
  5. Press `⏹ Stop`:
     - Audio stops and playback position resets to start.
  6. Click on waveform to seek:
     - Audio restarts at new position.
     - Position indicator jumps accordingly.
  7. Use `⏭ Skip to Next Sync` while playing:
     - Playback jumps to next sync region.
     - Decoding continues from the new position.

- **Optional:** Run with `RUST_LOG=debug cargo run` and add structured logs around playback and seeking to confirm consistent position handling.

---

## Milestone 2 – Non-blocking Decoding & Performance

### 2.1. Problem Statement

- `decode_at_position` runs in the UI thread during `update`, potentially doing heavy work on large files/high sample rates.
- Documentation (CLAUDE.md, CHANGELOG.md) emphasizes real-time responsiveness and non-blocking behavior.

Goal: Move decoding work off the UI thread to avoid frame drops, while keeping the code simple and maintainable.

### 2.2. Design Overview

- Introduce a **background decoding thread** that listens for decode requests.
- Use `std::sync::mpsc` (already imported in `app.rs`) or another channel to send decode jobs from `VoyagerApp` to the worker.
- The worker performs `video_decoder.decode(...)` and sends pixel buffers back to the UI thread.
- The UI thread applies the latest decoded image to `image_texture`.

### 2.3. Implementation Steps

1. **Define worker messages**
   - Create a `DecodeRequest` struct (in `app.rs` or a small new module):
     - `start_sample: usize`
     - `samples: Vec<f32>` (segment slice)
     - `params: DecoderParams`
     - `sample_rate: u32`
   - Create a `DecodeResult` struct:
     - `start_sample: usize`
     - `pixels: Vec<u8>`

2. **Add channels & thread state to `VoyagerApp`**
   - Add fields:
     - `decode_tx: Option<Sender<DecodeRequest>>`
     - `decode_rx: Option<Receiver<DecodeResult>>`
     - Optionally, a `decode_thread_handle` or just let the thread run detached.
   - Initialize these in `Default` via a helper that spawns a worker thread:
     - Worker loop:
       - Receives `DecodeRequest`s.
       - Calls `video_decoder.decode(...)`.
       - Sends `DecodeResult` back via `decode_rx`.

3. **Refactor `decode_at_position` to enqueue jobs**
   - Instead of calling `video_decoder.decode` directly:
     - Compute segment (`position`, `samples_to_decode`) as currently done.
     - Clone or slice samples into `Vec<f32>`.
     - Send `DecodeRequest` to `decode_tx`.
   - Optionally, avoid spamming jobs:
     - Only enqueue a new job if the previous one has completed or after a minimum time interval.

4. **Consume results in `update`**
   - Early in `update`, poll `decode_rx` for any available `DecodeResult` (non-blocking).
   - For the **latest** result:
     - Call `image_from_pixels` and update `image_texture` and `last_decoded`.
   - Optionally discard older results if multiple are queued; keep only the latest to avoid lag.

5. **Thread safety considerations**
   - `SstvDecoder` is currently used from the UI thread; decide on ownership:
     - Option A: Each worker thread owns its own `SstvDecoder` instance (simplest).
     - Option B: Share a `SstvDecoder` behind `Arc<Mutex<...>>`.
   - Prefer Option A for simplicity: instantiate `SstvDecoder` inside the worker thread.

### 2.4. Verification

- **Compile & tests:**
  - `cargo test`
  - `cargo fmt`
  - `cargo clippy`

- **Manual QA:**
  1. Run `cargo run`.
  2. Load a large/high-sample-rate synthetic WAV generated via the helpers described in 0.6.
  3. Start playback and interact with the UI (resize window, move mouse over waveform).
  4. Confirm UI remains smooth and responsive while image updates.
  5. Monitor logs (optional) to ensure worker thread is processing decode requests and not panicking.

- **Optional performance test:**
  - Use `RUST_LOG=debug cargo run` and log decode durations, job queue behavior, and frame times.

---

## Milestone 3 – Sync Detection Logging & Minor Fixes

### 3.1. Problem Statement

- `SstvDecoder::detect_sync` prints `"Sync tone detected!"` when a sync is found, but also prints `"Sync tone not detected!"` at the end unconditionally, which is misleading.
- `image` crate is enabled but currently unused.
- Several imports (`SineWave`, `mpsc`, `Arc`, `Mutex`, `thread`) are unused or partially used.

Goal: Clean up correctness and maintainability issues without changing behavior.

### 3.2. Implementation Steps

1. **Fix sync detection logging**
   - In `SstvDecoder::detect_sync`:
     - Track a boolean `found_sync` initialized to `false`.
     - When a sync is detected, set `found_sync = true` and print `"Sync tone detected!"`.
     - After the loop, print `"Sync tone not detected!"` only if `found_sync` is `false`.

2. **Clean up unused imports and scaffolding**
   - In `app.rs`:
     - Remove unused imports, or
     - Ensure all imported symbols are used (e.g., if you fully implement decoding worker, channels and threading will be used).
   - Re-run `cargo clippy` and address warnings about dead code, unused variables, and imports.

3. **Document `image` crate intended use**
   - No behavior change required in this milestone.
   - Optionally note in comments or in this spec that `image` will be used for export in Milestone 5.

### 3.3. Verification

- `cargo fmt`
- `cargo clippy` (should report fewer/no warnings compared to baseline).
- `cargo test`
- Manual check of log output when running sync detection (e.g., via existing integration tests or an ad-hoc call) to ensure only one of the messages is printed.

---

## Milestone 4 – Color Image Decoding (Initial Support)

### 4.1. Problem Statement

The roadmap calls for **color image decoding**. Today, `SstvDecoder::decode` outputs **binary grayscale** (0 or 255) based on a threshold.

Goal: Add an initial color decoding mode suitable for Voyager-style images, while keeping the existing binary grayscale mode as default.

### 4.2. Design Overview

- Introduce a `DecoderMode` enum:
  - `BinaryGrayscale` (current behavior).
  - `PseudoColor` (initial color implementation).
- Add a `mode: DecoderMode` field to `DecoderParams` or a separate configuration struct.
- For `PseudoColor` mode, implement a simple yet plausible color interpretation as a first iteration:
  - Example strategy: treat grouped lines as R, G, B components.
  - For every 3 lines of samples, compute three grayscale lines and combine into colored pixels.
- Update UI to allow mode selection and to render color images using `ColorImage` with color pixels.

### 4.3. Implementation Steps

1. **Extend decoder configuration**
   - In `sstv.rs`, define:
     - `pub enum DecoderMode { BinaryGrayscale, PseudoColor }`
   - Add a `mode: DecoderMode` field to `DecoderParams` with default `BinaryGrayscale`.

2. **Implement `PseudoColor` decoding path**
   - Add a new method (or extend `decode`):
     - Maintain backward-compatible signature but internally branch on `params.mode`.
   - For `PseudoColor` mode:
     - For each `samples_per_line` slice, compute grayscale value as currently done.
     - Group every 3 lines of grayscale pixels into one color image row:
       - First line → Red channel intensity.
       - Second line → Green channel intensity.
       - Third line → Blue channel intensity.
     - If not a multiple of 3, treat missing channels as 0 or duplicate last channel.
   - Change `image_from_pixels` or add a new `image_from_color_pixels` helper to handle full RGB data.

3. **Update image rendering**
   - In `app.rs`, when decoding:
     - For grayscale mode, keep current path.
     - For pseudo-color mode, call the new color image helper and create a texture from a color `ColorImage`.

4. **UI controls**
   - Add a `Decoder Mode` control to the central panel:
     - E.g., `ComboBox` with entries `"Binary (B/W)"` and `"PseudoColor"` mapped to `DecoderMode` variants.
   - When the mode is changed, re-run a decode for the current segment or entire file.

5. **Tests**
   - Add tests in `sstv.rs`:
     - Verify that `DecoderParams::default().mode` is `BinaryGrayscale`.
     - For a small synthetic signal, confirm that `PseudoColor` generates pixels of length multiple of width and plausible distributions across channels (e.g., non-zero values in R/G/B positions).

### 4.4. Verification

- `cargo test` (including new tests).
- Manual QA:
  1. Load a WAV file with visible stripe patterns (e.g., synthetic or Voyager data).
  2. Use `Binary (B/W)` mode and observe image.
  3. Switch to `PseudoColor` and confirm that image shows color variation (even if approximate).

---

## Milestone 5 – Presets, Session Persistence, and Export

### 5.1. Problem Statement

Roadmap items:

- Parameter presets for different image types.
- Session state saving/loading.
- Export functionality (PNG, TIFF, raw pixels).

Goal: Add pragmatic implementations for these features that fit the architecture and keep dependencies minimal. This milestone can be implemented in sub-phases if desired.

### 5.2. Parameter Presets

#### 5.2.1. Design

- Introduce a `DecoderPreset` struct:
  - `name: &'static str`
  - `params: DecoderParams`
- Maintain a static list of presets for known image types and test patterns.
- UI: ComboBox in the central panel to select a preset or `Custom`.

#### 5.2.2. Implementation Steps

1. Define presets in a new module or within `app.rs`:
   - Example presets:
     - `"Voyager Default"` – current default params.
     - A small set of variations for experimentation.
2. Add `selected_preset: Option<&'static str>` or similar to `VoyagerApp`.
3. In UI, when selecting a preset:
   - Set `params` to the preset’s values.
   - Mark mode as `Custom` whenever the user manually changes a parameter.
4. Optionally, trigger re-decode when preset changes.

#### 5.2.3. Verification

- `cargo test`.
- Manual QA: switch between presets and confirm parameters update and image changes.

### 5.3. Session State Saving/Loading

#### 5.3.1. Design

- Represent session state as a serializable struct:
  - `wav_path: Option<PathBuf>`
  - `current_position_samples: usize`
  - `selected_channel: WaveformChannel`
  - `decoder_params: DecoderParams`
  - `selected_preset: Option<String>`
- Use a simple text-based format (e.g., JSON) to save/load.
- **New dependencies:** `serde` + `serde_json` are acceptable because they significantly reduce implementation time.

#### 5.3.2. Implementation Steps

1. Add dependencies in `Cargo.toml`:
   - `serde` with `derive` feature.
   - `serde_json`.
2. Define `SessionState` struct in a shared module:
   - Derive `Serialize`, `Deserialize`.
3. Implement `to_session_state` / `from_session_state` helpers on `VoyagerApp`:
   - `to_session_state` gathers current state.
   - `from_session_state` applies stored state (reloads WAV if path is valid, resets playback states, etc.).
4. UI buttons:
   - `Save Session` – open save dialog with `rfd`, write JSON.
   - `Load Session` – open open-file dialog, read JSON, apply state.

#### 5.3.3. Verification

- `cargo test` for serialization/deserialization round-trips.
- Manual QA:
  1. Load a WAV, choose channel and parameters, seek somewhere.
  2. Save session.
  3. Quit & restart app.
  4. Load session and confirm state restored.

### 5.4. Export Functionality (PNG/TIFF/raw)

#### 5.4.1. Design

- Use existing pixel buffer (`last_decoded: Option<Vec<u8>>`) and `image` crate to export.
- Support at least PNG export; TIFF is optional in first iteration.

#### 5.4.2. Implementation Steps

1. Add a `Save Image` button in the image panel.
2. Behavior when clicked:
   - If `last_decoded` is `None`, disable or show a message.
   - Else, open a save dialog via `rfd`.
   - Convert grayscale (or color) pixels to an `image` crate buffer:
     - For grayscale: use `GrayImage` or convert to RGB by repeating luminance.
     - For color: use `RgbaImage`.
   - Call `image::save_buffer_with_format` with appropriate dimensions and format (PNG).
3. Optionally add `Save Raw Pixels` that writes the raw `Vec<u8>` and metadata to disk.

#### 5.4.3. Verification

- Manual QA:
  1. Decode an image.
  2. Export as PNG.
  3. Open in an external viewer; confirm dimensions and content match on-screen image.

---

## Milestone 6 – Advanced Signal Analysis & Batch Processing (Optional)

These are more advanced/optional features that can be approached after the core functionality is complete.

### 6.1. Signal Analysis Tools

- Add a simple spectrum analyzer panel:
  - Use `realfft` to compute magnitude spectrum of a window around the current position.
  - Display via `egui` plotting widgets.
- Add basic noise reduction (e.g., simple filter or smoothing) before decoding.

**Verification:**

- `cargo test` for any new helper methods.
- Manual QA to confirm spectrum updates when moving through the waveform.

### 6.2. Batch Processing

- Provide a non-interactive mode (CLI or additional UI) to:
  - Load multiple WAV files.
  - Apply selected preset and decode.
  - Export images to an output directory.

**Verification:**

- Integration-style tests that synthesize a few WAVs and verify images are produced without panic.

---

## Execution Strategy for Agents

1. **Start with Milestone 1:** implement rodio playback and keep visual playback in sync.
2. After each milestone:
   - Run the full command suite (`cargo test`, `cargo fmt`, `cargo clippy`, `cargo check`, `cargo run`).
   - Manually test behavior as described.
3. Keep `README.md`, `CLAUDE.md`, and this spec in sync when making structural changes.
4. When adding dependencies (e.g., `serde`, `serde_json`), document rationale in `CHANGELOG.md` under a new version section.

This plan should provide enough detail for an autonomous coding agent to implement and verify all missing functionality and roadmap items in a staged, testable way.
