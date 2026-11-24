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
- Test assets (gitignored): `assets/sync_image1.wav` is the
  calibration-circle-only oracle; `assets/golden_record_excerpt_48k.wav`
  is the multi-image fixture (left channel, lead-in tone + 16 frames,
  calibration circle through the "Home"/Earth color triplet, including the
  ill-posed Solar Spectrum triplet). Regenerate it with `just fixtures`
  from the source 48 kHz rip. Ignored integration tests in
  `tests/excerpt_fixture_tests.rs` (segments-into-16, per-frame picture
  rows, burst-open frames not sheared, Earth-triplet composites) run with
  `cargo test --test excerpt_fixture_tests -- --ignored`.

## Architecture

- `main.rs` — eframe entry point plus clap CLI: `batch`, the diagnostics
  subcommands (flattened from `cli.rs`), and a `--load <wav>` GUI flag.
- `cli.rs` — diagnostics harness: `decode` (time window → PNG with
  rotate/flip/invert/gamma), `spectrogram`, `syncs`, `classify`, `stats`,
  `segment` (auto image-boundary catalog, optional per-image PNG decode
  that also lists each frame's detected picture rows),
  `carve`. Thin shims over `analysis/` + `pipeline`; this is the
  agent-facing iteration loop against the real assets in `assets/`.
- `app.rs` (`VoyagerApp`) — composition root: UI layout, playback state,
  and panel wiring. Orchestration lives in `services/`. The preview
  decodes one whole image at the playhead, from per-image ranges
  (`find_image_bounds`) scanned in the background on load/channel switch.
  With no bounds available, it decodes a `decode_window_secs` window.
  A window decode made during a scan (Decode button or seek) refreshes
  once at the current playhead if the scan finds an image and no later
  decode supersedes it. A still-current decode dropped by a worker restart
  is re-sent once (`RestartRetry`), playing or paused.
- `audio.rs` (`WavReader`) — WAV loading/normalization: float32 and
  integer PCM at any rate ≥ 8 kHz (the masters are 384 kHz float32);
  `from_file_range` for windowed reads of huge files; samples live in
  shared `Arc<[f32]>` buffers so seeks are zero-copy.
- `audio_state.rs` — `AudioPlaybackState` state machine and `AudioError`.
  Use `audio_state.is_playing()`, never bare booleans.
- `sstv.rs` (`SstvDecoder`, `DecoderParams`) — baseband slow-scan decoder:
  sync-locked line segmentation (time-domain detector from
  `analysis/sync.rs`, fixed-period fallback), stabilized 2nd-order
  Butterworth inverse high-pass DC restoration (`restore_dc` /
  `restore_dc_with_preroll`, default on, `--no-dc-restore` in CLI
  `decode`/`segment`/`batch`, "DC restore" in GUI; GUI batch inherits
  the active preview `DecoderParams` with the batch panel's mode;
  windowed decodes warm the filter on `DC_RESTORE_PREROLL_SECS` of
  preceding audio; standalone slices, and windows with under 30 ms of
  audio before them, warm on their first two contiguous scan lines; sync
  tracking and picture-row detection still measure the raw signal),
  anti-aliased per-line resampling, percentile normalization with
  `invert`/`gamma`.
  The 1200 Hz FFT tone detector remains for navigation only (tone regions,
  not per-line syncs). PseudoColor (3 scanlines as RGB) is a legacy mode;
  real Voyager color (frame triplets) is Phase 14.
- `analysis/` — `spectrogram` (STFT → labeled PNG), `stats` (RMS/peak/
  ZCR/crest/dominant), `classify` (silence/tone/image-periodic/broadband
  segments), `sync` (spike+falling-edge line detector, predictive
  tracker seeded at the sync edge outside any start burst and walked in
  both directions, re-acquiring phase across inter-image boundaries when
  a coasting walk meets a phase-shifted coherent run, with per-line jitter
  smoothing split into separate runs at re-locks ≥1% of a line off
  prediction and at phase re-acquisitions, interval summary),
  `segment` (per-image boundaries from sync-cadence breaks: split sync
  runs where an interval exceeds `gap_factor`× the median, drop short
  runs and tone-classified runs), `picture` (`find_picture_rows`: the
  rows of one decoded frame that hold the picture, found from raw levels
  by dropping leader lines, which are flat to the noise floor, and the
  start burst), plus the one-shot `compute_spectrum` (linear magnitude;
  UI converts to dB).
- `pipeline.rs`, `batch.rs` — decode pipeline (`PipelineResult` →
  egui/PNG images; `to_egui_image(crop_sync)` transposes to the published
  orientation, with optional display-only cropping: a segmented,
  sync-locked image to its detected `picture_rows`, and any sync-locked
  decode to the columns between its sync bands),
  triplet compositing (`composite_triplet_levels`:
  joint percentile bounds over the content region only, plane
  registration via 2D normalized cross-correlation of per-scan-line-
  demeaned content blocks computed on raw levels before normalization
  clipping, smallest lag within 0.002 of the peak wins), and batch file
  processing; cancellation via `Arc<AtomicBool>` with Release/Acquire
  ordering.
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
sync-locked lines per image (not the nominal 512). Inside a segmented
range a complete picture is 519–534 lines (three ranges on the rip are
cut short); the rest is leader (none to a fifth of the range), a 20-line
start burst (a tone at 16 periods per line, not line-locked: its peaks
also recur 410 samples apart at 48 kHz, close enough to a 400-sample line
to capture a tracker seeded on it; the sync spike is present throughout)
and up to 11 blank lines after it. The recording is AC-coupled: a line
that starts at picture black drifts back toward mid-grey over roughly a
fifth of the line, so with DC restore off black-background diagrams
decode with a grey background and a dark strip at the line start. That
strip is picture; DC restore (default on) flattens the background.
Inter-image gaps
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
