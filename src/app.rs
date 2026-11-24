use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::TextureHandle;
#[cfg(feature = "audio_playback")]
use rodio::mixer::Mixer;
#[cfg(feature = "audio_playback")]
use rodio::{OutputStream, OutputStreamBuilder, Sink};

use crate::audio::{WavReader, WaveformChannel};
#[cfg(feature = "audio_playback")]
use crate::audio_state::AudioError;
use crate::audio_state::AudioPlaybackState;
use crate::config::AppConfig;
use crate::error::VoyagerError;
use crate::metrics::AppMetrics;
use crate::pipeline::PipelineResult;
#[cfg(feature = "audio_playback")]
use crate::services::audio::AudioBufferSource;
use crate::services::batch::{BatchProgressMsg, BatchRunner};
use crate::services::decoder::{DecodeOrchestrator, DecodeResult};
use crate::sstv::{DecoderMode, DecoderParams, SstvDecoder};
use crate::ui::batch::BatchPanel;
use crate::ui::controls::{ControlAction, ControlsPanel};
use crate::ui::spectrum::SpectrumPanel;
use crate::ui::theme;
use crate::ui::waveform::WaveformPanel;
use crate::utils::format_duration;

/// Minimum spacing between restart-driven re-requests.
const RESTART_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Re-request policy for a decode that a worker restart dropped.
///
/// A restart discards whatever the worker had queued. The preview would then
/// stay stale whether playing or paused: the per-image playback trigger
/// already counts that image as requested, and a paused seek has no trigger
/// at all. The dropped decode is therefore re-sent at the playhead — once.
/// If the re-sent request takes the worker down as well, the decode itself is
/// failing, and sending it again would only restart the worker in a loop.
/// Re-requests are also spaced by [`RESTART_RETRY_INTERVAL`]; one that falls
/// inside the interval waits for it rather than being lost.
#[derive(Debug, Default)]
struct RestartRetry {
    last_retry: Option<Instant>,
    /// A dropped decode is waiting to be re-sent.
    pending: bool,
    /// The request now with the worker is itself a restart retry.
    retried: bool,
}

impl RestartRetry {
    /// Record a restart. `dropped` is whether it discarded a decode that is
    /// still current.
    fn on_restart(&mut self, dropped: bool) {
        if !dropped {
            return;
        }
        if self.retried {
            tracing::warn!("Decode failed again after a worker restart; not retrying it");
            self.retried = false;
            self.pending = false;
        } else {
            self.pending = true;
        }
    }

    /// A request from the user or from playback replaces whatever was dropped.
    fn on_new_request(&mut self) {
        self.pending = false;
        self.retried = false;
    }

    /// The worker completed a decode, so its next failure is a new one.
    fn on_result(&mut self) {
        self.retried = false;
    }

    fn is_pending(&self) -> bool {
        self.pending
    }

    /// Returns true when the dropped decode should be re-sent now. The caller
    /// sends it and then calls [`Self::mark_retried`].
    fn take_due(&mut self, now: Instant) -> bool {
        let interval_elapsed = self
            .last_retry
            .is_none_or(|last| now.duration_since(last) >= RESTART_RETRY_INTERVAL);
        if !self.pending || !interval_elapsed {
            return false;
        }
        self.pending = false;
        self.last_retry = Some(now);
        true
    }

    fn mark_retried(&mut self) {
        self.retried = true;
    }
}

/// How a window preview shown during the image-bounds scan was produced, so
/// it can be redone the same way as a full frame once the bounds arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowPreview {
    /// The Decode button: decoded on the UI thread.
    Manual,
    /// A seek or live decode: decoded by the background worker.
    Worker,
}

pub struct VoyagerApp {
    // Configuration
    config: AppConfig,

    // Audio data
    wav_reader: Option<WavReader>,
    video_decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<PipelineResult>,
    /// Display-only: hide the decoder's per-line sync/retrace and leader
    /// margins in the image view. Toggling re-skins the cached decode
    /// without re-running the decoder.
    crop_sync_margins: bool,
    selected_channel: WaveformChannel,
    /// Sync-tone positions cached at file load / channel switch; rendered as
    /// amber markers in the waveform strip. Never recomputed per frame.
    sync_positions: Vec<usize>,
    /// Receiver for an in-flight background sync scan. The full-file scan can
    /// take seconds on real Golden Record audio, so it must not block the UI;
    /// replacing the receiver cancels delivery from a stale scan.
    sync_scan_rx: Option<std::sync::mpsc::Receiver<Vec<usize>>>,
    /// Per-image sample ranges for the selected channel, found in the
    /// background on load/channel-switch. The preview decodes one whole image
    /// at a time from these, so a multi-image file shows a recognizable frame
    /// instead of an arbitrary time window.
    image_bounds: Vec<std::ops::Range<usize>>,
    image_bounds_rx: Option<std::sync::mpsc::Receiver<Vec<std::ops::Range<usize>>>>,
    /// A window preview taken while the bounds scan was running, to be
    /// replaced by the full frame once the bounds arrive.
    pending_full_frame: Option<WindowPreview>,
    /// Start sample of the image last sent to the decoder, so playback does
    /// not re-decode the current image every interval.
    last_decoded_image_start: Option<usize>,
    /// Set when a live decode request was dropped because the worker queue was
    /// full. The current image is then re-sent once the queue frees (a poll
    /// drains a slot), rather than re-attempted every frame while the worker is
    /// busy — which would spin `decode_at_position` for nothing.
    pending_image_retry: bool,

    // Audio playback state
    audio_state: AudioPlaybackState,
    #[cfg(feature = "audio_playback")]
    audio_stream: Option<OutputStream>,
    #[cfg(feature = "audio_playback")]
    audio_sink: Option<Sink>,
    current_position_samples: usize,
    last_decode_position: usize,
    waveform_hover_position: Option<f32>,
    /// Sample offset at which the current audio source was appended; the true
    /// playhead is `playback_base_samples + sink.get_pos() · rate`.
    #[cfg(feature = "audio_playback")]
    playback_base_samples: usize,
    /// Visual-only playback simulation clock (no audio device to anchor to).
    #[cfg(not(feature = "audio_playback"))]
    playback_start_time: Option<Instant>,
    #[cfg(not(feature = "audio_playback"))]
    playback_start_position: usize,

    // Background decoding worker
    decode_worker: DecodeOrchestrator,
    restart_retry: RestartRetry,
    /// Bumped on file load, channel switch, and successful manual decode so
    /// older worker results cannot replace the current preview.
    decode_generation: u64,
    /// Last decode error surfaced, to avoid re-raising the identical error
    /// every live-decode interval (which would defeat the dismiss button).
    last_decode_error: Option<String>,

    // Metrics and errors
    metrics: AppMetrics,
    error_message: Option<String>,
    frame_start: Option<Instant>,

