# Voyager Golden Record Explorer

A Rust + egui desktop application that decodes SSTV-style image data from
NASA's Voyager Golden Record audio. Load a WAV of the record's image
section, play it back, and watch the encoded pictures emerge in real time.

## What it does

- **Plays the audio** (rodio, optional `audio_playback` feature, on by
  default) with play/pause/stop, click-to-seek on an interactive waveform,
  and FFT-based 1200 Hz sync detection with "Skip to Next Sync" navigation.
- **Decodes images live** during playback to 512px-wide output, in binary
  grayscale or pseudo-color (3 scanlines as R/G/B) mode, with adjustable
  line duration and threshold.
- **Analyzes the signal** in a spectrum panel: log frequency scale, dB
  magnitude, peak frequency readout.
- **Processes in batch**, writing decoded images to PNG, via CLI
  (`voyager_explorer batch --input "*.wav" --output out/`) or a UI queue
  with progress and cancellation.

## Getting started

Requires a recent stable Rust toolchain. On Linux, audio playback needs
ALSA headers
(`alsa-lib-devel` / `libasound2-dev`).

```bash
cargo run                            # run with audio playback
cargo build --release                # optimized build
cargo test                           # full test suite
cargo test --no-default-features     # without audio deps (CI/sandboxes)
```

With [just](https://github.com/casey/just) installed, `just --list` shows
all recipes; `just ci` runs the full pre-push verification. Enable the
shared pre-commit hook once per clone with `just install-hooks`.

Load a WAV with **Load WAV**, adjust the decoder parameters, press
**Play**, and click the waveform to seek.

## Documentation

- [ROADMAP.md](ROADMAP.md) — implementation plan: completed phases and
  future work
- [CLAUDE.md](CLAUDE.md) — architecture, conventions, and agent/contributor
  guidance

## License

MIT

*"To the makers of music — all worlds, all times."*
