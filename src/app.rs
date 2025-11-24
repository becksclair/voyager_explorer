use crate::audio::{WavReader, WaveformChannel};
use crate::audio_state::AudioPlaybackState;
use crate::config::AppConfig;
use crate::error::VoyagerError;
use crate::metrics::AppMetrics;
#[cfg(feature = "audio_playback")]
use crate::services::audio::AudioBufferSource;
use crate::services::decoder::{spawn_decode_worker, DecodeRequest, DecodeResult};
use crate::sstv::{DecoderMode, DecoderParams, SstvDecoder};
use crate::ui::batch::{BatchPanel, BatchStatus};
use crate::ui::controls::{ControlAction, ControlsPanel};
use crate::ui::spectrum::SpectrumPanel;
use crate::ui::waveform::WaveformPanel;
use crate::utils::format_duration;
use eframe::egui;
use egui::TextureHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(feature = "audio_playback")]
use crate::audio_state::AudioError;

#[cfg(feature = "audio_playback")]
use rodio::{OutputStream, OutputStreamBuilder, Sink};

#[cfg(feature = "audio_playback")]
use rodio::mixer::Mixer;

// Batch progress messages
enum BatchProgressMsg {
    ItemStatus(usize, BatchStatus),
    Progress(f32),
}

pub struct VoyagerApp {
    // Configuration
    config: AppConfig,

    // Audio data
    wav_reader: Option<WavReader>,
    video_decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<Vec<u8>>,
    selected_channel: WaveformChannel,

    // Audio playback state
    audio_state: AudioPlaybackState,
    #[cfg(feature = "audio_playback")]
    audio_stream: Option<OutputStream>,
    #[cfg(feature = "audio_playback")]
    audio_sink: Option<Sink>,
    current_position_samples: usize,
    last_decode_position: usize,
    waveform_hover_position: Option<f32>,
    playback_start_time: Option<Instant>,

    // Background decoding worker
    decode_tx: Option<Sender<DecodeRequest>>,
    decode_rx: Option<Receiver<DecodeResult>>,
    next_decode_id: u64,
    pending_decode_requests: usize,
    worker_handle: Option<JoinHandle<()>>,
    worker_last_response: Instant,

    // Metrics and errors
    metrics: AppMetrics,
    error_message: Option<String>,
    frame_start: Option<Instant>,

    // Signal Analysis
    spectrum_panel: SpectrumPanel,

    // Batch Processing
    batch_panel: BatchPanel,
    batch_worker: Option<JoinHandle<()>>,
    batch_rx: Option<Receiver<BatchProgressMsg>>,
    batch_cancel: Option<Arc<AtomicBool>>,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        // Spawn background decoding worker
        let (decode_tx, decode_rx, worker_handle) = spawn_decode_worker();

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
            threshold: config.decoder.default_threshold,
            decode_window_secs: config.decoder.decode_window_secs as f64,
            mode: DecoderMode::BinaryGrayscale,
        };

        Self {
            config: config.clone(),
            wav_reader: None,
            video_decoder: SstvDecoder::new(),
            image_texture: None,
            params,
            last_decoded: None,
            selected_channel: WaveformChannel::Left,
            audio_state: AudioPlaybackState::Uninitialized,
            #[cfg(feature = "audio_playback")]
            audio_stream: None,
            #[cfg(feature = "audio_playback")]
            audio_sink: None,
            current_position_samples: 0,
            last_decode_position: 0,
            waveform_hover_position: None,
            playback_start_time: None,
            decode_tx: Some(decode_tx),
            decode_rx: Some(decode_rx),
            next_decode_id: 0,
            pending_decode_requests: 0,
            worker_handle: Some(worker_handle),
            worker_last_response: Instant::now(),
            metrics: AppMetrics::new(),
            error_message: None,
            frame_start: None,
            spectrum_panel: SpectrumPanel::default(),
            batch_panel: BatchPanel::default(),
            batch_worker: None,
            batch_rx: None,
            batch_cancel: None,
        }
    }
}

