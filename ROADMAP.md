# Roadmap

Multi-phase implementation plan for Voyager Golden Record Explorer.
Completed phases are kept as a record of what exists and why; unchecked
items are future work. Keep this file current when milestones land.

Quality gate for every milestone: `just ci` (format check, clippy and tests
on both feature configurations, type checks) plus a manual `cargo run`
smoke test. Tests use synthetic, code-generated audio fixtures
(`src/test_fixtures.rs`) — never binary WAVs in git.

## Phase 1 — Core Foundation (v0.1.0) ✅

- [x] egui/eframe desktop application shell
- [x] WAV loading via `hound` with i16 → f32 normalization
- [x] Binary-threshold SSTV-style decoding to 512px-wide grayscale images
- [x] FFT-based 1200 Hz sync tone detection (`realfft`)
- [x] Adjustable line duration (1–100 ms) and amplitude threshold (0.0–1.0)

## Phase 2 — Interactive Playback & Visualization (v0.2.0) ✅

- [x] Real-time playback position tracking with play/pause/stop controls
- [x] Interactive waveform: min/max amplitude rendering, hover indicator,
      click-to-seek
- [x] Hann-windowed sync detection with `find_sync_positions` /
      `find_next_sync` and "Skip to Next Sync" navigation
- [x] Live decoding during playback over a sliding window
- [x] Stereo support with left/right channel selection
- [x] Library target (`src/lib.rs`) and integration test suite

## Phase 3 — Real Audio Playback (v0.3.0, Milestone 1) ✅

- [x] rodio playback behind the `audio_playback` feature (on by default;
      `--no-default-features` for sandboxed/CI builds)
- [x] `AudioBufferSource` implementing `rodio::Source`, rodio 0.21 API
- [x] Zero-copy buffers: `Arc<[f32]>` shared between `WavReader` and
      sources, making seeks O(1) instead of cloning the remaining buffer
- [x] `AudioPlaybackState` state machine with `AudioError` variants
      replacing bare booleans; status icon and message in the debug panel
- [x] `AudioMetrics` counter scaffolding — never wired into playback
      paths and later removed as dead code (decode/frame/worker metrics
      in `metrics.rs` remain live)
- [x] Seek and skip-to-sync restart audio from the new position
- [x] Synthetic audio fixtures and hardware-free playback tests

## Phase 4 — Non-blocking Decoding & Cleanup (Milestones 2–3) ✅

- [x] Background decode worker thread with `DecodeRequest`/`DecodeResult`
      channels; UI polls results without blocking
- [x] Worker owns its own `SstvDecoder`; health monitoring with
      auto-restart
- [x] Decode performance metrics (duration, queue depth, errors)
- [x] Sync detection logging fixed (no unconditional "not detected")
- [x] Unused imports and scaffolding removed; clippy clean

## Phase 5 — Color Decoding (Milestone 4) ✅

- [x] `DecoderMode` enum: `BinaryGrayscale` (default) and `PseudoColor`
      (groups 3 scanlines as R/G/B)
- [x] Color-capable image output and mode selector in the UI

> Milestone 5 (presets, session persistence, single-image export from the
> UI) was previously documented as complete but was never implemented —
> it lives in Phase 8 below. PNG export exists only via batch processing.

## Phase 6 — Signal Analysis & Batch Processing (Milestone 6) ✅

- [x] Spectrum analysis panel (`egui_plot`): log frequency scale, dB
      magnitude scale, peak frequency detection
- [x] Batch CLI: `voyager_explorer batch --input "*.wav" --output out/`
- [x] Batch UI panel: multi-file queue with per-item status, output
      directory and mode selection, progress bar
- [x] PNG output via the `image` crate (grayscale and color)
- [x] Cancellation via `Arc<AtomicBool>`, per-file error recovery,
      panic isolation (`catch_unwind`), smart-rename on output conflicts

## Phase 7 — Hardening (review pass) ✅

- [x] Comprehensive 23-fix review: Hann window endpoints, pseudo-color
      boundary checks, Release/Acquire ordering on cancellation flags,
      playback timer drift, worker restart race, bounded decode queue,
      waveform seek bounds, shutdown timeout. (An RMS-adaptive threshold
      from this pass was later removed: for normalized audio it provably
      reduced to the plain threshold, so it was dead logic.)
- [x] `thiserror`-based `VoyagerError` hierarchy
- [x] TOML configuration file support (`config.rs`)
- [x] Structured logging via `tracing`

## Phase 8 — Future

- [ ] Decoder parameter presets (named parameter sets with a UI selector
      and custom-state tracking)
- [ ] Session save/load: wav path, position, channel, params
- [ ] "Save Image" PNG export from the main UI (currently batch-only)
- [ ] Tiled image paging for high-resolution viewing beyond GPU texture
      limits
- [ ] Noise reduction / filtering before decoding
- [ ] TIFF and raw pixel export alongside PNG
- [ ] Validate decoding against known Voyager Golden Record images
      end-to-end
- [ ] Audio device disconnect/reconnect recovery during playback
- [ ] UI polish: themes, accessibility
- [ ] Cross-platform distribution packages
