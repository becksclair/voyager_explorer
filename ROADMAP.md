# Roadmap

Multi-phase implementation plan for Voyager Golden Record Explorer.
Completed phases are kept as a record of what was built; unchecked items
are future work. Keep this file current when milestones land.

## Status (June 2026): the decoder works

The repair plan (Phases 8–13 below) is complete. The app decodes real
Voyager Golden Record audio end-to-end:

- **Gate 1 passed** — a clean, round calibration circle decodes from
  `assets/sync_image1.wav`, both via the diagnostics CLI and live in
  the GUI.
- **Gate 2 demonstrated** — recon decodes from the full 48 kHz stereo
  rip produced the pulsar map, the M31 galaxy photograph, and readable
  mathematical/physical-unit definition slides matching published
  reference decodes. The full 116-image catalog is Phase 14 work.

How it works now: `WavReader` reads float32 and integer WAVs at any
rate including the 384 kHz masters, with windowed range reads for the
multi-hundred-MB files. The decoder treats the signal as baseband
slow-scan video — per-line sync alignment from time-domain
spike/falling-edge detection with re-anchoring (slant correction),
anti-aliased resampling to the target width, percentile contrast
stretch with polarity and gamma parameters. Playback position comes
from the audio device clock (`Sink::get_pos`), and the live decode
window anchors to it. A CLI diagnostics harness (`decode`,
`spectrogram`, `syncs`, `classify`, `stats`, `carve`) drives all
analysis and regression work against the real assets. Round-trip tests
use an independent forward model with slant/noise injection rather
than mirroring decoder internals.

For the audit that motivated the repair (int16-only loader that could
never read the float32 assets, binary-threshold placeholder decoder,
unused sync detection, drifting frame-timer playhead, circular tests),
see the annotated history in Phases 1–7 and the git history.

## Success gates

- **Gate 1 (passed):** decode a recognizable calibration circle from
  `assets/sync_image1.wav` via the diagnostics CLI.
- **Gate 2 (in progress):** decode known images from the full record
  rips that are recognizable side-by-side against published reference
  decodes — demonstrated for several images; full catalog pending.

## Reference ground truth (encoding facts the decoder is built on)

- ~8.32 ms per scan line (≈3 197 samples at 384 kHz, ≈400 at 48 kHz),
  ~512 vertical scan lines per image, ~4.25 s per image; lines all scan
  the same direction (no boustrophedon); the cover's "interlace" appears
  as alternating sync spacing.
- Line start: a positive spike followed by a falling edge; the bottom of
  the falling edge marks the line start. Line period drifts with analog
  tape speed — decoders re-anchor at every sync and apply sub-sample
  slant correction.
- Brightness polarity is rip-dependent; inversion must be a parameter.
  The calibration circle (first image, left channel) is the tuning
  oracle: ellipse ⇒ wrong scan width, slant ⇒ wrong line period.
- 116 images across both channels; color images are three successive
  frames composited as R/G/B; group assignments are lookup tables in all
  reference decoders (`foodini/voyager`, `aizquier/voyagerimb`,
  `amazing-rando/voyager-decoder`, `MarcBaeuerle/Golden-record-images`).

---

## History — Phases 1–7 (as built, with corrections)

### Phase 1 — Core Foundation (v0.1.0)

- [x] egui/eframe desktop application shell
- [x] WAV loading via `hound` — **int16-only; cannot read the float32
      assets (fixed in Phase 8)**
- [x] Binary-threshold decoding to 512px-wide images — **a placeholder,
      not SSTV decoding; never produced a real image**
- [x] FFT-based 1200 Hz sync tone detection (`realfft`) — **never used
      by the decode path**
- [x] Adjustable line duration (1–100 ms) and amplitude threshold

### Phase 2 — Interactive Playback & Visualization (v0.2.0)

- [x] Play/pause/stop controls with position tracking — **frame-clocked
      timer that drifts from the audio playhead**
- [x] Interactive waveform: min/max rendering, hover, click-to-seek
- [x] Hann-windowed sync detection and "Skip to Next Sync" navigation
- [x] Live decoding during playback — **window not anchored to playback
      position**
