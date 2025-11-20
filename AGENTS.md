# AGENT GUIDANCE

- Prototype/MVP; accept debt, skip security/CI infra work, reuse existing code, and only add deps when they save >30 minutes.
- After edits run the happy-path smoke (`cargo run` or `RUST_LOG=debug cargo run`), `cargo build`/`cargo build --release`, `cargo fmt` + `cargo clippy`, and `cargo check`/`cargo test`.

## Build & Test

- `cargo run` for debug preview (audio_playback enabled by default); `cargo build` or `cargo build --release` for artifacts; append `RUST_LOG=debug` when tracing logs.
- For sandboxed environments: `cargo build --no-default-features` or `cargo test --no-default-features`.
- Format and lint with `cargo fmt` and `cargo clippy`, use `cargo check` for quick type validation.
- Run `cargo test` (all), `cargo test --lib` (unit), `cargo test --test integration_tests`; single-test shorthand `cargo test audio::tests::test_mono_wav_loading`.
- Coverage/regressions only when needed via `cargo tarpaulin --out html`.

## External APIs & docs

- On external crate errors (rodio, etc.), first inspect `Cargo.toml` for the crate and version.
- Use `cargo doc --open` or docs.rs for that version and look up the exact types/functions you're touching.
- Don't guess APIs from memory; align signatures/types to the docs and then refactor.
- For borrow checker issues in UI code, split immutable reading/drawing from mutating calls by using flags like `pending_seek` and calling the mutating method afterwards.
- For feature-gated modules (`test_fixtures`, `audio_playback`), run tests with `--features test_fixtures` and configure `rust-analyzer.cargo.features` accordingly for good IDE hints.
- Note: `audio_playback` is enabled by default; use `--no-default-features` to disable it for CI/CD environments.

## Architecture

- `src/main.rs` starts eframe, installs egui image loaders, and instantiates `VoyagerApp`.
- `VoyagerApp` (src/app.rs) owns WAV input, `SstvDecoder`, textures, playback state, waveform hover logic, and forwards audio playback (optional `audio_playback`/rodio feature).
- `audio::WavReader` normalizes samples, fills `WaveformChannel`, and exposes `get_samples` for UI/decoder.
- `sstv::SstvDecoder` (plus `DecoderParams`) drives FFT-based sync detection, `find_sync_positions`, `find_next_sync`, and returns 512px-wide binary pixel streams fed to `image_output::image_from_pixels`.
- `utils::format_duration`, `assets/golden_record_*.wav`, and `tests/` + `#[cfg(test)]` modules round out support; no databases or external services are involved.

## Style

- Use CamelCase for types, snake_case for functions/locals, keep `use` blocks grouped, and keep each module focused on a single concern.
- Handle `Result`/Option with `match`/`if let`, log errors with `eprintln!`, clamp audio accesses, and leave UI state unchanged on failure (see *Error handling & robustness* below).
- Keep `cargo fmt`/`cargo clippy` clean and document architecture or public API changes in this file; keep it and `README.md` in sync when you reshape modules.
- Rely on `README.md` (features + commands) and this file (architecture, performance, testing) for house style; no `.cursor`, `.windsurfrules`, `.clinerules`, `.goosehints`, or `.github/copilot-instructions.md` exist here.

## Project overview

- Voyager Golden Record Explorer is a Rust + egui desktop app that decodes SSTV-style image data from NASA's Voyager Golden Record.
- It loads WAV audio via `audio::WavReader`, visualizes the waveform, detects sync tones, decodes to 512px-wide grayscale images, and displays them in real time.
- `specs/implementation.md` is the canonical implementation roadmap; keep it and this file in sync when making architectural changes.

## Modules & responsibilities

- `app.rs` (`VoyagerApp`): UI state, user interactions, playback, and real-time coordination.
- `audio.rs` (`WavReader`): WAV loading, format handling, channel management, and normalization.
- `sstv.rs` (`SstvDecoder`, `DecoderParams`): sync detection and SSTV-style decoding.
- `image_output.rs`: pixel processing and texture generation.
- `utils.rs`: helper utilities such as `format_duration`.

## Performance & UX

- Aim for smooth real-time playback and waveform updates; keep heavy work off the UI hot path where possible.
- Cache or reuse expensive computations (e.g. waveform min/max per column, decoded textures) instead of recomputing every frame.
- Avoid blocking operations inside egui painting and audio callbacks.

## Error handling & robustness

- Follow the file-loading pattern: log with `eprintln!`, keep UI state unchanged on failure, and return early.
- Clamp indices and positions when mapping between screen coordinates and sample indices or seeking in audio.
- Treat malformed or extreme input data defensively, but avoid panicking in normal user flows.

## Development workflow

- When setting up the repo, enable shared git hooks with `git config core.hooksPath githooks` (or run `just install-hooks`) so the `githooks/pre-commit` hook runs `cargo fmt` automatically on each commit.
- Prefer small, incremental changes that keep the app running; prototype/MVP first, then refine.
- For non-trivial edits, run at least: `cargo fmt`, `cargo clippy`, `cargo check`, and `cargo test` (with `--features test_fixtures` when relevant), plus a quick `cargo run` smoke test.
- **CRITICAL: Before marking any task as complete, run `just ci` to verify all CI checks pass.** This runs formatting checks, clippy on both feature configurations, all tests, and type checking—ensuring everything works before pushing to GitHub.
- Add at least one happy-path test when it provides clear value, especially for new decoding or audio behaviors.
- Verify real-time performance and interactive behavior with reasonably large WAV files before considering work "done".
- Use the `justfile` for common tasks (see `just --list` for all available commands); key recipes include `just run`, `just test-all`, `just clippy-all`, and `just ci`.
- Keep `AGENTS.md`, `README.md`, and `specs/implementation.md` aligned when you reshape modules or public APIs.
