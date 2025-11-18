use crate::audio::{WavReader, WaveformChannel};
use crate::audio_state::{AudioMetrics, AudioPlaybackState};
use crate::image_output::image_from_pixels;
use crate::sstv::{DecoderParams, SstvDecoder};
use crate::utils::format_duration;
use eframe::egui;
use egui::TextureHandle;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "audio_playback")]
use crate::audio_state::AudioError;

#[cfg(feature = "audio_playback")]
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

/// Request to decode audio samples in background thread.
///
/// # Message Passing Architecture
///
/// The decoding operation is CPU-intensive (FFT, sync detection, pixel processing)
/// and can take 100-500ms for large files. Running it on the UI thread causes
/// frame drops and unresponsive controls.
///
/// **Solution**: Offload decoding to a background worker thread using message passing:
/// - Main thread sends `DecodeRequest` via `decode_tx` channel
/// - Worker thread performs decoding with its own `SstvDecoder` instance
/// - Worker thread sends `DecodeResult` back via `decode_rx` channel
/// - Main thread polls `decode_rx` in `update()` and applies results
///
/// **Benefits**:
/// - UI remains responsive during decode (60fps maintained)
/// - No blocking operations in event loop
/// - Clean separation of concerns (UI thread vs compute thread)
#[derive(Debug)]
struct DecodeRequest {
    /// Unique request ID for matching results to requests
    id: u64,
    /// Shared audio buffer (Arc enables zero-copy sharing with worker thread)
    samples: Arc<[f32]>,
    /// Starting sample position for decode window
    start_offset: usize,
    /// Decoder parameters (line duration, threshold)
    params: DecoderParams,
    /// Sample rate in Hz (needed for samples-per-line calculation)
    sample_rate: u32,
}

/// Result from background decoding operation.
#[derive(Debug)]
struct DecodeResult {
    /// Request ID (matches DecodeRequest.id) - currently unused but reserved for future features
    #[allow(dead_code)]
    id: u64,
    /// Decoded image data (grayscale pixels, 512px width)
    pixels: Vec<u8>,
    /// Time taken to decode (for performance monitoring)
    decode_duration: Duration,
}

#[cfg(feature = "audio_playback")]
/// Audio source that plays from a shared buffer of f32 samples with zero-copy seeking.
///
/// # Performance Characteristics
///
/// **Traditional approach (cloning):**
/// ```ignore
/// let remaining_samples = samples[position..].to_vec();  // O(n) clone
/// ```
/// - For 100MB file @ 50% position: **50MB copied per seek**
/// - Seek latency: ~100ms for large files
/// - Memory pressure: High (GC thrashing)
///
/// **This approach (Arc + offset):**
/// ```ignore
/// AudioBufferSource::new(Arc::clone(&buffer), offset, ...)  // O(1)
/// ```
/// - For 100MB file: **16 bytes (Arc pointer + offset) per seek**
/// - Seek latency: ~1ms (just metadata update)
/// - Memory pressure: Minimal (Arc shared across all instances)
///
/// # Implementation Details
///
/// The `buffer` is shared via `Arc<[f32]>`, so all `AudioBufferSource` instances
/// point to the same underlying memory. The `offset` field marks where in the
/// buffer this source should start reading, and `position` tracks the current
/// read position relative to that offset.
///
/// When seeking, we don't clone any samples - we just create a new `AudioBufferSource`
/// with a different `offset`, reusing the same `Arc<[f32]>`.
struct AudioBufferSource {
    /// Shared reference to the audio buffer. Arc enables zero-copy sharing.
    buffer: Arc<[f32]>,
    /// Starting position in the buffer (sample index where playback begins).
    offset: usize,
    /// Sample rate in Hz (e.g., 44100, 48000).
    sample_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo).
    channels: u16,
    /// Current read position relative to offset.
    position: usize,
}

#[cfg(feature = "audio_playback")]
impl AudioBufferSource {
    fn new(buffer: Arc<[f32]>, offset: usize, sample_rate: u32, channels: u16) -> Self {
        Self {
            buffer,
            offset,
            sample_rate,
            channels,
            position: 0,
        }
    }
}

#[cfg(feature = "audio_playback")]
impl Iterator for AudioBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let absolute_position = self.offset + self.position;
        if absolute_position < self.buffer.len() {
            let sample = self.buffer[absolute_position];
            self.position += 1;
            Some(sample)
        } else {
            None
        }
    }
}

