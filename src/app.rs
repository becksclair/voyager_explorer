use crate::audio::{WavReader, WaveformChannel};
use crate::image_output::image_from_pixels;
use crate::sstv::{DecoderParams, SstvDecoder};
use crate::utils::format_duration;
use eframe::egui;
use egui::TextureHandle;
use rodio::{source::SineWave, OutputStreamBuilder, Sink, Source};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Audio source that plays from a buffer of f32 samples
struct AudioBufferSource {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    position: usize,
}

impl AudioBufferSource {
    fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            position: 0,
        }
    }
}

impl Iterator for AudioBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position < self.samples.len() {
            let sample = self.samples[self.position];
            self.position += 1;
            Some(sample)
        } else {
            None
        }
    }
}

impl Source for AudioBufferSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.position)
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let total_samples = self.samples.len() as u64;
        let duration_secs = total_samples as f64 / (self.sample_rate as f64 * self.channels as f64);
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
    // audio_output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    audio_sink: Option<Sink>,
    is_playing: bool,
    current_position_samples: usize,
    waveform_hover_position: Option<f32>,
    playback_start_time: Option<Instant>,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        Self {
            wav_reader: None,
            video_decoder: SstvDecoder::new(),
            image_texture: None,
            params: DecoderParams::default(),
            last_decoded: None,
            selected_channel: WaveformChannel::Left,
            // audio_output: None,
            audio_sink: None,
            is_playing: false,
            current_position_samples: 0,
            waveform_hover_position: None,
            playback_start_time: None,
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
                }
                Err(e) => {
                    eprintln!("Failed to load WAV file: {}", e);
                }
            }
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            self.video_decoder
                .detect_sync(samples.to_vec(), reader.sample_rate);

            let pixels = self
                .video_decoder
                .decode(samples, &self.params, reader.sample_rate);
            let img = image_from_pixels(&pixels);
            self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
            self.last_decoded = Some(pixels);
        }
    }

    fn toggle_playback(&mut self) {
        // For now, just toggle the state - we'll implement actual audio playback later
        self.is_playing = !self.is_playing;
        if self.is_playing {
            self.playback_start_time = Some(Instant::now());
            println!("Starting playback...");
        } else {
            println!("Pausing playback...");
        }
    }

    fn stop_playback(&mut self) {
        // For now, just reset the state
        self.is_playing = false;
        self.current_position_samples = 0;
        self.playback_start_time = None;
        println!("Stopping playback...");
    }

    fn decode_at_position(&mut self, ctx: &egui::Context, position: usize) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);

            // Calculate how many samples to decode for a reasonably-sized image segment
            let decode_duration_seconds = 2.0; // Decode 2 seconds worth of audio
            let samples_to_decode = (decode_duration_seconds * reader.sample_rate as f32) as usize;

            let end_position = (position + samples_to_decode).min(samples.len());

            if position < samples.len() && end_position > position {
                let segment = &samples[position..end_position];

                // Decode this segment
                let pixels = self
                    .video_decoder
                    .decode(segment, &self.params, reader.sample_rate);

                if !pixels.is_empty() {
                    let img = image_from_pixels(&pixels);
                    self.image_texture =
                        Some(ctx.load_texture("decoded_realtime", img, Default::default()));
                    self.last_decoded = Some(pixels);
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

                // If playing, update the start time for position tracking
                if self.is_playing {
                    self.playback_start_time = Some(Instant::now());
                }
            } else {
                println!("No more sync signals found");
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
            if samples.len() > 0 {
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
        // Update playback position if playing
        if self.is_playing {
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
        if self.is_playing {
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
            .default_width(ctx.screen_rect().width() * 0.6)
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
                    let play_button_text = if self.is_playing {
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
                    if response.clicked() {
                        let click_pos = response.interact_pointer_pos().unwrap_or_default();
                        let relative_x = (click_pos.x - rect.min.x) / rect.width();
                        let seek_sample = (relative_x * samples.len() as f32) as usize;
                        self.current_position_samples =
                            seek_sample.min(samples.len().saturating_sub(1));
                        println!("Seeking to sample: {}", self.current_position_samples);
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
