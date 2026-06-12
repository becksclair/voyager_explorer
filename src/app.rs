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

pub struct VoyagerApp {
    // Configuration
    config: AppConfig,

    // Audio data
    wav_reader: Option<WavReader>,
    video_decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<PipelineResult>,
    selected_channel: WaveformChannel,
    /// Sync-tone positions cached at file load / channel switch; rendered as
    /// amber markers in the waveform strip. Never recomputed per frame.
    sync_positions: Vec<usize>,
    /// Receiver for an in-flight background sync scan. The full-file scan can
    /// take seconds on real Golden Record audio, so it must not block the UI;
    /// replacing the receiver cancels delivery from a stale scan.
    sync_scan_rx: Option<std::sync::mpsc::Receiver<Vec<usize>>>,

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
    /// Input-state generation: bumped on file load and channel switch so
    /// stale worker results can be recognized and dropped.
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
            selected_channel: WaveformChannel::Left,
            sync_positions: Vec::new(),
            sync_scan_rx: None,
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
                // Cache sync markers once per load (not per frame)
                self.refresh_sync_positions();
            }
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to load WAV file");
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
                    _ => format!("Failed to load audio file: {}", e),
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

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            // Clear any previous errors
            self.error_message = None;

            // Perform decode with error handling using unified pipeline
            let pipeline = crate::pipeline::DecodingPipeline::new();
            match pipeline.process(samples, &self.params, reader.sample_rate) {
                Ok(result) => {
                    tracing::info!(pixels = result.pixels.len(), "Decode completed successfully");
                    let img = result.to_egui_image();
                    self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
                    self.last_decoded = Some(result);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Decode failed");
                    self.error_message = Some(format!("Decode failed: {}", e));
                    self.image_texture = None;
                    self.last_decoded = None;
                }
            }
        } else {
            self.error_message = Some("No audio file loaded".to_string());
        }
    }

    /// Restart the decode worker after a crash or timeout, recording metrics.
    fn restart_worker(&mut self) {
        self.decode_worker.restart();
        self.metrics.record_worker_restart();
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

            // Ensure we have enough samples for a meaningful decode window
            let min_window_samples = (reader.sample_rate as f64 * 0.1) as usize; // 100ms minimum
            let remaining_samples = samples.len() - position;
            if remaining_samples < min_window_samples {
                tracing::debug!(
                    remaining = remaining_samples,
                    min_required = min_window_samples,
                    "Insufficient samples remaining for decode window"
                );
                return;
            }

            let sample_rate = reader.sample_rate;
            self.decode_worker.request(
                samples,
                position,
                self.params,
                sample_rate,
                self.config.worker.max_queue_size,
                self.decode_generation,
            );
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
                    let mode = self.batch_panel.selected_mode;
                    let cancel_flag = self.batch_runner.start(queue, output_dir, mode);
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

        // Check worker health and restart if needed
        let max_unresponsive = Duration::from_millis(self.config.worker.max_unresponsive_ms);
        if self.config.worker.auto_restart_on_panic && !self.decode_worker.is_healthy(max_unresponsive) {
            self.error_message = Some("Worker thread crashed or timed out, restarting...".to_string());
            self.restart_worker();
        }

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

            // Results from a previous file/channel generation are stale:
            // displaying (or exporting) them would attribute old audio's
            // pixels to the current input.
            if generation != self.decode_generation {
                tracing::debug!(generation, current = self.decode_generation, "Dropping stale decode result");
                continue;
            }

            // Handle error or update texture. Identical consecutive errors are
            // not re-raised — live decode retries every interval and would
            // otherwise make the dismiss button useless.
            if let Some(err_msg) = error {
                let msg = format!("Decode failed: {}", err_msg);
                if self.last_decode_error.as_deref() != Some(msg.as_str()) {
                    self.error_message = Some(msg.clone());
                    self.last_decode_error = Some(msg);
                }
            } else if let Some(res) = pipeline_result {
                self.last_decode_error = None;
                let img = res.to_egui_image();
                self.image_texture = Some(ctx.load_texture("decoded_realtime", img, Default::default()));
                self.last_decoded = Some(res);
            }
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

                    // Real-time decoding: decode from current position only if significant change
                    // Decode at configurable interval to avoid flooding worker thread
                    let decode_threshold_samples =
                        (sample_rate as f32 * (self.config.worker.decode_interval_ms as f32 / 1000.0)) as usize;
                    let position_change = new_position.abs_diff(self.last_decode_position);

                    if position_change >= decode_threshold_samples {
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
                                // Sync markers are channel-specific; rescan once.
                                self.refresh_sync_positions();
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
                                self.handle_decode(ctx);
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

                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        if let Some(texture) = &self.image_texture {
                            ui.add(egui::Image::new(texture).max_width(ui.available_width()));
                        } else {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new(
                                        "No image decoded yet — load a WAV and press Decode, or play to decode live",
                                    )
                                    .color(theme::TEXT_MUTED),
                                );
                            });
                        }
                    });
                });
            });
    }
}
