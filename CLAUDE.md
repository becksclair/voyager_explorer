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

- `main.rs` — eframe entry point plus CLI (`batch` subcommand via clap).
- `app.rs` (`VoyagerApp`) — UI state, playback coordination, background
  decode worker (request/result channels), batch UI state.
- `audio.rs` (`WavReader`) — WAV loading and normalization; samples live in
  shared `Arc<[f32]>` buffers so seeks are zero-copy.
- `audio_state.rs` — `AudioPlaybackState` state machine and `AudioError`.
  Use `audio_state.is_playing()`, never bare booleans.
- `sstv.rs` (`SstvDecoder`, `DecoderParams`) — FFT sync detection (1200 Hz),
  binary grayscale and pseudo-color decoding to 512px-wide pixel streams.
- `analysis.rs` — spectrum FFT (returns linear magnitude; UI converts
  to dB).
- `pipeline.rs`, `batch.rs`, `services/` — decode pipeline and batch
  processing; cancellation via `Arc<AtomicBool>` with Release/Acquire
  ordering.
- `image_output.rs` — pixels to `egui::ColorImage` / PNG.
- `ui/` — waveform, spectrum, controls, and batch panels.
- `config.rs` (TOML), `error.rs` (thiserror), `metrics.rs`,
  `test_fixtures.rs` (synthetic audio generators, always compiled; no
  binary fixtures in git).

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