- [x] Stereo support with left/right channel selection
- [x] Library target and integration test suite — **circular fixtures**

### Phase 3 — Real Audio Playback (v0.3.0)

- [x] rodio playback behind the `audio_playback` feature
- [x] `AudioBufferSource` over zero-copy `Arc<[f32]>` buffers
- [x] `AudioPlaybackState` state machine with `AudioError` variants
- [x] Seek and skip-to-sync restart audio from the new position

### Phase 4 — Non-blocking Decoding & Cleanup

- [x] Background decode worker with request/result channels, health
      monitoring, auto-restart, and decode metrics

### Phase 5 — Color Decoding

- [x] `DecoderMode::PseudoColor` (3 scanlines as R/G/B) — **wrong model
      for Voyager color; superseded by frame-triplet color in Phase 9**

### Phase 6 — Signal Analysis & Batch Processing

- [x] Spectrum analysis panel (`egui_plot`), log scale, peak readout
- [x] Batch CLI and batch UI panel with progress and cancellation
- [x] PNG output via the `image` crate; per-file error recovery, panic
      isolation, smart-rename on conflicts

### Phase 7 — Hardening (review pass)

- [x] 23-fix review (windowing, ordering, drift, races, bounds),
      `thiserror` error hierarchy, TOML config, `tracing` logging —
      **hardened the plumbing; did not touch the decoder's validity**

---

## Repair plan

### Phase 8 — Diagnostics harness (CLI-first) ✅

The iteration loop for everything that follows: library-level analysis
functions surfaced as CLI subcommands, runnable against the real assets.

- [x] Fix WAV ingestion: handle `SampleFormat::Float` (f32) and integer
      bit depths correctly; accept 384 kHz; add
      `WavReader::from_file_range` for cheap windowed reads of the
      728 MB+ files
- [x] Grow `analysis.rs` into an `analysis/` module:
      STFT spectrogram rendered to PNG with labeled frequency/time
      markers; rolling signal stats (RMS, peak, DC, ZCR, dominant
      frequency); segment classifier (silence / tone / image-like
      periodic / broadband); time-domain peak+falling-edge sync detector
      with inter-sync interval histogram
- [x] CLI subcommands: `decode` (time window → PNG, with rotate/flip),
      `spectrogram`, `syncs`, `classify`, `stats`, `carve` (cut WAV
      excerpts for fixtures)
- [x] Gate met: `sync_image1.wav` reads with real RMS; spectrogram shows
      the calibration-tone harmonic stack and line-periodic image
      structure; sync intervals cluster at 8.32–8.33 ms (643/656 within
      ±1% in the image region)

### Phase 9 — Decoder rewrite (Gate 1) ✅

- [x] Baseband level decoding: direct signal-level → grayscale with
      polarity (`invert`) parameter (no inversion needed for the 48 kHz
      remaster). Anti-aliasing via bin-averaged downsampling; an
      explicit pre-filter proved unnecessary in practice.
- [x] Per-line sync alignment via time-domain detection; median-interval
      validation with per-line re-anchoring (slant correction); fixed
      fallback when no line cadence is present
- [x] Bin-averaged downsampling / linear-interp upsampling to the target
      width; percentile contrast stretch; optional gamma
- [x] Non-circular tests: true forward-model encoder with slant/noise
      injection, correlation round-trips (sync lock provably beats fixed
      slicing under slant), ignored-by-default integration test against
      `sync_image1.wav`
- [x] **Gate 1 PASSED: clean, round calibration circle from
      `assets/sync_image1.wav`.** Recon on the full 48 kHz stereo rip
      decoded the pulsar map, M31 galaxy, and the readable mathematical/
      physical-unit definition slides — matching published references
      (Gate 2 partially demonstrated).
- [ ] Frame-triplet color mode (3 successive frames → R/G/B with
      registration) — moved to Phase 14: needs image-boundary
      segmentation, which belongs with the full-record catalog work.
      The 3-scanline PseudoColor mode remains as a legacy curiosity.

