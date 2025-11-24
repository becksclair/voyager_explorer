# Voyager Explorer TODO

## Now

## Later (Milestone 7 - Next Phase)

(Future enhancements and features will be listed here)

## Done

- [x] Milestone 6: Kickoff & Planning
  - [x] Milestone 6 kickoff: choose owner and scope for spectrum view + batch decode
  - [x] Spike: sketch UI for signal analysis panel (spectrum + playback overlay)
  - [x] Define batch processing flow (CLI flags + optional UI queue) and list required WAV fixtures

- [x] Milestone 6: Advanced features
  - [x] Spectrum Panel Enhancements
    - [x] Add logarithmic frequency scale toggle
    - [x] Add dB magnitude scale
    - [x] Add peak frequency detection and display
  - [x] Batch Processing UI
    - [x] Create Batch Processing panel/tab
    - [x] Implement file selection (multiple files)
    - [x] Implement progress tracking (queue, progress bar)
    - [x] Integrate with existing `batch::process_file` logic

- [x] Milestone 1: Implement real rodio audio playback (src/app.rs)
  - [x] Add feature-gated `audio_stream: Option<(OutputStream, OutputStreamHandle)>` to VoyagerApp
  - [x] Implement `ensure_audio_stream()` helper to lazily initialize rodio
  - [x] Extend `AudioBufferSource` with proper implementation for seeking
  - [x] Add `make_buffer_source_from_current_position()` helper
  - [x] Wire up `toggle_playback()` with rodio integration (play/pause/resume)
  - [x] Wire up `stop_playback()` with rodio cleanup
  - [x] Update seek operations (waveform click + skip to sync) to restart audio
  - [x] Ensure feature flag behavior (builds with/without `audio_playback`)
  - [x] Tests pass and code compiles both with and without feature

- [x] Milestone 1 follow-up: rodio/audio_state alignment (src/app.rs + src/audio_state.rs + src/audio.rs)
  - [x] Integrate `AudioPlaybackState` and `AudioMetrics` into `VoyagerApp` (replaced bare `is_playing` with state machine)
  - [x] Update rodio wiring to use `OutputStream::try_default()` and `Sink::try_new(&handle)` (rodio 0.21 API)
  - [x] Switch `AudioBufferSource` to use shared `Arc<[f32]>` (zero-copy seeks) - eliminates O(n) clone per seek
  - [x] Convert `WavReader` fields to `Arc<[f32]>` for efficient buffer sharing
  - [x] Add audio status indicator in debug panel using `AudioPlaybackState::status_icon()` / `status_message()`
  - [x] Update all state checks throughout app.rs to use `audio_state.is_playing()`
  - [x] Refactor `toggle_playback()`, `stop_playback()`, `restart_audio_from_current_position()` for state machine
  - [x] Add metrics recording (play/pause/stop/seek counts)
  - [x] Fix feature-gated imports to eliminate clippy warnings
  - [x] All tests pass (48 tests), zero clippy warnings, cargo check succeeds

- [x] Milestone 2: Non-blocking decoding & performance (src/app.rs)
  - [x] Define `DecodeRequest` and `DecodeResult` message structs
  - [x] Add channels (`decode_tx`, `decode_rx`) to VoyagerApp
  - [x] Spawn background worker thread with own SstvDecoder instance
  - [x] Refactor `decode_at_position` to enqueue jobs instead of blocking
  - [x] Poll `decode_rx` in `update()` and apply latest results only
  - [x] Manual QA: verify UI remains smooth during decode

- [x] Milestone 3: Sync detection logging & cleanup
  - [x] Fix `detect_sync` logging in `src/sstv.rs`
  - [x] Clean up unused imports in `src/app.rs`
  - [x] Run clippy and fix warnings

- [x] Milestone 4: Color image decoding (src/sstv.rs + src/image_output.rs)
  - [x] Add `DecoderMode` enum (BinaryGrayscale, PseudoColor)
  - [x] Extend `DecoderParams` with `mode` field
  - [x] Implement PseudoColor decoding (group 3 lines as R/G/B)
  - [x] Add color image helper or extend `image_from_pixels`
  - [x] Add UI ComboBox for mode selection
  - [x] Add tests for color mode

- [x] Milestone 5: Presets, Session Persistence, Export
  - [x] Sub-milestone 5.1: Parameter Presets
    - [x] Define `DecoderPreset` struct with name and params
    - [x] Create static list of presets (Voyager Default, Test Pattern, etc.)
    - [x] Add preset UI (ComboBox) in central panel
    - [x] Track custom vs preset state

  - [x] Sub-milestone 5.2: Session State Persistence
    - [x] Add `serde` + `serde_json` dependencies
    - [x] Define `SessionState` struct (serializable with wav path, position, channel, params, preset name)
    - [x] Implement Save/Load Session buttons with rfd dialogs
    - [x] Add session state serialization/deserialization helpers

  - [x] Sub-milestone 5.3: Image Export
    - [x] Implement Save Image button (PNG export via `image` crate)
    - [x] Add file dialog for save location
    - [x] Optionally implement Save Raw Pixels

---

**Notes:**
- Follow `specs/implementation.md` for detailed requirements per milestone
- Run quality gates after each milestone: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo run`
- Update CHANGELOG.md for user-visible changes and new dependencies
- Use synthetic audio fixtures for testing (see `research/synthetic_audio_fixtures.md`)
