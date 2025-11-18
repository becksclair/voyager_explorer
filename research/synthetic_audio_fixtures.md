---
description: Strategy for embedding small synthetic audio fixtures for testing
---

# Synthetic Audio Fixtures Strategy

This document describes how Voyager Golden Record Explorer should represent and use audio data for tests without committing large binary assets.

The core idea: **treat audio fixtures as code**, not as `.wav` files in the repository.

## 1. Current Pattern (Recommended)

The project already uses a good pattern in `tests/integration_tests.rs` and `audio.rs`:

- Generate raw `i16` sample values in Rust code.
- Use a helper like `create_wav_file(samples, sample_rate, channels)` to write a proper 16-bit PCM WAV header and data into a `tempfile::NamedTempFile`.
- Run tests against that temporary WAV file.

### 1.1. Benefits

- **Zero binary blobs** in git history.
- **Deterministic fixtures** – you can tweak the generator code, and tests explain exactly what the signal contains.
- **Small footprint** – even complex scenarios are just a few loops of math over samples.

### 1.2. Existing building blocks

- `tests/integration_tests.rs`:
  - `create_wav_file(samples: &[i16], sample_rate: u32, channels: u16) -> NamedTempFile`
  - `create_test_sstv_wav(sample_rate: u32, duration_secs: f32) -> NamedTempFile`
- `src/sstv.rs` tests:
  - `generate_test_signal(frequency: f32, duration_secs: f32, sample_rate: u32) -> Vec<f32>`
  - `generate_noise(duration_secs: f32, sample_rate: u32) -> Vec<f32>`

These functions already cover:

- Pure tones at specific frequencies (e.g., sync at 1200 Hz).
- Pseudo-random noise.
- Alternating patterns that create visible image lines.

## 2. Golden, Fixed Fixtures as Code

When you need a **fixed** waveform (e.g., a stable reference dataset), use a constant array instead of a `.wav` file:

1. **Precompute offline**
   - Take a short snippet from a real or synthetic WAV (e.g. 0.5–2 s, mono, 44.1 kHz).
   - Dump the raw `i16` samples without the header using a one-off script.

2. **Embed as `const` in test code**
   - In a test-only module (e.g. `tests/golden_audio.rs` or inside `tests/integration_tests.rs`):

     ```rust
     const GOLDEN_SAMPLES: &[i16] = &[
         123, -456, 789, /* ... truncated ... */
     ];
     ```

3. **Use the existing WAV helper**
   - Call `create_wav_file(GOLDEN_SAMPLES, sample_rate, 1)` to obtain a temporary WAV file.
   - Run the full pipeline (WAV → `WavReader` → `SstvDecoder` → `image_output`) against that file.

This gives you a *logical* fixed fixture while keeping the repository free of large binaries.

## 3. Recommended Synthetic Scenarios

For realistic testing and coverage, use combinations of the following synthetic audio patterns:

- **Sync-only segments**
  - Pure 1200 Hz tone at a known amplitude for known durations.
  - Used to validate `detect_sync_tone`, `find_sync_positions`, and `find_next_sync`.

- **Sync + noise**
  - Start with a sync burst, followed by noise, followed by another sync burst.
  - Used to validate navigation between multiple sync positions and robustness to noise.

- **Image-like alternating patterns**
  - Alternating high/low amplitude segments that produce visible stripes when decoded.
  - Used to test that line duration and threshold changes affect decoded image height and content.

- **Stereo differentiation**
  - Left channel: sync tone at 1200 Hz.
  - Right channel: different frequency (e.g. 800 Hz).
  - Used to verify `WaveformChannel` selection and channel-specific sync detection.

Each of these scenarios is already partially implemented in existing tests; new tests should follow the same style.

## 4. When (Not) to Commit .wav Files

Avoid committing `.wav` files unless **absolutely necessary**, for example:

- Legal/licensing constraints require exactly preserving a public Voyager snippet.
- External users or tools depend on a specific binary file.

If you must add a `.wav` file:

- Keep it **short** (≤ a few seconds) and mono if possible.
- Place it under `tests/data/`.
- Document its origin and purpose near the test that uses it.

In most cases, synthetic fixtures generated in code are sufficient and preferred.

## 5. How This Ties Into the Implementation Plan

- All new milestones that need audio for tests (rodio playback, non-blocking decoding, presets, export, etc.) should **use these synthetic generators** instead of real-recording assets.
- `specs/implementation.md` describes where synthetic audio is required; this document describes *how* to build it.

Any agent adding or modifying tests that involve audio should:

1. Look at existing generators in `tests/integration_tests.rs` and `src/sstv.rs` tests.
2. Extend those generators or add new ones in the same style.
3. Avoid committing new binary `.wav` files; prefer code-defined fixtures.
