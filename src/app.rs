use eframe::egui;
use egui::TextureHandle;
use crate::audio::{WavReader, WaveformChannel};
use crate::sstv::{SstvDecoder, DecoderParams};
use crate::image_output::image_from_pixels;

pub struct VoyagerApp {
    wav_reader: Option<WavReader>,
    video_decoder: SstvDecoder,
    image_texture: Option<TextureHandle>,
    params: DecoderParams,
    last_decoded: Option<Vec<u8>>,
    selected_channel: WaveformChannel,
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
        }
    }
}

impl VoyagerApp {
    fn handle_load_wav(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .pick_file()
            .and_then(|path| WavReader::from_file(&path).ok())
        {
            self.wav_reader = Some(path);
            self.image_texture = None;
            self.last_decoded = None;
        }
    }

    fn handle_decode(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.wav_reader {
            let samples = reader.get_samples(self.selected_channel);
            let pixels = self.video_decoder.decode(samples, &self.params, reader.sample_rate);
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

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.image_texture {
                ui.image(texture);
            } else {
                ui.label("🖼️ No image decoded yet.");
            }
        });
    }
}
