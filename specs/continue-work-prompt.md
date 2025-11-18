# Instructions

You are working on a Rust desktop app called “Voyager Golden Record Explorer”.

Your job: **implement the next milestones end‑to‑end, autonomously**, including running tests and verifying behavior, while keeping changes minimal and prototype‑friendly.

## PROJECT CONTEXT (DO NOT SKIP)

This is a Rust 2021 + egui/eframe desktop app that:
- Loads Voyager Golden Record WAV audio (`hound` via `WavReader`).
- Provides interactive waveform visualization + click‑to‑seek.
- Performs SSTV‑style decoding to produce a 512px‑wide image (`SstvDecoder` + `image_output`).
- Has optional audio playback via `rodio` behind an `audio_playback` feature.

**Key files to read FIRST (no edits yet):**

1. `AGENTS.md`
   - Project rules: prototype/MVP mindset, minimal deps, what commands to run after changes.

2. `CLAUDE.md`
   - Architecture overview, data flow, quality gates, dev workflow.

3. `specs/implementation.md`
   - Multi‑phase implementation plan and milestones.
   - Pay special attention to:
     - Section “0.2. Key Modules”
     - Section “0.7. Current Implementation Status (2025-11-18)”
     - Milestone 1, 2, 3 descriptions.

4. `TODO.md`
   - Current actionable tasks.
   - Note the “Now” section; it starts with:
     - “Milestone 1 follow-up: rodio/audio_state alignment (src/app.rs + src/audio_state.rs)”
     - Then “Milestone 2: Non-blocking decoding & performance (src/app.rs)”

5. `README.md`
   - High‑level feature list, what’s considered implemented vs planned.

6. Audio design docs (read, don’t edit):
   - `docs/audio_playback_design.md`
   - `docs/refactoring_progress.md`
   - `docs/testing_without_audio.md`
   - `docs/ultrathink_summary.md`

7. Core modules (skim to refresh, detailed when you change them):
   - `src/app.rs`
   - `src/audio.rs`
   - `src/audio_state.rs`
   - `src/sstv.rs`
   - `src/image_output.rs`
   - `src/test_fixtures.rs`
   - `tests/integration_tests.rs`
   - `tests/audio_playback_tests.rs`

## GLOBAL CONSTRAINTS

Follow these rules strictly:

- **Prototype / MVP bias**
  - Implement minimum that demonstrates value. Accept technical debt.
  - Don’t introduce heavy new dependencies unless they save >30 minutes and are obviously justified.

- **Security & infra**
  - Ignore auth/security hardening.
  - Do NOT touch CI/CD, containers, or deployment infra.

- **Tests**
  - At least keep existing tests green.
  - Only add focused tests for new behavior or to cover a confirmed bug.
  - Synthetic audio fixtures are already available; prefer them over real assets.

- **Style & docs**
  - Don’t delete or rewrite existing comments/docs unless necessary.
  - If you change architecture or public API, update `CLAUDE.md` and/or `specs/implementation.md` minimally and precisely.

- **Dependencies**
  - Prefer existing crates in `Cargo.toml`.
  - If you absolutely must add a dependency, pick a lightweight crate and note why (time saved or risk reduced).

## EXECUTION PRIORITIES (WHAT TO IMPLEMENT NEXT)

Treat this as your ordered roadmap:

1. **Milestone 1 follow-up: rodio/audio_state alignment**
   Goal: make audio playback via `rodio` fully working and aligned with the design.

   Concretely:
   - In `src/app.rs` and `src/audio_state.rs`:
     - Integrate `AudioPlaybackState` and `AudioMetrics` into `VoyagerApp` (replace bare `is_playing: bool` / manual counters).
     - Align rodio integration with **rodio 0.21** APIs:
       - Use `OutputStreamBuilder::open_default_stream()` which returns `(OutputStream, OutputStreamHandle)`.
       - Store both `OutputStream` and `OutputStreamHandle` in `VoyagerApp`.
       - Create `Sink` instances via `Sink::try_new(&OutputStreamHandle)` and handle failure gracefully.
     - Refactor `AudioBufferSource` and its usage to avoid cloning large `Vec<f32>` on each seek:
       - Prefer shared `Arc<[f32]>` buffers + offsets as in the design docs.
     - Add a minimal audio status indicator in an existing panel using:
       - `AudioPlaybackState::status_icon()` and `status_message()`.
   - Ensure the `audio_playback` feature build still compiles and runs.

