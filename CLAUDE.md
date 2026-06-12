# Agent Guidance

Prototype/MVP project: accept reasonable debt, reuse existing code, skip
security/CI infra work, and only add dependencies when they save real time.
Prefer small, incremental changes that keep the app running.

## Build & Test

- `cargo run` — debug run (`audio_playback` feature on by default);
  `RUST_LOG=debug cargo run` for verbose logs.
- `cargo build --no-default-features` / `cargo test --no-default-features`
  — sandboxed environments without audio libraries.
- Single test: `cargo test audio::tests::test_mono_wav_loading`.
- **Before declaring any task complete, run `just ci`** — format check,
  clippy and tests on both feature configurations, type checks.
  `just --list` shows all recipes.

## Architecture

- `main.rs` — eframe entry point plus clap CLI: `batch`, the diagnostics
  subcommands (flattened from `cli.rs`), and a `--load <wav>` GUI flag.
- `cli.rs` — diagnostics harness: `decode` (time window → PNG with
  rotate/flip/invert/gamma), `spectrogram`, `syncs`, `classify`, `stats`,
  `segment` (auto image-boundary catalog, optional per-image PNG decode),
  `carve`. Thin shims over `analysis/` + `pipeline`; this is the
  agent-facing iteration loop against the real assets in `assets/`.
- `app.rs` (`VoyagerApp`) — composition root: UI layout, playback state,
  and panel wiring. Orchestration lives in `services/`.
- `audio.rs` (`WavReader`) — WAV loading/normalization: float32 and
  integer PCM at any rate ≥ 8 kHz (the masters are 384 kHz float32);
  `from_file_range` for windowed reads of huge files; samples live in
  shared `Arc<[f32]>` buffers so seeks are zero-copy.
- `audio_state.rs` — `AudioPlaybackState` state machine and `AudioError`.
  Use `audio_state.is_playing()`, never bare booleans.
- `sstv.rs` (`SstvDecoder`, `DecoderParams`) — baseband slow-scan decoder:
  sync-locked line segmentation (time-domain detector from
  `analysis/sync.rs`, fixed-period fallback), anti-aliased per-line
  resampling, percentile normalization with `invert`/`gamma`. The 1200 Hz
  FFT tone detector remains for navigation only (tone regions, not
  per-line syncs). PseudoColor (3 scanlines as RGB) is a legacy mode;
  real Voyager color (frame triplets) is Phase 14.
- `analysis/` — `spectrogram` (STFT → labeled PNG), `stats` (RMS/peak/
  ZCR/crest/dominant), `classify` (silence/tone/image-periodic/broadband
  segments), `sync` (spike+falling-edge line detector, interval summary),
  `segment` (per-image boundaries from sync-cadence breaks: split sync
  runs where an interval exceeds `gap_factor`× the median, drop short
  runs and tone-classified runs), plus the one-shot `compute_spectrum`
  (linear magnitude; UI converts to dB).
- `pipeline.rs`, `batch.rs` — decode pipeline (`PipelineResult` →
  egui/PNG images) and batch file processing; cancellation via
  `Arc<AtomicBool>` with Release/Acquire ordering.
- `services/` — `decoder.rs` (`DecodeOrchestrator`: background decode
  worker, queue depth, health/restart), `batch.rs` (`BatchRunner`),
  `playback.rs` (device-anchored playhead math: base offset +
  `Sink::get_pos()`; never trust frame timers), `audio.rs`
  (`AudioBufferSource`).
- `image_output.rs` — pixels to `egui::ColorImage` / PNG.
- `ui/` — waveform (with cached sync markers), spectrum, controls
  (transport bar), and batch panels; `ui/theme.rs` holds the dark
  "mission console" palette and egui style (`theme::apply_theme`,
  applied once in `main.rs`).
- `config.rs` (TOML), `error.rs` (thiserror), `metrics.rs`,
  `test_fixtures.rs` (synthetic generators plus the forward-model
  `encode_image_to_audio` with slant/noise injection — deliberately
  independent of decoder internals; no binary fixtures in git).

Rip-specific decoding facts (verified): assets are float32 WAV; the
48 kHz remaster needs no polarity inversion; decoded images need
rotate90 + horizontal flip for correct orientation; lines are ~8.32 ms
(≈400 samples at 48 kHz). Catalog slots are ~5.8 s apart with ~550–750
sync-locked lines per image (not the nominal 512); inter-image gaps
show as 1.5–4× interval breaks in the sync cadence; the lead-in
calibration tone sync-locks like an image and must be rejected by
classification, not cadence. "Location of Our Solar System" is one
published image containing both the galaxy picture and the pulsar map.

## Conventions

- Log with `tracing` (`info!`, `warn!`, `error!`), not `eprintln!`.
- On failure: log, leave UI state unchanged, return early. Don't panic in
  user flows; clamp indices when mapping screen coordinates to samples.
- For borrow-checker conflicts in egui code, split immutable drawing from
  mutation with a flag (e.g. `pending_seek`) applied after the closure.
- On external crate errors, check the version in `Cargo.toml` and the
  matching docs.rs pages instead of guessing APIs from memory.
- Keep `cargo fmt`/`cargo clippy` clean. Add a happy-path test for new
  decoding or audio behavior. Update this file and `README.md` when you
  reshape modules or public APIs, and check off / extend `ROADMAP.md` as
  milestones land.

## Setup

Enable the shared pre-commit hook (runs `cargo fmt`) once per clone:
`just install-hooks` or `git config core.hooksPath githooks`.