#[cfg(feature = "audio_playback")]
impl Source for AudioBufferSource {
    fn current_span_len(&self) -> Option<usize> {
        Some((self.buffer.len() - self.offset).saturating_sub(self.position))
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let remaining_samples = (self.buffer.len() - self.offset) as u64;
        let duration_secs =
            remaining_samples as f64 / (self.sample_rate as f64 * self.channels as f64);
        Some(Duration::from_secs_f64(duration_secs))
    }
}

pub struct VoyagerApp {
    wav_reader: Option<WavReader>,
    video_decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<Vec<u8>>,
    selected_channel: WaveformChannel,
    // Audio playback state
    audio_state: AudioPlaybackState,
    audio_metrics: AudioMetrics,
    #[cfg(feature = "audio_playback")]
    audio_stream: Option<(OutputStream, OutputStreamHandle)>,
    #[cfg(feature = "audio_playback")]
    audio_sink: Option<Sink>,
    current_position_samples: usize,
    waveform_hover_position: Option<f32>,
    playback_start_time: Option<Instant>,
    // Background decoding worker
    decode_tx: Option<Sender<DecodeRequest>>,
    decode_rx: Option<Receiver<DecodeResult>>,
    next_decode_id: u64,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        // Create channels for background decoding worker
        let (decode_tx, result_rx) = channel();
        let (result_tx, decode_rx) = channel();

        // Spawn background worker thread for non-blocking decoding
        // This thread runs for the lifetime of the application
        thread::spawn(move || {
            // Worker has its own SstvDecoder instance to avoid sharing across threads
            let decoder = SstvDecoder::new();

            // Process decode requests until channel is closed
            while let Ok(request) = result_rx.recv() {
                let DecodeRequest {
                    id,
                    samples,
                    start_offset,
                    params,
                    sample_rate,
                } = request;

                let decode_start = Instant::now();

                // Extract decode window from shared buffer
                let window_duration_secs = 2.0; // 2-second decode window
                let window_samples = (window_duration_secs * sample_rate as f64) as usize;
                let end_offset = (start_offset + window_samples).min(samples.len());
                let decode_slice = &samples[start_offset..end_offset];

                // Perform actual decoding (CPU-intensive FFT + pixel processing)
                let pixels = decoder.decode(decode_slice, &params, sample_rate);

                let decode_duration = decode_start.elapsed();

                // Send result back to main thread
                let result = DecodeResult {
                    id,
                    pixels,
                    decode_duration,
                };

                // If send fails, main thread has dropped the receiver (app is closing)
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        });

        Self {
            wav_reader: None,
            video_decoder: SstvDecoder::new(),
            image_texture: None,
            params: DecoderParams::default(),
            last_decoded: None,
            selected_channel: WaveformChannel::Left,
            audio_state: AudioPlaybackState::Uninitialized,
            audio_metrics: AudioMetrics::default(),
            #[cfg(feature = "audio_playback")]
            audio_stream: None,
            #[cfg(feature = "audio_playback")]
            audio_sink: None,
            current_position_samples: 0,
            waveform_hover_position: None,
            playback_start_time: None,
            decode_tx: Some(decode_tx),
            decode_rx: Some(decode_rx),
            next_decode_id: 0,
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
                    self.wav_reader = Some(reader);
                    self.image_texture = None;
                    self.last_decoded = None;
                    // Update audio state to Ready when WAV is loaded
                    self.audio_state = AudioPlaybackState::Ready;
                }
                Err(e) => {
                    eprintln!("Failed to load WAV file: {}", e);
                    self.audio_state = AudioPlaybackState::Uninitialized;
                }
            }
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            // Detect sync presence (logged only in debug builds)
            #[cfg(debug_assertions)]
            {
                let sync_detected = self
                    .video_decoder
                    .detect_sync(samples.to_vec(), reader.sample_rate);
                println!(
                    "Sync detection result: {}",
                    if sync_detected { "found" } else { "not found" }
                );
            }

