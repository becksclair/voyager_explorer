use eframe::egui;
use egui::TextureHandle;
use crate::audio::WavReader;
use crate::sstv::{SstvDecoder, DecoderParams};
use crate::image_output::image_from_pixels;

pub struct VoyagerApp {
    wav_reader: Option<WavReader>,
    waveform: Vec<f32>,
    decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<Vec<u8>>,
}

impl Default for VoyagerApp {
    fn default() -> Self {
        Self {
            wav_reader: None,
            waveform: vec![],
            decoder: SstvDecoder::new(),
            image_texture: None,
            params: DecoderParams::default(),
            last_decoded: None,
        }
    }
}

impl VoyagerApp {
    fn handle_load_wav(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .pick_file()
        {
            if let Ok(reader) = WavReader::from_file(&path) {
                self.waveform = reader.get_waveform_preview(1024);
                self.wav_reader = Some(reader);
                self.image_texture = None;
                self.last_decoded = None;
            }
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let pixels = self.decoder.decode(&reader.samples, &self.params);
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
            });
        });

        egui::SidePanel::left("waveform_panel")
            .resizable(true)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.label("📉 Waveform Preview");

                if !self.waveform.is_empty() {
                    let (width, height) = (ui.available_width(), 100.0);
                    let response = ui.allocate_response(egui::vec2(width, height), egui::Sense::hover());
                    let painter = ui.painter_at(response.rect);

                    let wave = &self.waveform;
                    let step = width / wave.len() as f32;
                    let mid = response.rect.center().y;

                    for (i, amp) in wave.iter().enumerate().skip(1) {
                        let x1 = response.rect.left() + step * (i - 1) as f32;
                        let x2 = response.rect.left() + step * i as f32;
                        let y1 = mid - amp * 50.0;
                        let y2 = mid - wave[i - 1] * 50.0;

                        painter.line_segment(
                            [egui::pos2(x1, y1), egui::pos2(x2, y2)],
                            egui::Stroke::new(1.0, egui::Color32::LIGHT_GREEN),
                        );
                    }
                } else {
                    ui.label("No waveform loaded.");
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.image_texture {
                ui.image(texture);
            } else {
                ui.label("🖼️ No image decoded yet.");
            }
        });

        egui::TopBottomPanel::bottom("debug_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(reader) = &self.wav_reader {
                    ui.label(format!("📦 {} samples @ {} Hz", reader.samples.len(), reader.sample_rate));
                } else {
                    ui.label("📦 No file loaded");
                }

                if let Some(pixels) = &self.last_decoded {
                    ui.label(format!("🖼️ Decoded size: {}x{}", 512, pixels.len() / 512));
                }
            });
        });
    }
}
