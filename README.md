# Voyager Golden Record Explorer

A Rust + egui desktop application that decodes the images encoded in
NASA's Voyager Golden Record audio. Load a WAV of the record's image
section, play it back, and watch the pictures emerge — the calibration
circle, the pulsar map, the M31 galaxy, the mathematical definition
slides — decoded from the real signal.

## What it does

- **Decodes the record's baseband slow-scan video**: per-line sync
  alignment (time-domain spike/falling-edge detection with slant
  correction), anti-aliased resampling to 512 px lines, percentile
  contrast stretch with polarity and gamma controls. Validated against
  the record's calibration circle and published reference decodes.
- **Plays the audio** (rodio, optional `audio_playback` feature, on by
  default) with play/pause/stop, click-to-seek on an interactive
  waveform with sync markers, and skip-to-next-sync navigation. The
  playhead is anchored to the audio device clock, and live decoding
  follows it during playback.
- **Diagnoses the signal** via a CLI harness: decode any time window to
  PNG (`decode`), render spectrograms with frequency markers
  (`spectrogram`), detect scan-line syncs with interval statistics
  (`syncs`), classify regions as silence/tone/image/broadband
  (`classify`), print signal stats (`stats`), and carve WAV excerpts
  (`carve`). Run `voyager_explorer help` for the full surface.
- **Processes in batch**, writing decoded images to PNG, via CLI
  (`voyager_explorer batch --input "*.wav" --output out/`) or a UI
  queue with progress and cancellation; single-image PNG export from
  the main UI.

## Getting started

Requires a recent stable Rust toolchain. On Linux, audio playback needs
ALSA headers (`alsa-lib-devel` / `libasound2-dev`).

```bash
cargo run                            # run the GUI
cargo run -- --load file.wav         # GUI with a file preloaded
cargo build --release                # optimized build
cargo test                           # full test suite
cargo test --no-default-features     # without audio deps (CI/sandboxes)
```

Decode a window of real record audio from the command line:

```bash
cargo run -- decode --input assets/sync_image1.wav --start 5 \
    --out circle.png --rotate
cargo run -- spectrogram --input assets/sync_image1.wav --out spec.png
```

With [just](https://github.com/casey/just) installed, `just --list`
shows all recipes; `just ci` runs the full pre-push verification.
Enable the shared pre-commit hook once per clone with
`just install-hooks`.

## Documentation

- [ROADMAP.md](ROADMAP.md) — status, success gates, completed phases,
  and future work
- [CLAUDE.md](CLAUDE.md) — architecture, conventions, and
  agent/contributor guidance

## License

MIT

*"To the makers of music — all worlds, all times."*