2. **Milestone 2: Non-blocking decoding & performance**
   Goal: decode off the UI thread so the egui UI remains responsive.

   Concretely:
   - In `src/app.rs`:
     - Define `DecodeRequest` and `DecodeResult` structs (position/samples/params in; pixel buffer out).
     - Add decoding channels to `VoyagerApp` (e.g. `decode_tx`, `decode_rx`) and spawn a worker thread that owns an `SstvDecoder`.
     - Refactor `decode_at_position` to enqueue a request instead of blocking the UI thread.
     - In `update`, poll `decode_rx` and apply only the most recent decode result to `image_texture` / `last_decoded`.
   - Keep the visual behavior the same from the user’s perspective, just smoother.

3. **Milestone 3: Sync detection logging & minor fixes**
   Goal: clean up sync detection behavior & logs.

   Concretely:
   - In `src/sstv.rs`:
     - Fix `SstvDecoder::detect_sync` so it only prints `"Sync tone not detected!"` if no sync was detected at all.
   - Run `cargo clippy` and clean up low‑hanging warnings related to audio/sync code without large refactors.

If you reach these, continue with Milestones 4–6 as described in `specs/implementation.md`, but **only after** 1–3 are done and verified.

## WORKFLOW & TOOLING EXPECTATIONS

Operate autonomously and methodically:

1. **Context & plan**
   - After reading the files above, synthesize a short internal plan:
     - Affected files (3–7 max at a time).
     - The minimal code changes needed per milestone.
   - Update `specs/implementation.md` **only** to reflect actual status and design changes you introduce.
   - Update `TODO.md` to mark completed tasks and add any small, concrete follow‑ups you create.

2. **Code changes**
   - Prefer editing existing modules over creating new ones.
   - Keep diffs focused; avoid large rewrites unless strictly necessary.
   - Respect feature flags:
     - `audio_playback` gates rodio usage.
     - `test_fixtures` gates synthetic audio helpers.

3. **Verification (MANDATORY after non-trivial changes)**
   - From repo root, run at least:
     - `cargo fmt`
     - `cargo clippy --all-targets` (fix or explicitly justify any remaining warnings you leave)
     - `cargo test --all`
     - `cargo check`
   - For audio changes, also run:
     - `cargo run` (or `cargo run --features audio_playback`)
     - Manually load a WAV (or synthetic test WAV) and confirm:
       - Playback starts/stops/pauses as expected.
       - Seeking works and doesn’t crash.
       - UI remains responsive while decoding.

   - If any command fails:
     - Do NOT ignore it.
     - Investigate, fix the root cause, and re-run until green.

4. **Autonomy & reporting**
   - You do not need human confirmation between substeps.
   - However, keep the codebase in a state where:
     - It builds.
     - Tests pass.
     - The plan in `specs/implementation.md` and `TODO.md` accurately describes reality.

5. **Safety & scope**
   - Do not modify CI config, Dockerfiles, or deployment scripts.
   - Do not introduce complex new subsystems (e.g. databases, services).
   - Keep all work constrained to this app’s current architecture.

## SUCCESS CRITERIA

Your work is considered successful when:

- `cargo test --all`, `cargo fmt`, `cargo clippy --all-targets`, and `cargo check` all succeed.
- With `audio_playback` enabled, the app:
  - Plays audio via rodio without obvious errors.
  - Reflects audio state via `AudioPlaybackState` in the UI.
  - Allows seeking without large allocations or stuttering.
- Decoding happens off the UI thread, and the egui UI remains smooth during playback/decoding.
- `specs/implementation.md` “Current Implementation Status” and relevant milestone sections match the code.
- `TODO.md` has all Milestone 1 follow‑up and Milestone 2 items either completed or updated with clearly defined remaining work.