    // Signal Analysis
    spectrum_panel: SpectrumPanel,
    waveform_panel: WaveformPanel,

    // Batch Processing
    batch_panel: BatchPanel,
    batch_runner: BatchRunner,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        // Load configuration from file or use defaults
        let mut config = AppConfig::load_or_default(AppConfig::default_path());
        if let Err(e) = config.validate() {
            tracing::warn!("Invalid config, using defaults: {}", e);
            config = AppConfig::default();
        }

        tracing::info!(
            config_path = %AppConfig::default_path().display(),
            "Application configuration loaded"
        );

        // Initialize decoder params from config
        let params = DecoderParams {
            line_duration_ms: config.decoder.default_line_duration_ms,
            invert: config.decoder.default_invert,
            gamma: config.decoder.default_gamma,
            dc_restore: config.decoder.default_dc_restore,
            decode_window_secs: config.decoder.decode_window_secs as f64,
            mode: DecoderMode::Grayscale,
            ..Default::default()
        };

        Self {
            config: config.clone(),
            wav_reader: None,
            video_decoder: SstvDecoder::new(),
            image_texture: None,
            params,
            last_decoded: None,
            crop_sync_margins: true,
            selected_channel: WaveformChannel::Left,
            sync_positions: Vec::new(),
            sync_scan_rx: None,
            image_bounds: Vec::new(),
            image_bounds_rx: None,
            pending_full_frame: None,
            last_decoded_image_start: None,
            pending_image_retry: false,
            audio_state: AudioPlaybackState::Uninitialized,
            #[cfg(feature = "audio_playback")]
            audio_stream: None,
            #[cfg(feature = "audio_playback")]
            audio_sink: None,
            current_position_samples: 0,
            last_decode_position: 0,
            waveform_hover_position: None,
            #[cfg(feature = "audio_playback")]
            playback_base_samples: 0,
            #[cfg(not(feature = "audio_playback"))]
            playback_start_time: None,
            #[cfg(not(feature = "audio_playback"))]
            playback_start_position: 0,
            decode_worker: DecodeOrchestrator::new(),
            restart_retry: RestartRetry::default(),
            decode_generation: 0,
            last_decode_error: None,
            metrics: AppMetrics::new(),
            error_message: None,
            frame_start: None,
            spectrum_panel: SpectrumPanel::default(),
            waveform_panel: WaveformPanel::default(),
            batch_panel: BatchPanel::default(),
            batch_runner: BatchRunner::default(),
        }
    }
}

impl VoyagerApp {
    fn handle_load_wav(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() {
            self.load_wav_from_path(&path);
        }
    }

    /// Load a WAV file directly by path (shared by the file dialog and the
    /// `--load` startup flag).
    pub fn load_wav_from_path(&mut self, path: &std::path::Path) {
        match WavReader::from_file(path) {
            Ok(reader) => {
                tracing::info!(path = %path.display(), "WAV file loaded successfully");
                self.wav_reader = Some(reader);
                self.image_texture = None;
                self.last_decoded = None;
                // In-flight worker results now belong to the previous input
                self.decode_generation += 1;
                self.last_decode_error = None;
                // Pointer-keyed caches must not survive a buffer swap (ABA)
                self.waveform_panel.invalidate();
                self.spectrum_panel.invalidate();
                // Update audio state to Ready when WAV is loaded
                self.audio_state = AudioPlaybackState::Ready;
                // Clear any previous error and reset decode position
                self.error_message = None;
                self.last_decode_position = 0;
                // Cache sync markers and per-image ranges once per load.
                self.refresh_sync_positions();
                self.refresh_image_bounds();
            }
            Err(e) => {
                tracing::error!(path = %path.display(), error = %crate::error::error_chain(&e), "Failed to load WAV file");
                // Keep any previously loaded file fully usable — only the new
                // load failed. Reset state only when nothing was loaded.
                if self.wav_reader.is_none() {
                    self.audio_state = AudioPlaybackState::Uninitialized;
                    self.sync_positions.clear();
                    self.sync_scan_rx = None;
                }

                // Extract user-friendly error message
                self.error_message = Some(match e {
                    VoyagerError::Audio(audio_err) => audio_err.user_message(),
                    _ => format!("Failed to load audio file: {}", crate::error::error_chain(&e)),
                });
            }
        }
    }

    /// Apply one batch-progress message to panel state.
    fn apply_batch_message(&mut self, msg: BatchProgressMsg, ctx: &egui::Context) {
        match msg {
            BatchProgressMsg::ItemStatus(index, status) => {
                if let Some(item) = self.batch_panel.queue.get_mut(index) {
                    item.status = status;
                }
                ctx.request_repaint();
            }
            BatchProgressMsg::Progress(progress) => {
                self.batch_panel.progress = progress;
                ctx.request_repaint();
            }
            BatchProgressMsg::Error(error_msg) => {
                tracing::error!("Batch worker error: {}", error_msg);
                self.error_message = Some(format!("Batch error: {}", error_msg));
                self.batch_panel.is_processing = false;
                ctx.request_repaint();
            }
        }
    }

    /// Rebuild the displayed texture from the last decode result without
    /// re-running the decoder — used when a display-only option (the
    /// sync-margin crop) toggles.
    fn refresh_texture_from_last(&mut self, ctx: &egui::Context) {
        if let Some(result) = &self.last_decoded {
            let img = result.to_egui_image(self.crop_sync_margins);
            self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
        }
    }