            let pixels = self
                .video_decoder
                .decode(samples, &self.params, reader.sample_rate);
            let img = image_from_pixels(&pixels);
            self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
            self.last_decoded = Some(pixels);
        }
    }

    #[cfg(feature = "audio_playback")]
    /// Ensure audio stream is initialized, return OutputStreamHandle if available
    fn ensure_audio_stream(&mut self) -> Option<OutputStreamHandle> {
        if self.audio_stream.is_none() {
            match OutputStream::try_default() {
                Ok((stream, handle)) => {
                    self.audio_stream = Some((stream, handle.clone()));
                    Some(handle)
                }
                Err(e) => {
                    eprintln!("Failed to initialize audio stream: {}", e);
                    self.audio_state = AudioPlaybackState::Error(AudioError::StreamInitFailed);
                    self.audio_metrics.record_device_error();
                    None
                }
            }
        } else {
            // Return cloned handle from existing stream
            self.audio_stream.as_ref().map(|(_, handle)| handle.clone())
        }
    }

    fn toggle_playback(&mut self) {
        #[cfg(feature = "audio_playback")]
        {
            match self.audio_state {
                AudioPlaybackState::Playing => {
                    // Pause playback
                    if let Some(sink) = &self.audio_sink {
                        sink.pause();
                    }
                    self.audio_state = AudioPlaybackState::Paused;
                    self.audio_metrics.record_pause();
                    println!("Pausing playback...");
                }
                AudioPlaybackState::Paused => {
                    // Resume playback
                    if let Some(sink) = &self.audio_sink {
                        sink.play();
                        self.audio_state = AudioPlaybackState::Playing;
                        self.playback_start_time = Some(Instant::now());
                        self.audio_metrics.record_play();
                        println!("Resuming playback...");
                    }
                }
                AudioPlaybackState::Ready => {
                    // Start fresh playback
                    if self.wav_reader.is_none() {
                        return;
                    }

                    // Ensure audio stream is available
                    let handle = match self.ensure_audio_stream() {
                        Some(h) => h,
                        None => return, // Error already logged in ensure_audio_stream
                    };

                    // Create sink with the handle
                    match Sink::try_new(&handle) {
                        Ok(sink) => {
                            if let Some(source) = self.make_buffer_source_from_current_position() {
                                sink.append(source);
                                sink.play();
                                self.audio_sink = Some(sink);
                                self.audio_state = AudioPlaybackState::Playing;
                                self.playback_start_time = Some(Instant::now());
                                self.audio_metrics.record_play();
                                println!("Starting playback...");
                            } else {
                                eprintln!("No audio samples available to start playback");
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create audio sink: {}", e);
                            self.audio_state =
                                AudioPlaybackState::Error(AudioError::SinkCreationFailed);
                            self.audio_metrics.record_device_error();
                        }
                    }
                }
                AudioPlaybackState::Uninitialized | AudioPlaybackState::Error(_) => {
                    // Can't play in these states
                    println!("Cannot play: {}", self.audio_state);
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
                    self.audio_metrics.record_play();
                    println!("Starting visual playback...");
                }
                AudioPlaybackState::Playing => {
                    self.audio_state = AudioPlaybackState::Paused;
                    self.audio_metrics.record_pause();
                    println!("Pausing visual playback...");
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
        self.audio_metrics.record_stop();
        self.current_position_samples = 0;
        self.playback_start_time = None;
        println!("Stopping playback...");
    }

    /// Enqueue a non-blocking decode request at the given sample position.
    ///
    /// # Non-Blocking Architecture
    ///
    /// **OLD approach (blocking):**
    /// ```ignore
    /// let pixels = self.video_decoder.decode(segment, &params, sample_rate);
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

                // Bounds check
                if position >= samples.len() {
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

                // Send to worker thread (non-blocking, returns immediately)
                // If worker is busy, request queues up in channel
                if decode_tx.send(request).is_err() {
                    eprintln!("Warning: decode worker thread has terminated");
                }
            }
        }
    }

    fn seek_to_next_sync(&mut self) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);
            let next_sync = self.video_decoder.find_next_sync(
                samples,
                self.current_position_samples,
                reader.sample_rate,
            );

            if let Some(sync_position) = next_sync {
                self.current_position_samples = sync_position;
                println!("Seeking to next sync at sample: {}", sync_position);

                // If playing, restart audio from new position
                #[cfg(feature = "audio_playback")]
                self.restart_audio_from_current_position();

                #[cfg(not(feature = "audio_playback"))]
                if self.audio_state.is_playing() {
                    self.playback_start_time = Some(Instant::now());
                }
            } else {
                println!("No more sync signals found");
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
        Some(AudioBufferSource::new(
            buffer,
            self.current_position_samples,
            reader.sample_rate,
            1, // Mono playback (we've already selected a channel)
        ))
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
        let handle = match self.ensure_audio_stream() {
            Some(h) => h,
            None => {
                self.audio_state = AudioPlaybackState::Error(AudioError::StreamInitFailed);
                return;
            }
        };

        // Create new sink with source from current position
        match Sink::try_new(&handle) {
            Ok(sink) => {
                if let Some(source) = self.make_buffer_source_from_current_position() {
                    sink.append(source);
                    sink.play();
                    self.audio_sink = Some(sink);
                    self.playback_start_time = Some(Instant::now());
                    self.audio_metrics.record_seek();
                } else {
                    eprintln!("No audio samples available after seek");
                }
            }
            Err(e) => {
                eprintln!("Failed to create audio sink after seek: {}", e);
                self.audio_state = AudioPlaybackState::Error(AudioError::SinkCreationFailed);
                self.audio_metrics.record_device_error();
            }
        }
    }

    fn draw_waveform_internal(
        &self,
        ui: &mut egui::Ui,
        rect: &egui::Rect,
        samples: &[f32],
        current_position: usize,
        hover_position: Option<f32>,
    ) {
        if ui.is_rect_visible(*rect) {
            let painter = ui.painter();

            // Background
            painter.rect_filled(*rect, 0.0, egui::Color32::from_gray(20));

            // Draw waveform
            let samples_per_pixel = samples.len().max(1) as f32 / rect.width();

            for pixel_x in 0..rect.width() as i32 {
                let start_sample = (pixel_x as f32 * samples_per_pixel) as usize;
                let end_sample =
                    (((pixel_x + 1) as f32 * samples_per_pixel) as usize).min(samples.len());

                if start_sample < samples.len() {
                    // Find min/max in this pixel range for better visualization
                    let mut min_val = 1.0f32;
                    let mut max_val = -1.0f32;

                    for sample_idx in start_sample..end_sample {
                        if sample_idx < samples.len() {
                            let sample = samples[sample_idx];
                            min_val = min_val.min(sample);
                            max_val = max_val.max(sample);
                        }
                    }

                    let center_y = rect.center().y;
                    let amplitude_scale = rect.height() * 0.4; // Use 40% of height for amplitude

                    let min_y = center_y - min_val * amplitude_scale;
                    let max_y = center_y - max_val * amplitude_scale;

                    let x = rect.min.x + pixel_x as f32;

                    // Draw vertical line from min to max
                    painter.line_segment(
                        [egui::Pos2::new(x, min_y), egui::Pos2::new(x, max_y)],
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 200, 255)),
                    );
                }
            }

            // Draw current position indicator
            if !samples.is_empty() {
                let position_x =
                    rect.min.x + (current_position as f32 / samples.len() as f32) * rect.width();
                painter.line_segment(
                    [
                        egui::Pos2::new(position_x, rect.min.y),
                        egui::Pos2::new(position_x, rect.max.y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::RED),
                );
            }

            // Draw hover line
            if let Some(hover_x) = hover_position {
                let hover_pixel_x = rect.min.x + hover_x * rect.width();
                painter.line_segment(
                    [
                        egui::Pos2::new(hover_pixel_x, rect.min.y),
                        egui::Pos2::new(hover_pixel_x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::YELLOW),
                );
            }
        }
    }
}

