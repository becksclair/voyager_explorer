# AGENT GUIDANCE

- Prototype/MVP; accept debt, skip security/CI infra work, reuse existing code, and only add deps when they save >30 minutes.
- After edits run the happy-path smoke (`cargo run` or `RUST_LOG=debug cargo run`), `cargo build`/`cargo build --release`, `cargo fmt` + `cargo clippy`, and `cargo check`/`cargo test`.

## Build & Test

- `cargo run` for debug preview; `cargo build` or `cargo build --release` for artifacts; append `RUST_LOG=debug` when tracing logs.
- Format and lint with `cargo fmt` and `cargo clippy`, use `cargo check` for quick type validation.
- Run `cargo test` (all), `cargo test --lib` (unit), `cargo test --test integration_tests`; single-test shorthand `cargo test audio::tests::test_mono_wav_loading`.
- Coverage/regressions only when needed via `cargo tarpaulin --out html`.

## External APIs & docs

- On external crate errors (rodio, etc.), first inspect `Cargo.toml` for the crate and version.
- Use `cargo doc --open` or docs.rs for that version and look up the exact types/functions you're touching.
- Don't guess APIs from memory; align signatures/types to the docs and then refactor.
- For borrow checker issues in UI code, split immutable reading/drawing from mutating calls by using flags like `pending_seek` and calling the mutating method afterwards.
- For feature-gated modules (`test_fixtures`, `audio_playback`), run tests with `--features test_fixtures` and configure `rust-analyzer.cargo.features` accordingly for good IDE hints.

## Architecture

- `src/main.rs` starts eframe, installs egui image loaders, and instantiates `VoyagerApp`.
- `VoyagerApp` (src/app.rs) owns WAV input, `SstvDecoder`, textures, playback state, waveform hover logic, and forwards audio playback (optional `audio_playback`/rodio feature).
- `audio::WavReader` normalizes samples, fills `WaveformChannel`, and exposes `get_samples` for UI/decoder.
- `sstv::SstvDecoder` (plus `DecoderParams`) drives FFT-based sync detection, `find_sync_positions`, `find_next_sync`, and returns 512px-wide binary pixel streams fed to `image_output::image_from_pixels`.
- `utils::format_duration`, `assets/golden_record_*.wav`, and `tests/` + `#[cfg(test)]` modules round out support; no databases or external services are involved.

## Style

- Use CamelCase for types, snake_case for functions/locals, keep `use` blocks grouped, and keep each module focused on a single concern.
- Handle `Result`/Option with `match`/`if let`, log errors with `eprintln!`, clamp audio accesses, and leave UI state unchanged on failure (see CLAUDE.md error-handling patterns).
- Keep `cargo fmt`/`cargo clippy` clean and document architecture or public API changes in CLAUDE.md; update CLAUDE.md when you reshape modules.
- Rely on README.md (features + commands) and CLAUDE.md (architecture, performance, testing) for house style; no `.cursor`, `.windsurfrules`, `.clinerules`, `.goosehints`, or `.github/copilot-instructions.md` exist here.