    /// Decode the playhead's image (or window) on the UI thread. A `refresh`
    /// replaces a manual window preview once image bounds arrive, and leaves
    /// an unread error in place. A failed decode keeps the previous preview,
    /// and with it any full-frame refresh that preview is still due.
    fn handle_decode(&mut self, ctx: &egui::Context, refresh: bool) {
        // Decode the single image at the playhead when boundaries are known,
        // so a multi-image file shows one recognizable frame. When bounds are
        // not yet available (still scanning, or a tone-only file) decode a
        // bounded window at the playhead — never the whole, possibly
        // multi-minute, buffer, which would freeze the UI thread.
        let range = self.image_range_at(self.current_position_samples);
        let Some(reader) = &self.wav_reader else {
            self.error_message = Some("No audio file loaded".to_string());
            return;
        };
        let all = reader.get_samples(self.selected_channel);
        let (slice_start, samples, image_start): (usize, &[f32], Option<usize>) = match &range {
            Some(r) if r.end <= all.len() => (r.start, &all[r.clone()], Some(r.start)),
            _ => {
                let pos = self.current_position_samples.min(all.len().saturating_sub(1));
                let window = (reader.sample_rate as f64 * self.params.decode_window_secs) as usize;
                let end = pos.saturating_add(window.max(1)).min(all.len());
                (pos, &all[pos..end], None)
            }
        };
        let preroll_len = (reader.sample_rate as f64 * crate::sstv::DC_RESTORE_PREROLL_SECS) as usize;
        let preroll = &all[slice_start.saturating_sub(preroll_len)..slice_start];

        // A decode the user asked for starts from a clean status line. A
        // refresh is automatic, so it leaves an undismissed error alone.
        if !refresh {
            self.error_message = None;
        }

        // Perform decode with error handling using unified pipeline
        let pipeline = crate::pipeline::DecodingPipeline::new();
        match pipeline.process_with_preroll(preroll, samples, &self.params, reader.sample_rate) {
            Ok(mut result) => {
                result.segmented_image = image_start.is_some();
                self.pending_full_frame = (range.is_none() && self.image_bounds_rx.is_some()).then_some(WindowPreview::Manual);
                // An older live decode must not overwrite this manual preview,
                // and neither should a re-request of one a restart dropped.
                self.decode_generation += 1;
                self.pending_image_retry = false;
                self.restart_retry.on_new_request();
                tracing::info!(pixels = result.pixels.len(), "Decode completed successfully");
                let img = result.to_egui_image(self.crop_sync_margins);
                self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
                self.last_decoded = Some(result);
                // Only record the image as decoded when its slice was actually
                // used; otherwise the per-image playback trigger would skip
                // re-decoding it.
                self.last_decoded_image_start = image_start;
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "Decode failed");
                self.error_message = Some(format!("Decode failed: {e:#}"));
                // The previous preview stays on screen under the error. It no
                // longer counts as the playhead's image, so playback decodes
                // that image again.
                self.last_decoded_image_start = None;
            }
        }
    }

    /// Restart the decode worker after a crash or timeout, recording metrics.
    fn restart_worker(&mut self) {
        // Anything still queued (or finished but not yet polled) is lost. Only
        // work for the current generation is worth re-sending: a file load,
        // channel switch, or manual decode already superseded anything older.
        let dropped = self.decode_worker.newest_pending_generation() == Some(self.decode_generation);
        self.decode_worker.restart();
        self.metrics.record_worker_restart();
        self.restart_retry.on_restart(dropped);
    }

    /// Kick off a background scan of the selected channel for sync tones,
    /// caching the positions for the waveform markers. Called on file load
    /// and channel switch only; the full-file FFT scan can take seconds on
    /// real recordings, so it runs off the UI thread and the result is
    /// collected in `update()`.
    fn refresh_sync_positions(&mut self) {
        self.sync_positions.clear();
        self.sync_scan_rx = None;

        let Some(reader) = &self.wav_reader else {
            return;
        };
        let samples: Arc<[f32]> = match self.selected_channel {
            WaveformChannel::Left => Arc::clone(&reader.left_channel),
            WaveformChannel::Right => Arc::clone(&reader.right_channel),
        };
        if samples.is_empty() {
            return;
        }
        let sample_rate = reader.sample_rate;

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = Instant::now();
            let decoder = SstvDecoder::new();
            let positions = decoder.find_tone_regions(&samples, sample_rate);
            tracing::info!(
                count = positions.len(),
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Background sync scan completed"
            );
            // Receiver may have been replaced by a newer scan; ignore failure.
            let _ = tx.send(positions);
        });
        self.sync_scan_rx = Some(rx);
    }

    /// Find per-image sample ranges for the selected channel on a background
    /// thread (full-file segmentation is too slow to block the UI). The
    /// preview decodes one of these whole ranges at a time.
    fn refresh_image_bounds(&mut self) {
        self.image_bounds.clear();
        self.image_bounds_rx = None;
        self.pending_full_frame = None;
        self.last_decoded_image_start = None;
        self.pending_image_retry = false;
        self.restart_retry.on_new_request();

        let Some(reader) = &self.wav_reader else {
            return;
        };
        let samples: Arc<[f32]> = match self.selected_channel {
            WaveformChannel::Left => Arc::clone(&reader.left_channel),
            WaveformChannel::Right => Arc::clone(&reader.right_channel),
        };
        if samples.is_empty() {
            return;
        }
        let sample_rate = reader.sample_rate;

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = Instant::now();
            let params = crate::analysis::SegmentImagesParams::default();
            let ranges: Vec<std::ops::Range<usize>> = crate::analysis::find_image_bounds(&samples, sample_rate, &params)
                .into_iter()
                .map(|b| b.start_sample..b.end_sample)
                .collect();
            tracing::info!(
                count = ranges.len(),
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Background image-bounds scan completed"
            );
            let _ = tx.send(ranges);
        });
        self.image_bounds_rx = Some(rx);
    }

    /// The image range to preview for a playhead at `pos`: the image
    /// containing it, else the next image starting after it, else the last.
    /// `None` only while no image bounds are known (still scanning, or a file
    /// with no detected images — e.g. pure tone).
    fn image_range_at(&self, pos: usize) -> Option<std::ops::Range<usize>> {
        if let Some(r) = self.image_bounds.iter().find(|r| r.contains(&pos)) {
            return Some(r.clone());
        }
        self.image_bounds
            .iter()
            .find(|r| r.start >= pos)
            .or_else(|| self.image_bounds.last())
            .cloned()
    }

    /// Export the last decoded image as a PNG via a save dialog.
    fn handle_export(&mut self) {
        let Some(result) = &self.last_decoded else {
            self.error_message = Some("No decoded image to export".to_string());
            return;
        };

        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name("voyager_decode.png")
            .save_file()
        else {
            return;
        };

        match result.to_dynamic_image() {
            Ok(img) => {
                // Same published orientation as the preview
                // (`PipelineResult::to_egui_image`); the full frame is kept,
                // the sync-margin crop is display-only.
                let img = img.rotate90().fliph();
                if let Err(e) = img.save(&path) {
                    tracing::error!(path = %path.display(), error = %e, "Failed to save PNG");
                    self.error_message = Some(format!("Export failed: {}", e));
                } else {
                    tracing::info!(path = %path.display(), "Exported decoded image");
                    self.error_message = None;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to convert decoded pixels to image");
                self.error_message = Some(format!("Export failed: {}", e));
            }
        }
    }

    #[cfg(feature = "audio_playback")]
    fn ensure_audio_stream(&mut self) -> Option<&Mixer> {
        if self.audio_stream.is_none() {
            match OutputStreamBuilder::open_default_stream() {
                Ok(stream) => {
                    self.audio_stream = Some(stream);
                }
                Err(e) => {
                    tracing::error!("Failed to initialize audio stream: {}", e);
                    self.audio_state = AudioPlaybackState::Error(AudioError::StreamInitFailed);
                    return None;
                }
            }
        }

        self.audio_stream.as_ref().map(|stream| stream.mixer())
    }

    fn toggle_playback(&mut self) {
        #[cfg(feature = "audio_playback")]
        {
            match self.audio_state {
                AudioPlaybackState::Playing => {
                    // Pause playback
                    if let Some(sink) = &self.audio_sink {
                        sink.pause();
                        self.audio_state = AudioPlaybackState::Paused;
                        tracing::info!("Pausing playback");
                    } else {
                        tracing::warn!("Cannot pause: no audio sink available");
                        self.audio_state = AudioPlaybackState::Error(AudioError::SinkNotAvailable);
                    }
                }
                AudioPlaybackState::Paused => {
                    // Resume playback
                    if let Some(sink) = &self.audio_sink {
                        // The sink's get_pos persists across pause; the base
                        // offset is still valid, so no clock bookkeeping.
                        sink.play();
                        self.audio_state = AudioPlaybackState::Playing;
                        tracing::info!("Resuming playback");
                    } else {
                        // No sink means the user seeked while paused (the
                        // stale sink was dropped); rebuild from the current
                        // position so audio matches the visuals.
                        self.audio_state = AudioPlaybackState::Playing;
                        self.restart_audio_from_current_position();
                        tracing::info!("Resuming playback after paused seek");
                    }
                }
                AudioPlaybackState::Ready => {
                    // Start fresh playback
                    if self.wav_reader.is_none() {
                        self.error_message = Some("No audio file loaded".to_string());
                        return;
                    }

                    // Ensure audio stream is available
                    let stream = match self.ensure_audio_stream() {
                        Some(s) => s,
                        None => {
                            self.error_message = Some("Failed to initialize audio stream".to_string());
                            return;
                        }
                    };

                    // Create sink with the mixer
                    let sink = Sink::connect_new(stream);
                    if let Some(source) = self.make_buffer_source_from_current_position() {
                        sink.append(source);
                        sink.play();
                        self.audio_sink = Some(sink);
                        self.audio_state = AudioPlaybackState::Playing;
                        self.playback_base_samples = self.current_position_samples;
                        tracing::info!("Starting playback");
                    } else {
                        tracing::error!("No audio samples available to start playback");
                        self.error_message = Some("No audio samples available".to_string());
                    }
                }
                AudioPlaybackState::Uninitialized => {
                    self.error_message = Some("Audio not initialized - load a file first".to_string());
                }
                AudioPlaybackState::Error(_) => {
                    self.error_message = Some(format!("Cannot play: {}", self.audio_state));
                }
            }
        }

        #[cfg(not(feature = "audio_playback"))]
        {
            // Visual-only playback simulation
            match self.audio_state {
                AudioPlaybackState::Ready | AudioPlaybackState::Paused => {
                    self.audio_state = AudioPlaybackState::Playing;
                    self.playback_start_time = Some(Instant::now());
                    self.playback_start_position = self.current_position_samples;
                    tracing::info!("Starting visual playback");
                }
                AudioPlaybackState::Playing => {
                    self.audio_state = AudioPlaybackState::Paused;
                    tracing::info!("Pausing visual playback");
                }
                _ => {}
            }
        }
    }

    fn stop_playback(&mut self) {
        #[cfg(feature = "audio_playback")]
        {
            // Stop and drop the audio sink
            if let Some(sink) = self.audio_sink.take() {
                sink.stop();
            }
        }

        // Reset playback state
        if self.wav_reader.is_some() {
            self.audio_state = AudioPlaybackState::Ready;
        } else {
            self.audio_state = AudioPlaybackState::Uninitialized;
        }
        self.current_position_samples = 0;
        self.last_decode_position = 0;
        #[cfg(feature = "audio_playback")]
        {
            self.playback_base_samples = 0;
        }
        #[cfg(not(feature = "audio_playback"))]
        {
            self.playback_start_time = None;
            self.playback_start_position = 0;
        }
        tracing::info!("Stopping playback");
    }

    /// Enqueue a non-blocking decode request at the given sample position.
    ///
    /// # Non-Blocking Architecture
    ///
    /// **OLD approach (blocking):**
    /// ```ignore
    /// let pixels = self.video_decoder.decode(segment, &params, sample_rate)?;
    /// // UI frozen for 100-500ms during FFT + pixel processing
    /// ```
    ///
    /// **NEW approach (async via worker thread):**
    /// ```ignore
    /// let request = DecodeRequest { samples: Arc::clone(&buffer), ... };
    /// decode_tx.send(request);  // Returns immediately (microseconds)
    /// // Worker processes in background, UI stays at 60fps
    /// // Result arrives via decode_rx, polled in update()
    /// ```
    ///
    /// # Performance Impact
    /// - Decode latency: ~100-500ms (unchanged, runs in background)
    /// - UI frame time: <16ms (previously spiked to 100-500ms during decode)
    /// - Responsiveness: Immediate (no blocking operations)
    fn decode_at_position(&mut self, _ctx: &egui::Context, position: usize) {
        // Clear any armed retry up front: if this call bails out early (no
        // reader, position past EOF, too few samples) there is nothing to
        // retry, and if it reaches the worker the flag is re-armed below only
        // when the request is dropped. Keeps "retry armed" ⟺ "last attempt hit
        // the queue and was refused".
        self.pending_image_retry = false;
        self.restart_retry.on_new_request();
        if let Some(reader) = &self.wav_reader {
            // Get shared reference to samples (Arc enables zero-copy sharing with worker)
            let samples: Arc<[f32]> = match self.selected_channel {
                WaveformChannel::Left => Arc::clone(&reader.left_channel),
                WaveformChannel::Right => Arc::clone(&reader.right_channel),
            };

            // Bounds check - ensure we have samples and position is valid
            if samples.is_empty() {
                tracing::warn!("No samples available for decoding");
                return;
            }

            if position >= samples.len() {
                tracing::warn!(
                    position = position,
                    samples_len = samples.len(),
                    "Decode position out of bounds"
                );
                return;
            }

            let sample_rate = reader.sample_rate;
            // Decode the whole image at the playhead when boundaries are known,
            // so the live preview is a full frame rather than a 2 s slice.
            let range = self.image_range_at(position);

            // Ensure the decode spans enough samples to be meaningful. With a
            // known image range the decode covers the whole image
            // (`r.start..r.end`, always several seconds) even when the playhead
            // sits near EOF; only the windowed fallback is bounded by what is
            // left after the playhead. Gating unconditionally on
            // `samples.len() - position` would drop a legitimate final image
            // once the playhead entered its last 100 ms.
            let min_window_samples = (sample_rate as f64 * 0.1) as usize; // 100ms minimum
            let decode_span = match &range {
                Some(r) => r.end.saturating_sub(r.start),
                None => samples.len() - position,
            };
            if decode_span < min_window_samples {
                tracing::debug!(
                    span = decode_span,
                    min_required = min_window_samples,
                    "Insufficient samples for a decode window"
                );
                return;
            }

            let image_start = range.as_ref().map(|r| r.start);
            let windowed_during_scan = range.is_none() && self.image_bounds_rx.is_some();
            let accepted = self.decode_worker.request(
                samples,
                position,
                range,
                self.params,
                sample_rate,
                self.config.worker.max_queue_size,
                self.decode_generation,
            );
            // Only a request the worker took supersedes an earlier window
            // preview; an early return or refusal leaves that preview on
            // screen, still due its full-frame refresh. A window decoded
            // while the scan is running is itself due one.
            if accepted {
                self.pending_full_frame = windowed_during_scan.then_some(WindowPreview::Worker);
            }
            // Record the image we're on regardless of acceptance, so the
            // per-image playback trigger does not re-fire every frame. If the
            // request was dropped on a full queue, arm a retry instead of
            // leaving the image un-decoded: it is re-sent once a poll frees a
            // queue slot (see `update`), so the playhead's image is still shown
            // without spinning `decode_at_position` here on every repaint.
            // A refusal with free slots means the worker is gone; retrying
            // that would spin every frame until it is restarted.
            self.last_decoded_image_start = image_start;
            self.pending_image_retry = !accepted && self.decode_worker.pending() >= self.config.worker.max_queue_size;
        }
    }

    fn seek_to_next_sync(&mut self) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            if samples.is_empty() {
                tracing::warn!("No samples available for sync detection");
                return;
            }

            // Ensure current position is within bounds
            if self.current_position_samples >= samples.len() {
                tracing::warn!(
                    position = self.current_position_samples,
                    samples_len = samples.len(),
                    "Current position out of bounds, resetting to start"
                );
                self.current_position_samples = 0;
            }

            // Prefer the cached sync markers (the same ones drawn on the
            // waveform) so the jump matches what the user sees and costs
            // nothing; fall back to a live scan only while the background
            // marker scan hasn't finished yet.
            let min_jump = self.current_position_samples + (reader.sample_rate as usize / 100); // skip the marker we're on
            let next_sync = self.sync_positions.iter().copied().find(|&p| p > min_jump).or_else(|| {
                self.video_decoder
                    .find_next_tone_region(samples, self.current_position_samples, reader.sample_rate)
            });

            if let Some(sync_position) = next_sync {
                // Validate sync position
                if sync_position < samples.len() {
                    self.current_position_samples = sync_position;
                    tracing::info!(sync_position, "Seeking to next sync");

                    // If playing, restart audio from new position
                    #[cfg(feature = "audio_playback")]
                    self.restart_audio_from_current_position();

                    #[cfg(not(feature = "audio_playback"))]
                    if self.audio_state.is_playing() {
                        self.playback_start_time = Some(Instant::now());
                        self.playback_start_position = self.current_position_samples;
                    }
                } else {
                    tracing::warn!(
                        sync_position = sync_position,
                        samples_len = samples.len(),
                        "Sync position out of bounds, ignoring"
                    );
                }
            } else {
                tracing::info!("No more sync signals found");
            }
        }
    }

    #[cfg(feature = "audio_playback")]
    /// Create an AudioBufferSource from the current position in the selected channel (zero-copy)
    fn make_buffer_source_from_current_position(&self) -> Option<AudioBufferSource> {
        let reader = self.wav_reader.as_ref()?;

        // Get the Arc buffer for the selected channel
        let buffer = match self.selected_channel {
            WaveformChannel::Left => Arc::clone(&reader.left_channel),
            WaveformChannel::Right => Arc::clone(&reader.right_channel),
        };

        if self.current_position_samples >= buffer.len() {
            return None;
        }

        // Use Arc + offset instead of cloning - zero-copy seek!
        // AudioBufferSource::new validates parameters and returns Result
        AudioBufferSource::new(
            buffer,
            self.current_position_samples,
            reader.sample_rate,
            1, // Mono playback (we've already selected a channel)
        )
        .inspect_err(|e| {
            tracing::error!(
                error = %e,
                offset = self.current_position_samples,
                "Failed to create AudioBufferSource"
            );
        })
        .ok()
    }

    #[cfg(feature = "audio_playback")]
    /// Restart audio playback from the current position (used when seeking)
    fn restart_audio_from_current_position(&mut self) {
        if !self.audio_state.is_playing() {
            // Seek while paused: the existing sink still holds the pre-seek
            // offset. Drop it so resume rebuilds from the new position
            // instead of playing audio that diverges from the visuals.
            if self.audio_state == AudioPlaybackState::Paused {
                if let Some(sink) = self.audio_sink.take() {
                    sink.stop();
                }
            }
            return;
        }

        // Stop existing sink if present
        if let Some(sink) = self.audio_sink.take() {
            sink.stop();
        }

        // Ensure audio stream is available
        let stream = match self.ensure_audio_stream() {
            Some(s) => s,
            None => {
                self.audio_state = AudioPlaybackState::Error(AudioError::StreamInitFailed);
                return;
            }
        };

        // Create new sink with source from current position
        let sink = Sink::connect_new(stream);
        if let Some(source) = self.make_buffer_source_from_current_position() {
            sink.append(source);
            sink.play();
            self.audio_sink = Some(sink);
            self.playback_base_samples = self.current_position_samples;
        } else {
            tracing::error!("No audio samples available after seek");
        }
    }
}