### Phase 10 — Real playback clock & anchored live decode ✅

- [x] Playhead from `Sink::get_pos()` plus the source's base sample
      offset (`services/playback.rs`); the audio device is the clock, so
      UI frame jitter cannot drift the position. Seeks rebuild the
      source zero-copy and reset the base (`try_seek` unnecessary).
- [x] Live decode requests anchored to the device playhead
- [x] Position math unit-tested (pure function over sink duration)

### Phase 11 — App state decomposition ✅

- [x] Extracted `DecodeOrchestrator` (worker channels, ids, queue depth,
      health, restart, shutdown) and `BatchRunner` (worker thread,
      progress channel, cancellation, reaping) into `services/`;
      `VoyagerApp` composes them with no behavior change

### Phase 12 — Visual redesign (egui) ✅

- [x] `ui/theme.rs`: dark mission-console palette (near-black chrome,
      teal active states, amber sync accents, cyan signal traces),
      shared panel frames and typography helpers
- [x] Layout rework: header, transport bar with monospace timecode,
      full-width waveform strip with cached amber sync markers
      (computed on a background thread per load), three-column main row
      (spectrum + signal info / scrollable image view with line count /
      decode controls + export), segmented status bar with real metrics
- [x] "Export Current Image…" PNG export from the main UI (an old
      Phase 8 wishlist item, landed early)
- [x] `--load <file>` startup flag for deterministic launches

### Phase 13 — GUI verification pass ✅

- [x] Driven live via desktop automation: load via `--load`, full-file
      decode (line count + image render), waveform click-to-seek
      (position readout matches), play/pause/stop with the
      device-anchored clock, live decode following the playhead (the
      calibration circle appears on seek into the image region),
      skip-to-next-sync, status-bar telemetry
- [x] Fixed two bugs the pass uncovered: a spurious "worker
      unresponsive" restart when the first decode request followed an
      idle period (idle time counted as unresponsiveness), and
      Skip-to-Next-Sync overshooting to end-of-file (now walks the
      cached waveform markers instead of rescanning with the FFT tone
      detector)

### Phase 14 — Gate 2 at scale (in progress)

- [x] Image-boundary segmentation (`analysis/segment.rs`, CLI `segment`):
      sync-cadence-break detection splits the record into per-image
      sample ranges; tone-classified runs (lead-in tone) are rejected.
      On `golden_record_stereo_48k.wav` it finds 80 left / 77 right
      candidates against the published 78 + 78 frame catalog, and
      `--decode-dir` decodes every candidate to PNG in one command
      (calibration circle, pulsar map/galaxy, definition slides all
      verified recognizable).
- [x] Frame-triplet color (`catalog.rs`, `pipeline::composite_rgb`, CLI
      `segment --color`): published 78-frame catalog with color roles and
      labels (cross-validated across three reference decoders; R-G-B
      frame order verified empirically on the Sunset triplet), row-offset
      plane registration via luminance-profile correlation, all 20
      triplets composited. Segmentation cleanup (merge false splits,
      split fused runs against the median slot length) brings both
      channels to exactly 78/78.
- [ ] **Gate 2 acceptance:** review all 156 frames + 20 composites
      side-by-side against published reference decodes; fix the residual
      composite quality gaps (plane fringing on some triplets, hue casts
      from per-frame percentile normalization — normalize planes jointly
      per triplet)
- [ ] Big-file streaming/mmap and decimated waveform cache (the 1.5 GB
      stereo rip currently implies ~3 GB resident)
- [ ] Playback-speed detection (sync-interval median ≈ 4.15 ms ⇒ 2×
      speed rip)
- [ ] Decoder presets and session save/load (single-image export from
      the main UI already landed in Phase 12)
- [ ] TIFF/raw export; audio device disconnect recovery; accessibility;
      distribution packaging

Quality gate for every phase: `just ci` (format check, clippy and tests
on both feature configurations, type checks) plus the phase-specific
checks above. Tests use synthetic, code-generated fixtures plus
explicitly ignored integration tests against the real assets (never
binary fixtures in git).