impl eframe::App for VoyagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll for decode results from background worker (non-blocking)
        if let Some(decode_rx) = &self.decode_rx {
            // try_recv() returns immediately without blocking the UI thread
            // If a result is available, apply it; otherwise continue normally
            while let Ok(result) = decode_rx.try_recv() {
                let DecodeResult {
                    id: _,
                    pixels,
                    decode_duration,
                } = result;

                // Log performance metrics
                #[cfg(debug_assertions)]
                println!(
                    "Decode completed in {:.2}ms ({} pixels)",
                    decode_duration.as_secs_f64() * 1000.0,
                    pixels.len()
                );

                // Update texture with decoded image
                if !pixels.is_empty() {
                    let img = image_from_pixels(&pixels);
                    self.image_texture =
                        Some(ctx.load_texture("decoded_realtime", img, Default::default()));
                    self.last_decoded = Some(pixels);
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

                    // Real-time decoding: decode from current position
                    self.decode_at_position(ctx, new_position);
                }
            }
        }

        // Request continuous repaints during playback for position updates
        if self.audio_state.is_playing() {
            ctx.request_repaint();
        }
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🚀 Voyager Golden Record Explorer");

                if ui.button("📂 Load WAV").clicked() {
                    self.handle_load_wav();
                }

                if ui.button("🧠 Decode").clicked() {
                    self.handle_decode(ctx);
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label("📏 Line Duration (ms):");
                ui.add(egui::DragValue::new(&mut self.params.line_duration_ms).range(1..=100));
                ui.label("🔪 Threshold:");
                ui.add(egui::Slider::new(&mut self.params.threshold, 0.0..=1.0));

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

        // Bottom panel for waveform visualization
        egui::TopBottomPanel::bottom("waveform_panel")
            .default_height(200.0)
            .show(ctx, |ui| {
                ui.heading("Audio Waveform");
                ui.separator();

                // Playback controls
                ui.horizontal(|ui| {
                    let play_button_text = if self.audio_state.is_playing() {
                        "⏸ Pause"
                    } else {
                        "▶ Play"
                    };
                    if ui.button(play_button_text).clicked() {
                        self.toggle_playback();
                    }

                    if ui.button("⏹ Stop").clicked() {
                        self.stop_playback();
                    }

                    if ui.button("⏭ Skip to Next Sync").clicked() {
                        self.seek_to_next_sync();
                    }

                    // Position display
                    if let Some(reader) = &self.wav_reader {
                        let duration_secs =
                            reader.left_channel.len() as f32 / reader.sample_rate as f32;
                        let current_secs =
                            self.current_position_samples as f32 / reader.sample_rate as f32;
                        ui.label(format!(
                            "Position: {} / {}",
                            format_duration(current_secs),
                            format_duration(duration_secs)
                        ));
                    }
                });

                ui.separator();

                // Waveform visualization (placeholder for now)
                if self.wav_reader.is_some() {
                    let selected_channel = self.selected_channel;
                    let current_position = self.current_position_samples;
                    let hover_position = self.waveform_hover_position;
                    let wav_reader = self.wav_reader.as_ref().unwrap();

                    let samples = wav_reader.get_samples(selected_channel);
                    let available_width = ui.available_width();
                    let available_height = ui.available_height().min(150.0);

                    let response = ui.allocate_response(
                        egui::Vec2::new(available_width, available_height),
                        egui::Sense::click_and_drag(),
                    );
                    let rect = response.rect;

                    // Handle mouse interaction for seeking
                    #[cfg(feature = "audio_playback")]
                    let should_restart_audio = response.clicked() && self.audio_state.is_playing();

                    if response.clicked() {
                        let click_pos = response.interact_pointer_pos().unwrap_or_default();
                        let relative_x = (click_pos.x - rect.min.x) / rect.width();
                        let samples_len = samples.len();
                        let seek_sample = (relative_x * samples_len as f32) as usize;
                        self.current_position_samples =
                            seek_sample.min(samples_len.saturating_sub(1));
                        println!("Seeking to sample: {}", self.current_position_samples);

                        #[cfg(not(feature = "audio_playback"))]
                        if self.audio_state.is_playing() {
                            self.playback_start_time = Some(Instant::now());
                        }
                    }

                    // Track hover position for vertical line
                    if response.hovered() {
                        if let Some(hover_pos) = response.hover_pos() {
                            let relative_x = (hover_pos.x - rect.min.x) / rect.width();
                            self.waveform_hover_position = Some(relative_x.clamp(0.0, 1.0));
                        }
                    } else {
                        self.waveform_hover_position = None;
                    }

                    self.draw_waveform_internal(
                        ui,
                        &rect,
                        samples,
                        current_position,
                        hover_position,
                    );

                    // Restart audio after drawing is complete (borrow checker fix)
                    #[cfg(feature = "audio_playback")]
                    if should_restart_audio {
                        self.restart_audio_from_current_position();
                    }
                } else {
                    ui.label("📈 No waveform data available");
                }
            });

        // Central panel for controls and info
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("SSTV Decoder Settings");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("📏 Line Duration (ms):");
                ui.add(egui::DragValue::new(&mut self.params.line_duration_ms).range(1..=100));
                ui.label("🔪 Threshold:");
                ui.add(egui::Slider::new(&mut self.params.threshold, 0.0..=1.0));
            });
        });
    }
}