impl VoyagerApp {
    /// Current playhead in samples, anchored to the audio device clock.
    #[cfg(feature = "audio_playback")]
    fn live_position(&self) -> Option<usize> {
        let sink = self.audio_sink.as_ref()?;
        let reader = self.wav_reader.as_ref()?;
        Some(crate::services::playback::position_samples(
            self.playback_base_samples,
            sink.get_pos(),
            reader.sample_rate,
        ))
    }

    /// Visual-only simulated playhead (no audio device to anchor to).
    #[cfg(not(feature = "audio_playback"))]
    fn live_position(&self) -> Option<usize> {
        let start_time = self.playback_start_time?;
        let reader = self.wav_reader.as_ref()?;
        let samples_elapsed = (start_time.elapsed().as_secs_f32() * reader.sample_rate as f32) as usize;
        Some(self.playback_start_position + samples_elapsed)
    }
}

impl eframe::App for VoyagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Batch Processing Logic ---
        // Check if user wants to start processing. A missing output dir only
        // skips the batch start — never the rest of the frame, because
        // returning out of update() would render a blank window.
        if self.batch_panel.is_processing && !self.batch_runner.is_running() {
            match self.batch_panel.output_dir.clone() {
                Some(output_dir) => {
                    let queue = self.batch_panel.queue.clone();
                    let params = DecoderParams {
                        mode: self.batch_panel.selected_mode,
                        ..self.params
                    };
                    let cancel_flag = self.batch_runner.start(queue, output_dir, params);
                    self.batch_panel.cancel_flag = Some(cancel_flag);
                    ctx.request_repaint();
                }
                None => {
                    self.batch_panel.is_processing = false;
                }
            }
        }

        if self.batch_panel.is_processing {
            for msg in self.batch_runner.poll() {
                self.apply_batch_message(msg, ctx);
            }
        }

        // Reap the batch worker whenever it has finished — deliberately
        // outside the is_processing gate, because the Error branch above
        // clears that flag and a stale running worker would otherwise block
        // the next Start click. The reap returns any final messages that
        // raced the finish check; apply them so the last item doesn't stay
        // stuck at "Processing".
        if let Some(remaining) = self.batch_runner.reap_if_finished() {
            for msg in remaining {
                self.apply_batch_message(msg, ctx);
            }
            self.batch_panel.cancel_flag = None;
            self.batch_panel.is_processing = false;
            ctx.request_repaint();
        }

        // Record frame time for performance metrics
        if let Some(start) = self.frame_start {
            self.metrics.record_frame_time(start.elapsed());
        }
        self.frame_start = Some(Instant::now());

        // Collect the background sync scan result, if one is in flight
        if let Some(rx) = &self.sync_scan_rx {
            match rx.try_recv() {
                Ok(positions) => {
                    self.sync_positions = positions;
                    self.sync_scan_rx = None;
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Keep polling at a low rate while the scan runs
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("Sync scan thread exited without a result");
                    self.sync_scan_rx = None;
                }
            }
        }

        // Collect the background image-bounds scan result, if one is in flight
        let mut refresh_preview = None;
        if let Some(rx) = &self.image_bounds_rx {
            match rx.try_recv() {
                Ok(ranges) => {
                    self.image_bounds = ranges;
                    self.image_bounds_rx = None;
                    refresh_preview = self.pending_full_frame.take();
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("Image-bounds scan thread exited without a result");
                    self.image_bounds_rx = None;
                    self.pending_full_frame = None;
                }
            }
        }
        if let Some(preview) = refresh_preview {
            if self.image_range_at(self.current_position_samples).is_some() {
                match preview {
                    WindowPreview::Manual => self.handle_decode(ctx, true),
                    WindowPreview::Worker => self.decode_at_position(ctx, self.current_position_samples),
                }
            }
        }

        // Poll for decode results from background worker (non-blocking)
        for decode_result in self.decode_worker.poll() {
            let DecodeResult {
                id: _,
                generation,
                result: pipeline_result,
                decode_duration,
                error,
            } = decode_result;

            let success = error.is_none();
            let pixel_count = pipeline_result.as_ref().map(|r| r.pixels.len()).unwrap_or(0);

            // Log performance metrics and record in metrics system
            if success {
                tracing::debug!(
                    duration_ms = decode_duration.as_millis(),
                    pixels = pixel_count,
                    "Decode completed successfully"
                );
            } else {
                tracing::warn!(
                    duration_ms = decode_duration.as_millis(),
                    error = error.as_deref().unwrap_or("unknown"),
                    "Decode failed"
                );
            }

            // Record metrics (track both success and failure)
            self.metrics.record_decode(decode_duration, pixel_count, success);
            self.restart_retry.on_result();

            // A file load, channel switch, or manual decode superseded these results.
            if generation != self.decode_generation {
                tracing::debug!(generation, current = self.decode_generation, "Dropping stale decode result");
                continue;
            }

            // Handle error or update texture. Identical consecutive errors are
            // not re-raised — live decode retries every interval and would
            // otherwise make the dismiss button useless. The message carries
            // the cause, so a different failure does raise a new banner.
            if let Some(err_msg) = error {
                let msg = format!("Decode failed: {}", err_msg);
                if self.last_decode_error.as_deref() != Some(msg.as_str()) {
                    self.error_message = Some(msg.clone());
                    self.last_decode_error = Some(msg);
                }
            } else if let Some(res) = pipeline_result {
                self.last_decode_error = None;
                let img = res.to_egui_image(self.crop_sync_margins);
                self.image_texture = Some(ctx.load_texture("decoded_realtime", img, Default::default()));
                self.last_decoded = Some(res);
            }
        }

        // Check worker health and restart if needed. This runs after the
        // poll above on purpose: the health check only sees results that
        // have been collected, so checking first would restart a healthy
        // worker whenever the UI thread stalled past the timeout with a
        // decode in flight (a blocking file dialog, a hidden window).
        let max_unresponsive = Duration::from_millis(self.config.worker.max_unresponsive_ms);
        if self.config.worker.auto_restart_on_panic && !self.decode_worker.is_healthy(max_unresponsive) {
            self.error_message = Some("Worker thread crashed or timed out, restarting...".to_string());
            self.restart_worker();
        }
        // Re-send a decode the restart dropped, playing or paused. One that
        // is waiting out the retry interval needs a repaint to come due,
        // since a paused, idle UI stops repainting.
        if self.restart_retry.take_due(Instant::now()) {
            self.decode_at_position(ctx, self.current_position_samples);
            self.restart_retry.mark_retried();
        } else if self.restart_retry.is_pending() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Re-send a live decode that was dropped on a full queue, but only now
        // that `poll` above may have freed a slot — not every frame while the
        // worker is busy. `decode_at_position` clears the retry flag on entry
        // and re-arms it only if this attempt is dropped again, so this cannot
        // spin: with a free slot the request is accepted and the flag clears.
        if self.pending_image_retry && self.decode_worker.pending() < self.config.worker.max_queue_size {
            self.decode_at_position(ctx, self.current_position_samples);
        }

        // Keep the frame loop alive while a decode is in flight so its result
        // is polled within a frame. A paused, idle UI otherwise stops
        // repainting, leaving a finished result uncollected and the preview
        // stale until the next input event.
        if self.decode_worker.pending() > 0 {
            ctx.request_repaint_after(Duration::from_millis(33));
        }

        // Update playback position if playing. The position comes from the
        // audio device clock (sink.get_pos), not a UI-frame timer, so it
        // cannot drift under frame-rate jitter; the live decode window is
        // anchored to the same value.
        if self.audio_state.is_playing() {
            // A drained sink means the source ran out even if rounding in the
            // position math never quite reaches total_samples — without this,
            // playback can stall in Playing forever at end of file.
            #[cfg(feature = "audio_playback")]
            let sink_drained = self.audio_sink.as_ref().map(|s| s.empty()).unwrap_or(false);
            #[cfg(not(feature = "audio_playback"))]
            let sink_drained = false;

            if let (Some(new_position), Some(total_samples), Some(sample_rate)) = (
                self.live_position(),
                self.wav_reader.as_ref().map(|r| r.left_channel.len()),
                self.wav_reader.as_ref().map(|r| r.sample_rate),
            ) {
                if new_position >= total_samples || sink_drained {
                    // Reached end of audio, stop playback
                    self.stop_playback();
                } else {
                    self.current_position_samples = new_position;

                    // Real-time decoding. With image boundaries known, decode
                    // once per image (when the playhead crosses into a new
                    // frame); otherwise fall back to a fixed time interval to
                    // avoid flooding the worker.
                    let trigger = match self.image_range_at(new_position) {
                        Some(r) => Some(r.start) != self.last_decoded_image_start,
                        None => {
                            let threshold =
                                (sample_rate as f32 * (self.config.worker.decode_interval_ms as f32 / 1000.0)) as usize;
                            new_position.abs_diff(self.last_decode_position) >= threshold
                        }
                    };
                    if trigger {
                        self.decode_at_position(ctx, new_position);
                        self.last_decode_position = new_position;
                    }
                }
            }
        }

        // Request continuous repaints during playback for position updates
        if self.audio_state.is_playing() {
            ctx.request_repaint();
        }

        // Draw Batch Panel (floating window)
        self.batch_panel.draw(ctx);

        // --- Header bar ---
        egui::TopBottomPanel::top("header_bar")
            .frame(theme::strip_frame())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Voyager Golden Record Explorer")
                            .size(19.0)
                            .strong()
                            .color(theme::TEXT_BRIGHT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Interactive image recovery from the Golden Record audio")
                            .size(12.0)
                            .color(theme::TEXT_MUTED),
                    );
                });
            });

        // --- Transport bar ---
        egui::TopBottomPanel::top("transport_bar")
            .frame(theme::strip_frame())
            .show(ctx, |ui| {
                let (current_secs, total_secs) = match &self.wav_reader {
                    Some(reader) => {
                        let rate = reader.sample_rate.max(1) as f64;
                        (
                            self.current_position_samples as f64 / rate,
                            reader.left_channel.len() as f64 / rate,
                        )
                    }
                    None => (0.0, 0.0),
                };

                if let Some(action) = ControlsPanel::draw(
                    ui,
                    self.audio_state.is_playing(),
                    self.wav_reader.is_some(),
                    current_secs,
                    total_secs,
                ) {
                    match action {
                        ControlAction::OpenWav => self.handle_load_wav(),
                        ControlAction::TogglePlayback => self.toggle_playback(),
                        ControlAction::StopPlayback => self.stop_playback(),
                        ControlAction::SeekToNextSync => self.seek_to_next_sync(),
                    }
                }
            });

        // --- Waveform strip ---
        egui::TopBottomPanel::top("waveform_strip")
            .exact_height(200.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                theme::section_label(ui, "Waveform");
                if let Some(new_pos) = self.waveform_panel.draw(
                    ui,
                    &self.wav_reader,
                    self.selected_channel,
                    self.current_position_samples,
                    &mut self.waveform_hover_position,
                    &self.sync_positions,
                ) {
                    self.current_position_samples = new_pos;

                    #[cfg(feature = "audio_playback")]
                    self.restart_audio_from_current_position();

                    #[cfg(not(feature = "audio_playback"))]
                    if self.audio_state.is_playing() {
                        self.playback_start_time = Some(Instant::now());
                        self.playback_start_position = self.current_position_samples;
                    }

                    // Trigger decode on manual seek
                    self.decode_at_position(ctx, self.current_position_samples);
                    self.last_decode_position = self.current_position_samples;
                }
            });

        // --- Status bar ---
        egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                let mut dismiss_error = false;
                ui.horizontal(|ui| {
                    let status_color = if self.audio_state.is_playing() {
                        theme::ACCENT
                    } else if self.audio_state.is_error() {
                        theme::ERROR
                    } else {
                        theme::TEXT_MUTED
                    };
                    ui.colored_label(
                        status_color,
                        egui::RichText::new(format!(
                            "{} {}",
                            self.audio_state.status_icon(),
                            self.audio_state.status_message()
                        ))
                        .size(12.0),
                    );

                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("Worker: {} pending", self.decode_worker.pending()))
                            .size(12.0)
                            .color(theme::TEXT_MUTED),
                    );

                    if let Some(reader) = &self.wav_reader {
                        ui.separator();
                        let duration_secs = reader.left_channel.len() as f32 / reader.sample_rate as f32;
                        ui.label(
                            egui::RichText::new(format!(
                                "{} Hz · {} · {}",
                                reader.sample_rate,
                                if reader.channels == 1 { "mono" } else { "stereo" },
                                format_duration(duration_secs)
                            ))
                            .size(12.0)
                            .color(theme::TEXT_MUTED),
                        );
                    }

                    ui.separator();
                    let summary = self.metrics.summary();
                    ui.label(
                        egui::RichText::new(format!(
                            "frame p99 {:.0} ms · decode p50 {:.0} ms",
                            summary.frame_p99_ms, summary.decode_p50_ms
                        ))
                        .size(12.0)
                        .color(theme::TEXT_MUTED),
                    );

                    if let Some(error) = self.error_message.clone() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("✖").clicked() {
                                dismiss_error = true;
                            }
                            ui.colored_label(theme::ERROR, egui::RichText::new(error).size(12.0));
                        });
                    }
                });
                if dismiss_error {
                    self.error_message = None;
                }
            });

        // --- Left column: spectrum analyzer + signal info ---
        egui::SidePanel::left("analysis_panel")
            .exact_width(280.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                let mut peak = None;
                theme::panel_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::section_label(ui, "Spectrum Analyzer");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let toggle_text = if self.spectrum_panel.visible { "Hide" } else { "Show" };
                            if ui.small_button(toggle_text).clicked() {
                                self.spectrum_panel.visible = !self.spectrum_panel.visible;
                            }
                        });
                    });
                    if self.spectrum_panel.visible {
                        peak =
                            self.spectrum_panel
                                .draw(ui, &self.wav_reader, self.current_position_samples, self.selected_channel);
                    }
                });

                ui.add_space(8.0);

                theme::panel_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    theme::section_label(ui, "Signal");
                    ui.add_space(2.0);
                    if let Some(reader) = &self.wav_reader {
                        let duration_secs = reader.left_channel.len() as f32 / reader.sample_rate as f32;
                        theme::key_value(ui, "Sample rate", &format!("{} Hz", reader.sample_rate));
                        theme::key_value(ui, "Channels", if reader.channels == 1 { "mono" } else { "stereo" });
                        theme::key_value(ui, "Duration", &format_duration(duration_secs));
                        let sync_marks = if self.sync_scan_rx.is_some() {
                            "scanning…".to_string()
                        } else {
                            self.sync_positions.len().to_string()
                        };
                        theme::key_value(ui, "Sync marks", &sync_marks);
                        let images = if self.image_bounds_rx.is_some() {
                            "scanning…".to_string()
                        } else {
                            self.image_bounds.len().to_string()
                        };
                        theme::key_value(ui, "Detected images", &images);
                        if let Some((freq, mag)) = peak {
                            theme::key_value(ui, "Dominant", &format!("{:.0} Hz ({:.1})", freq, mag));
                        }
                    } else {
                        ui.label(egui::RichText::new("No audio loaded").size(12.0).color(theme::TEXT_MUTED));
                    }
                });
            });

        // --- Right column: decode controls + export ---
        egui::SidePanel::right("decode_controls_panel")
            .exact_width(280.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    theme::section_label(ui, "Decode Controls");
                    ui.add_space(2.0);
                    let params_before = self.params;

                    egui::Grid::new("decode_params_grid")
                        .num_columns(2)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Line (ms)").size(12.0).color(theme::TEXT_MUTED));
                            ui.add(
                                egui::DragValue::new(&mut self.params.line_duration_ms)
                                    .range(1..=100)
                                    .speed(0.01),
                            );
                            ui.end_row();

                            ui.label(egui::RichText::new("Gamma").size(12.0).color(theme::TEXT_MUTED));
                            ui.add(egui::Slider::new(&mut self.params.gamma, 0.2..=3.0));
                            ui.end_row();

                            ui.label(egui::RichText::new("Mode").size(12.0).color(theme::TEXT_MUTED));
                            egui::ComboBox::from_id_salt("decode_mode_combo")
                                .selected_text(match self.params.mode {
                                    DecoderMode::Grayscale => "Grayscale",
                                    DecoderMode::PseudoColor => "PseudoColor",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.params.mode, DecoderMode::Grayscale, "Grayscale");
                                    ui.selectable_value(&mut self.params.mode, DecoderMode::PseudoColor, "PseudoColor");
                                });
                            ui.end_row();

                            ui.label(egui::RichText::new("Channel").size(12.0).color(theme::TEXT_MUTED));
                            let previous_channel = self.selected_channel;
                            egui::ComboBox::from_id_salt("channel_combo")
                                .selected_text(match self.selected_channel {
                                    WaveformChannel::Left => "Left",
                                    WaveformChannel::Right => "Right",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.selected_channel, WaveformChannel::Left, "Left");
                                    ui.selectable_value(&mut self.selected_channel, WaveformChannel::Right, "Right");
                                });
                            if self.selected_channel != previous_channel {
                                // Sync markers and image ranges are
                                // channel-specific; rescan once.
                                self.refresh_sync_positions();
                                self.refresh_image_bounds();
                                // In-flight decode results are for the old channel
                                self.decode_generation += 1;
                                // The sink was built from the old channel's
                                // buffer; rebuild so audio matches the decode.
                                #[cfg(feature = "audio_playback")]
                                self.restart_audio_from_current_position();
                            }
                            ui.end_row();
                        });

                    ui.checkbox(&mut self.params.invert, "Invert");
                    ui.checkbox(&mut self.params.sync_lock, "Sync lock");
                    ui.checkbox(&mut self.params.dc_restore, "DC restore")
                        .on_hover_text("Compensate the recording's AC-coupling droop with an inverse high-pass filter");
                    if self.params != params_before {
                        // The live preview decodes once per image; forget the
                        // current one so playback re-decodes it with the new
                        // parameters instead of waiting for the next image.
                        self.last_decoded_image_start = None;
                    }
                    if ui
                        .checkbox(&mut self.crop_sync_margins, "Crop sync margins")
                        .on_hover_text("Hide the decoder's per-line sync/retrace and leader borders")
                        .changed()
                    {
                        // Display-only: re-skin the cached decode, no re-decode.
                        self.refresh_texture_from_last(ctx);
                    }
                });

                ui.add_space(8.0);

                theme::panel_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    theme::section_label(ui, "Export");
                    ui.add_space(2.0);
                    let can_export = self.last_decoded.is_some();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export Current Image…"))
                        .clicked()
                    {
                        self.handle_export();
                    }
                    if ui.button("Batch…").clicked() {
                        self.batch_panel.visible = !self.batch_panel.visible;
                    }
                });
            });

        // --- Center: decoded image ---
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                theme::panel_frame().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        theme::section_label(ui, "Image Decode");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Decode").clicked() {
                                self.handle_decode(ctx, false);
                            }
                            let lines = self.last_decoded.as_ref().map(|r| r.height).unwrap_or(0);
                            ui.label(
                                egui::RichText::new(format!("Lines: {}", lines))
                                    .size(12.0)
                                    .monospace()
                                    .color(theme::TEXT_BRIGHT),
                            );
                        });
                    });
                    ui.add_space(4.0);

                    // Fit the decoded image to the remaining panel rectangle,
                    // preserving aspect ratio (scaling up or down). The size is
                    // captured here, before any scroll area, because inside a
                    // vertical scroll area `available_height` is unbounded.
                    let avail = ui.available_size();
                    ui.centered_and_justified(|ui| {
                        if let Some(texture) = &self.image_texture {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(avail).maintain_aspect_ratio(true));
                        } else {
                            ui.label(
                                egui::RichText::new("No image decoded yet — load a WAV and press Decode, or play to decode live")
                                    .color(theme::TEXT_MUTED),
                            );
                        }
                    });
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::{RestartRetry, RESTART_RETRY_INTERVAL};
    use std::time::{Duration, Instant};

    #[test]
    fn dropped_decode_is_retried_once() {
        let mut retry = RestartRetry::default();
        let now = Instant::now();

        retry.on_restart(true);
        assert!(retry.take_due(now));
        assert!(!retry.take_due(now), "one restart yields one retry");
        retry.mark_retried();

        // The retry took the worker down too: give up rather than loop.
        retry.on_restart(true);
        assert!(!retry.is_pending());
        assert!(!retry.take_due(now + RESTART_RETRY_INTERVAL));
    }

    #[test]
    fn restart_that_dropped_nothing_is_not_retried() {
        let mut retry = RestartRetry::default();
        retry.on_restart(false);
        assert!(!retry.is_pending());
        assert!(!retry.take_due(Instant::now()));
    }

    #[test]
    fn retry_inside_the_interval_waits_for_it() {
        let mut retry = RestartRetry::default();
        let first = Instant::now();
        retry.on_restart(true);
        assert!(retry.take_due(first));
        retry.mark_retried();
        retry.on_result();

        // A later decode is dropped by a second restart inside the interval.
        retry.on_new_request();
        retry.on_restart(true);
        let expiry = first + RESTART_RETRY_INTERVAL;
        assert!(!retry.take_due(expiry - Duration::from_millis(1)));
        assert!(retry.is_pending());
        assert!(retry.take_due(expiry));
        assert!(!retry.take_due(expiry + Duration::from_millis(1)));
    }

    #[test]
    fn completed_decode_makes_the_next_failure_retryable() {
        let mut retry = RestartRetry::default();
        let now = Instant::now();
        retry.on_restart(true);
        assert!(retry.take_due(now));
        retry.mark_retried();

        retry.on_result();
        retry.on_restart(true);
        assert!(retry.is_pending());
    }

    #[test]
    fn new_request_supersedes_a_waiting_retry() {
        let mut retry = RestartRetry::default();
        retry.on_restart(true);
        retry.on_new_request();
        assert!(!retry.take_due(Instant::now()));
    }
}
