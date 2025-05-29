use eframe::egui;
use egui::TextureHandle;
use crate::audio::{WavReader, WaveformChannel};
use crate::sstv::{SstvDecoder, DecoderParams};
use crate::image_output::image_from_pixels;

pub struct VoyagerApp {
    wav_reader: Option<WavReader>,
    left_waveform: Vec<f32>,
    right_waveform: Vec<f32>,
    decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<Vec<u8>>,
    selected_channel: WaveformChannel,
    waveform_width: f32,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        Self {
            wav_reader: None,
            left_waveform: vec![],
            right_waveform: vec![],
            decoder: SstvDecoder::new(),
            image_texture: None,
            params: DecoderParams::default(),
            last_decoded: None,
            selected_channel: WaveformChannel::Left,
            waveform_width: 0.0,
        }
    }
}

impl VoyagerApp {
    fn update_waveforms_if_needed(&mut self, ui_width: f32) {
        if let Some(ref wav_reader) = self.wav_reader {
            // Use more pixels for better resolution, but cap at reasonable limits
            let target_width = (ui_width as usize * 2).min(2048).max(512);
            if (self.waveform_width - ui_width).abs() > 50.0 || self.left_waveform.is_empty() {
                self.left_waveform = wav_reader.get_waveform_preview(target_width, WaveformChannel::Left);
                self.right_waveform = wav_reader.get_waveform_preview(target_width, WaveformChannel::Right);
                self.waveform_width = ui_width;
            }
        }
    }

    fn draw_waveform(&self, painter: &egui::Painter, wave: &[f32], rect: egui::Rect, color: egui::Color32) {
        if wave.is_empty() {
            return;
        }

        // Draw background
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(15));
        
        // Draw grid lines
        let mid = rect.center().y;
        let grid_color = egui::Color32::from_gray(30);
        
        // Horizontal center line
        painter.line_segment(
            [egui::pos2(rect.left(), mid), egui::pos2(rect.right(), mid)],
            egui::Stroke::new(1.0, grid_color),
        );
        
        // Quarter and three-quarter lines
        let quarter = rect.top() + rect.height() * 0.25;
        let three_quarter = rect.top() + rect.height() * 0.75;
        painter.line_segment(
            [egui::pos2(rect.left(), quarter), egui::pos2(rect.right(), quarter)],
            egui::Stroke::new(0.5, grid_color),
        );
        painter.line_segment(
            [egui::pos2(rect.left(), three_quarter), egui::pos2(rect.right(), three_quarter)],
            egui::Stroke::new(0.5, grid_color),
        );

        // Draw waveform
        let width = rect.width();
        let step = width / wave.len() as f32;
        let height_scale = rect.height() * 0.45; // Use 45% of available height for amplitude scaling

        // Draw filled waveform for better visual representation
        for (i, &amp) in wave.iter().enumerate() {
            let x = rect.left() + step * i as f32;
            let y = mid - amp * height_scale;
            
            // Draw vertical line from center to amplitude
            if amp.abs() > 0.001 { // Only draw if amplitude is significant
                painter.line_segment(
                    [egui::pos2(x, mid), egui::pos2(x, y)],
                    egui::Stroke::new(1.0, color),
                );
            }
        }

        // Also draw connected line for smoother appearance
        for (i, amp) in wave.iter().enumerate().skip(1) {
            let x1 = rect.left() + step * (i - 1) as f32;
            let x2 = rect.left() + step * i as f32;
            let y1 = mid - wave[i - 1] * height_scale;
            let y2 = mid - amp * height_scale;

            painter.line_segment(
                [egui::pos2(x1, y1), egui::pos2(x2, y2)],
                egui::Stroke::new(0.8, color.gamma_multiply(0.7)),
            );
        }
    }

    fn handle_load_wav(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .pick_file()
            .and_then(|path| WavReader::from_file(&path).ok())
        {
            self.wav_reader = Some(path);
            self.left_waveform.clear();
            self.right_waveform.clear();
            self.waveform_width = 0.0;
            self.image_texture = None;
            self.last_decoded = None;
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);
            let pixels = self.decoder.decode(samples, &self.params, reader.sample_rate);
            let img = image_from_pixels(&pixels);
            self.image_texture = Some(ctx.load_texture("decoded", img, Default::default()));
            self.last_decoded = Some(pixels);
        }
    }
}

impl eframe::App for VoyagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                        ui.selectable_value(&mut self.selected_channel, WaveformChannel::Left, "Left");
                        ui.selectable_value(&mut self.selected_channel, WaveformChannel::Right, "Right");
                    });
            });
        });

        egui::TopBottomPanel::bottom("debug_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(reader) = &self.wav_reader {
                    let duration_secs = reader.left_channel.len() as f32 / reader.sample_rate as f32;
                    let minutes = (duration_secs / 60.0) as u32;
                    let seconds = duration_secs % 60.0;
                    ui.label(format!("📦 {} samples @ {} Hz ({} ch) - {:02}:{:05.2}s", 
                        reader.left_channel.len(), 
                        reader.sample_rate,
                        if reader.channels == 1 { "mono" } else { "stereo" },
                        minutes,
                        seconds
                    ));
                } else {
                    ui.label("📦 No file loaded");
                }

                if let Some(pixels) = &self.last_decoded {
                    ui.label(format!("🖼️ Decoded size: {}x{}", 512, pixels.len() / 512));
                }
            });
        });

        egui::TopBottomPanel::bottom("waveform_panel")
            .resizable(true)
            .min_height(250.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("📉 Waveform Preview");
                    if let Some(_reader) = &self.wav_reader {
                        ui.separator();
                        ui.label(format!("Resolution: {} points", self.left_waveform.len()));
                    }
                });
                ui.separator();

                if !self.left_waveform.is_empty() || self.wav_reader.is_some() {
                    // Update waveforms with current UI width
                    self.update_waveforms_if_needed(ui.available_width());
                    
                    if !self.left_waveform.is_empty() {
                        // Left channel
                        ui.horizontal(|ui| {
                            ui.label("🔊 Left Channel");
                            if self.selected_channel == WaveformChannel::Left {
                                ui.colored_label(egui::Color32::LIGHT_GREEN, "● SELECTED");
                            }
                        });
                        
                        let available_height = (ui.available_height() - 80.0) / 2.0; // Split remaining height between two waveforms, reserve space for labels
                        let height = available_height.max(100.0); // Minimum 100px height
                        let response = ui.allocate_response(egui::vec2(ui.available_width(), height), egui::Sense::hover());
                        let painter = ui.painter_at(response.rect);

                        self.draw_waveform(&painter, &self.left_waveform, response.rect, egui::Color32::LIGHT_GREEN);

                        ui.add_space(15.0);

                        // Right channel
                        ui.horizontal(|ui| {
                            ui.label("🔊 Right Channel");
                            if self.selected_channel == WaveformChannel::Right {
                                ui.colored_label(egui::Color32::LIGHT_BLUE, "● SELECTED");
                            }
                        });

                        let response = ui.allocate_response(egui::vec2(ui.available_width(), height), egui::Sense::hover());
                        let painter = ui.painter_at(response.rect);

                        self.draw_waveform(&painter, &self.right_waveform, response.rect, egui::Color32::LIGHT_BLUE);
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label("🔄 Generating waveform preview...");
                        });
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("📁 Load a WAV file to see waveform preview");
                    });
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.image_texture {
                ui.image(texture);
            } else {
                ui.label("🖼️ No image decoded yet.");
            }
        });
    }
}