impl VoyagerApp {
    fn handle_load_wav(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .pick_file()
        {
            match WavReader::from_file(&path) {
                Ok(reader) => {
                    tracing::info!(path = %path.display(), "WAV file loaded successfully");
                    self.wav_reader = Some(reader);
                    self.image_texture = None;
                    self.last_decoded = None;
                    // Update audio state to Ready when WAV is loaded
                    self.audio_state = AudioPlaybackState::Ready;
                    // Clear any previous error and reset decode position
                    self.error_message = None;
                    self.last_decode_position = 0;
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to load WAV file");
                    self.audio_state = AudioPlaybackState::Uninitialized;

                    // Extract user-friendly error message
                    self.error_message = Some(match e {
                        VoyagerError::Audio(audio_err) => audio_err.user_message(),
                        _ => format!("Failed to load audio file: {}", e),
                    });
                }
            }
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            // Clear any previous errors
            self.error_message = None;

            // Detect sync presence
            let sync_detected = self
                .video_decoder
                .detect_sync(samples.to_vec(), reader.sample_rate);
            tracing::debug!(sync_detected, "Sync detection completed");

            // Perform decode with error handling using unified pipeline
            let pipeline = crate::pipeline::DecodingPipeline::new();
            match pipeline.process(samples, &self.params, reader.sample_rate) {
                Ok(result) => {
                    tracing::info!(
                        pixels = result.pixels.len(),
                        "Decode completed successfully"
                    );
                    let img = result.to_egui_image();
                    self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
                    self.last_decoded = Some(result.pixels);
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

    /// Check if the worker thread is healthy and responsive.
    ///
    /// Returns true if the worker is healthy, false if it needs restart.
    ///
    /// Health checks:
    /// 1. Thread has not panicked (checked via JoinHandle::is_finished())
    /// 2. No response timeout (worker_last_response within max_unresponsive_ms)
    fn check_worker_health(&mut self) -> bool {
        let Some(handle) = &self.worker_handle else {
            tracing::warn!("Worker handle is None, needs restart");
            return false;
        };

        // Check if thread has panicked or exited
        if handle.is_finished() {
            tracing::error!("Worker thread has exited/panicked, needs restart");
            return false;
        }

        // Only check timeout if we have pending requests
        if self.pending_decode_requests > 0 {
            // Check for response timeout
            let elapsed = self.worker_last_response.elapsed();
            let timeout_threshold = Duration::from_millis(self.config.worker.max_unresponsive_ms);

            if elapsed > timeout_threshold {
                tracing::warn!(
                    elapsed_ms = elapsed.as_millis(),
                    threshold_ms = timeout_threshold.as_millis(),
                    "Worker thread unresponsive, needs restart"
                );
                return false;
            }
        }

        true
    }

    /// Restart the worker thread after a crash or timeout.
    ///
    /// This recreates the channels and spawns a new worker thread.
    /// Any pending decode requests in the old channels are lost.
    fn restart_worker(&mut self) {
        tracing::warn!("Restarting worker thread");

        // Drop old channels and handle to clean up resources
        self.decode_tx = None;
        self.decode_rx = None;
        self.worker_handle = None;

        // Spawn new worker with fresh channels
        let (decode_tx, decode_rx, worker_handle) = spawn_decode_worker();

        self.decode_tx = Some(decode_tx);
        self.decode_rx = Some(decode_rx);
        self.worker_handle = Some(worker_handle);
        self.pending_decode_requests = 0;
        self.worker_last_response = Instant::now();

        // Record restart in metrics
        self.metrics.record_worker_restart();

        tracing::info!("Worker thread restarted successfully");
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
                        sink.play();
                        self.audio_state = AudioPlaybackState::Playing;
                        self.playback_start_time = Some(Instant::now());
                        tracing::info!("Resuming playback");
                    } else {
                        tracing::warn!("Cannot resume: no audio sink available");
                        self.audio_state = AudioPlaybackState::Error(AudioError::SinkNotAvailable);
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
                            self.error_message =
                                Some("Failed to initialize audio stream".to_string());
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
                        self.playback_start_time = Some(Instant::now());
                        tracing::info!("Starting playback");
                    } else {
                        tracing::error!("No audio samples available to start playback");
                        self.error_message = Some("No audio samples available".to_string());
                    }
                }
                AudioPlaybackState::Uninitialized => {
                    self.error_message =
                        Some("Audio not initialized - load a file first".to_string());
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
        self.playback_start_time = None;
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
            if let Some(decode_tx) = &self.decode_tx {
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

                // Create decode request (all fields copied/cloned, O(1) operation)
                let request = DecodeRequest {
                    id: self.next_decode_id,
                    samples,
                    start_offset: position,
                    params: self.params,
                    sample_rate: reader.sample_rate,
                };

                self.next_decode_id += 1;
                self.pending_decode_requests += 1;

                // Send to worker thread (non-blocking, returns immediately)
                // If worker is busy, request queues up in channel
                if decode_tx.send(request).is_err() {
                    tracing::warn!("Decode worker thread has terminated");
                    self.pending_decode_requests = self.pending_decode_requests.saturating_sub(1);
                }
            }
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

            let next_sync = self.video_decoder.find_next_sync(
                samples,
                self.current_position_samples,
                reader.sample_rate,
            );

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
            self.playback_start_time = Some(Instant::now());
        } else {
            tracing::error!("No audio samples available after seek");
        }
    }
}

impl Drop for VoyagerApp {
    fn drop(&mut self) {
        // Drop channels first to signal worker thread to shut down
        self.decode_tx = None;
        self.decode_rx = None;

        // Join worker thread to prevent panic on shutdown
        if let Some(handle) = self.worker_handle.take() {
            if let Err(e) = handle.join() {
                tracing::error!("Worker thread panicked on shutdown: {:?}", e);
            }
        }

        // Join batch worker thread if active
        if let Some(handle) = self.batch_worker.take() {
            // Signal cancellation
            if let Some(flag) = &self.batch_cancel {
                flag.store(true, Ordering::Relaxed);
            }
            // Drop the receiver to potentially signal the sender
            self.batch_rx = None;
            if let Err(e) = handle.join() {
                tracing::error!("Batch worker thread panicked on shutdown: {:?}", e);
            }
        }
    }
}

impl eframe::App for VoyagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Batch Processing Logic ---
        // Check if user wants to start processing
        if self.batch_panel.is_processing && self.batch_worker.is_none() {
            // Start new batch processing
            let queue = self.batch_panel.queue.clone();
            let output_dir = match self.batch_panel.output_dir.clone() {
                Some(dir) => dir,
                None => {
                    self.batch_panel.is_processing = false;
                    return;
                }
            };
            let mode = self.batch_panel.selected_mode;

            // Create cancellation flag (use existing if initialized by start_processing)
            let cancel_flag = self.batch_panel.cancel_flag.clone().unwrap_or_else(|| {
                let flag = Arc::new(AtomicBool::new(false));
                self.batch_panel.cancel_flag = Some(flag.clone());
                flag
            });
            self.batch_cancel = Some(cancel_flag.clone());

            // Create progress channel
            let (tx, rx) = std::sync::mpsc::channel();
            self.batch_rx = Some(rx);

            // Spawn worker thread
            self.batch_worker = Some(std::thread::spawn(move || {
                let params = crate::sstv::DecoderParams {
                    mode,
                    ..Default::default()
                };

                let total = queue.len();

                for (index, item) in queue.iter().enumerate() {
                    // Check cancellation
                    if cancel_flag.load(Ordering::Relaxed) {
                        tracing::info!("Batch processing cancelled by user");
                        break;
                    }

                    // Mark as processing
                    let _ = tx.send(BatchProgressMsg::ItemStatus(index, BatchStatus::Processing));

                    // Process the file
                    let result =
                        crate::batch::process_single_file(&item.path, &output_dir, &params);

                    // Send status update
                    let status = match result {
                        Ok(_) => BatchStatus::Done,
                        Err(e) => BatchStatus::Error(e.to_string()),
                    };
                    let _ = tx.send(BatchProgressMsg::ItemStatus(index, status));

                    // Update progress
                    let progress = (index + 1) as f32 / total.max(1) as f32;
                    let _ = tx.send(BatchProgressMsg::Progress(progress));
                }

                tracing::info!("Batch processing thread completed");
            }));

            ctx.request_repaint();
        }

        if self.batch_panel.is_processing {
            // Poll for messages from worker
            if let Some(rx) = &self.batch_rx {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        BatchProgressMsg::ItemStatus(index, status) => {
                            // Update item status
                            if let Some(item) = self.batch_panel.queue.get_mut(index) {
                                item.status = status;
                            }
                            ctx.request_repaint();
                        }
                        BatchProgressMsg::Progress(progress) => {
                            // Update progress bar
                            self.batch_panel.progress = progress;
                            ctx.request_repaint();
                        }
                    }
                }
            }

