# Voyager Explorer TODO

## Now (Milestone 2 - Non-blocking Decoding)

- [x] Milestone 2: Non-blocking decoding & performance (src/app.rs)
  - [x] Define `DecodeRequest` and `DecodeResult` message structs
  - [x] Add channels (`decode_tx`, `decode_rx`) to VoyagerApp
  - [x] Spawn background worker thread with own SstvDecoder instance
  - [x] Refactor `decode_at_position` to enqueue jobs instead of blocking
  - [x] Poll `decode_rx` in `update()` and apply latest results only
  - [x] Manual QA: verify UI remains smooth during decode

## Now (Milestone 3 - Cleanup & Logging)

- [x] Milestone 3: Sync detection logging & cleanup
  - [x] Fix `detect_sync` logging in `src/sstv.rs`
  - [x] Clean up unused imports in `src/app.rs`
  - [x] Run clippy and fix warnings

## Later (Milestones 4-6)

- [ ] Milestone 4: Color image decoding (src/sstv.rs + src/image_output.rs)
  - [ ] Add `DecoderMode` enum (BinaryGrayscale, PseudoColor)
  - [ ] Extend `DecoderParams` with `mode` field
  - [ ] Implement PseudoColor decoding (group 3 lines as R/G/B)
  - [ ] Add color image helper or extend `image_from_pixels`
  - [ ] Add UI ComboBox for mode selection
  - [ ] Add tests for color mode

- [ ] Milestone 5: Presets, session persistence, export
  - [ ] Define `DecoderPreset` struct with static presets
  - [ ] Add preset UI (ComboBox) and custom state tracking
  - [ ] Add `serde` + `serde_json` dependencies
  - [ ] Define `SessionState` struct (serializable)
  - [ ] Implement Save/Load Session buttons with rfd dialogs
  - [ ] Implement Save Image (PNG export via `image` crate)
  - [ ] Optionally implement Save Raw Pixels

- [ ] Milestone 6: Advanced features (optional)
  - [ ] Signal analysis panel with spectrum view
  - [ ] Batch processing mode (CLI or UI)

## Done

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

---

**Notes:**
- Follow `specs/implementation.md` for detailed requirements per milestone
- Run quality gates after each milestone: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo run`
- Update CHANGELOG.md for user-visible changes and new dependencies
- Use synthetic audio fixtures for testing (see `research/synthetic_audio_fixtures.md`)
