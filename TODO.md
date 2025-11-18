# Voyager Explorer TODO

## Now (Milestone 2 - Non-blocking Decoding)

- [ ] Milestone 2: Non-blocking decoding & performance (src/app.rs)
  - [ ] Define `DecodeRequest` and `DecodeResult` message structs
  - [ ] Add channels (`decode_tx`, `decode_rx`) to VoyagerApp
  - [ ] Spawn background worker thread with own SstvDecoder instance
  - [ ] Refactor `decode_at_position` to enqueue jobs instead of blocking
  - [ ] Poll `decode_rx` in `update()` and apply latest results only
  - [ ] Manual QA: verify UI remains smooth during decode

## Next (Milestone 3)

- [ ] Milestone 3: Sync detection logging & cleanup (src/sstv.rs)
  - [ ] Fix `detect_sync` to only print "not detected" when no sync found
  - [ ] Clean up unused imports in app.rs after worker implementation
  - [ ] Run clippy and address warnings

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

---

**Notes:**
- Follow `specs/implementation.md` for detailed requirements per milestone
- Run quality gates after each milestone: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo run`
- Update CHANGELOG.md for user-visible changes and new dependencies
- Use synthetic audio fixtures for testing (see `research/synthetic_audio_fixtures.md`)