            // Check if worker finished
            if let Some(handle) = &self.batch_worker {
                if handle.is_finished() {
                    // Join the worker and clean up
                    if let Some(handle) = self.batch_worker.take() {
                        if let Err(e) = handle.join() {
                            tracing::error!("Batch worker thread panicked: {:?}", e);
                        }
                    }
                    self.batch_rx = None;
                    self.batch_cancel = None;
                    self.batch_panel.cancel_flag = None;
                    self.batch_panel.is_processing = false;
                    ctx.request_repaint();
                }
            }
        }

        // Record frame time for performance metrics
        if let Some(start) = self.frame_start {
            self.metrics.record_frame_time(start.elapsed());
        }
        self.frame_start = Some(Instant::now());

        // Check worker health and restart if needed
        if self.config.worker.auto_restart_on_panic && !self.check_worker_health() {
            self.error_message =
                Some("Worker thread crashed or timed out, restarting...".to_string());
            self.restart_worker();
        }

        // Poll for decode results from background worker (non-blocking)
        if let Some(decode_rx) = &self.decode_rx {
            // try_recv() returns immediately without blocking the UI thread
            // If a result is available, apply it; otherwise continue normally
            while let Ok(decode_result) = decode_rx.try_recv() {
                let DecodeResult {
                    id: _,
                    result: pipeline_result,
                    decode_duration,
                    error,
                } = decode_result;

                let success = error.is_none();
                let pixel_count = pipeline_result
                    .as_ref()
                    .map(|r| r.pixels.len())
                    .unwrap_or(0);

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
                self.metrics
                    .record_decode(decode_duration, pixel_count, success);
                self.pending_decode_requests = self.pending_decode_requests.saturating_sub(1);
                self.worker_last_response = Instant::now();

                // Handle error or update texture
                if let Some(err_msg) = error {
                    self.error_message = Some(format!("Decode failed: {}", err_msg));
                } else if let Some(res) = pipeline_result {
                    let img = res.to_egui_image();
                    self.image_texture =
                        Some(ctx.load_texture("decoded_realtime", img, Default::default()));
                    self.last_decoded = Some(res.pixels);
                }
            }
        }

        // Update playback position if playing
        if self.audio_state.is_playing() {
            if let (Some(start_time), Some(wav_reader)) =
                (self.playback_start_time, &self.wav_reader)
            {
                let elapsed = start_time.elapsed();
                let samples_elapsed =
                    (elapsed.as_secs_f32() * wav_reader.sample_rate as f32) as usize;
                let new_position = self.current_position_samples + samples_elapsed;

                if new_position >= wav_reader.left_channel.len() {
                    // Reached end of audio, stop playback
                    self.stop_playback();
                } else {
                    // Update position for next frame
                    self.playback_start_time = Some(Instant::now());
                    self.current_position_samples = new_position;

                    // Real-time decoding: decode from current position only if significant change
                    // Decode every 500ms of audio to avoid flooding worker thread
                    let decode_threshold_samples = (wav_reader.sample_rate as f32 * 0.5) as usize;
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

        // Draw Batch Panel
        self.batch_panel.draw(ctx);

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🚀 Voyager Golden Record Explorer");

                if ui.button("📂 Load WAV").clicked() {
                    self.handle_load_wav();
                }

                if ui.button("🧠 Decode").clicked() {
                    self.handle_decode(ctx);
                }

                if ui.button("📦 Batch").clicked() {
                    self.batch_panel.visible = !self.batch_panel.visible;
                }

                ui.separator();
                if ui
                    .selectable_label(self.spectrum_panel.visible, "📈 Spectrum")
                    .clicked()
                {
                    self.spectrum_panel.visible = !self.spectrum_panel.visible;
                }
            });

            // Display error message if present
            let mut dismiss_error = false;
            if let Some(error) = &self.error_message {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 100, 100),
                        format!("⚠️ {}", error),
                    );
                    if ui.button("✖").clicked() {
                        dismiss_error = true;
                    }
                });
            }
            if dismiss_error {
                self.error_message = None;
            }

            ui.separator();

            ui.horizontal(|ui| {
                ui.label("📏 Line Duration (ms):");
                ui.add(egui::DragValue::new(&mut self.params.line_duration_ms).range(1..=100));
                ui.label("🔪 Threshold:");
                ui.add(egui::Slider::new(&mut self.params.threshold, 0.0..=1.0));

                ui.separator();
                ui.label("🎨 Mode:");
                egui::ComboBox::from_label("")
                    .selected_text(match self.params.mode {
                        DecoderMode::BinaryGrayscale => "Binary (B/W)",
                        DecoderMode::PseudoColor => "PseudoColor",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.params.mode,
                            DecoderMode::BinaryGrayscale,
                            "Binary (B/W)",
                        );
                        ui.selectable_value(
                            &mut self.params.mode,
                            DecoderMode::PseudoColor,
                            "PseudoColor",
                        );
                    });

                ui.separator();
                ui.label("📻 Channel:");
                egui::ComboBox::from_label("")
                    .selected_text(match self.selected_channel {
                        WaveformChannel::Left => "Left",
                        WaveformChannel::Right => "Right",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.selected_channel,
                            WaveformChannel::Left,
                            "Left",
                        );
                        ui.selectable_value(
                            &mut self.selected_channel,
                            WaveformChannel::Right,
                            "Right",
                        );
                    });
            });
        });

        egui::TopBottomPanel::bottom("debug_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Audio status indicator
                let status_icon = self.audio_state.status_icon();
                let status_message = self.audio_state.status_message();
                ui.label(format!("{} {}", status_icon, status_message));
                ui.separator();

                if let Some(reader) = &self.wav_reader {
                    let duration_secs =
                        reader.left_channel.len() as f32 / reader.sample_rate as f32;
                    ui.label(format!(
                        "📦 {} samples @ {} Hz ({}) - {}",
                        reader.left_channel.len(),
                        reader.sample_rate,
                        if reader.channels == 1 {
                            "mono"
                        } else {
                            "stereo"
                        },
                        format_duration(duration_secs)
                    ));
                } else {
                    ui.label("📦 No file loaded");
                }

                if let Some(pixels) = &self.last_decoded {
                    ui.label(format!("🖼️ Decoded size: {}x{}", 512, pixels.len() / 512));
                }
            });
        });

        // Left panel for decoded image
        egui::SidePanel::left("image_panel")
            .default_width(ctx.input(|i| {
                i.viewport()
                    .inner_rect
                    .map(|r| r.width() * 0.6)
                    .unwrap_or(800.0)
            }))
            .show(ctx, |ui| {
                ui.heading("Decoded Image");
                ui.separator();
                if let Some(texture) = &self.image_texture {
                    ui.image(texture);
                } else {
                    ui.label("🖼️ No image decoded yet.");
                }
            });

        // Spectrum Analysis Panel (Right Side)
        if self.spectrum_panel.visible {
            egui::SidePanel::right("spectrum_panel")
                .default_width(400.0)
                .show(ctx, |ui| {
                    self.spectrum_panel.draw(
                        ui,
                        &self.wav_reader,
                        self.current_position_samples,
                        self.selected_channel,
                    );
                });
        }

        // Bottom panel for waveform visualization
        egui::TopBottomPanel::bottom("waveform_panel")
            .default_height(200.0)
            .show(ctx, |ui| {
                ui.heading("Audio Waveform");
                ui.separator();

                // Playback controls
                if let Some(action) = ControlsPanel::draw(ui, self.audio_state.is_playing()) {
                    match action {
                        ControlAction::TogglePlayback => self.toggle_playback(),
                        ControlAction::StopPlayback => self.stop_playback(),
                        ControlAction::SeekToNextSync => self.seek_to_next_sync(),
                    }
                }

                ui.separator();

                // Waveform visualization
                if let Some(new_pos) = WaveformPanel::draw(
                    ui,
                    &self.wav_reader,
                    self.selected_channel,
                    self.current_position_samples,
                    &mut self.waveform_hover_position,
                ) {
                    self.current_position_samples = new_pos;

                    #[cfg(feature = "audio_playback")]
                    self.restart_audio_from_current_position();

                    #[cfg(not(feature = "audio_playback"))]
                    if self.audio_state.is_playing() {
                        self.playback_start_time = Some(Instant::now());
                    }

                    // Trigger decode on manual seek
                    self.decode_at_position(ctx, self.current_position_samples);
                    self.last_decode_position = self.current_position_samples;
                }
            });

        // Central panel for controls and info
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("🚀 Voyager Golden Record Explorer");
            ui.separator();
            ui.label("Use the controls in the top panel to load audio files and adjust decoder settings.");
        });
    }
}
