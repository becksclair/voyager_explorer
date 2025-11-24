use crate::sstv::DecoderMode;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Debug, Clone, PartialEq)]
pub enum BatchStatus {
    Pending,
    Processing,
    Done,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct BatchItem {
    pub path: PathBuf,
    pub status: BatchStatus,
}

pub struct BatchPanel {
    pub visible: bool,
    pub queue: Vec<BatchItem>,
    pub output_dir: Option<PathBuf>,
    pub selected_mode: DecoderMode,
    pub is_processing: bool,
    pub current_index: usize,
    pub progress: f32,
    pub cancel_flag: Option<Arc<AtomicBool>>,
}

impl Default for BatchPanel {
    fn default() -> Self {
        Self {
            visible: false,
            queue: Vec::new(),
            output_dir: None,
            selected_mode: DecoderMode::BinaryGrayscale,
            is_processing: false,
            current_index: 0,
            progress: 0.0,
            cancel_flag: None,
        }
    }
}

impl BatchPanel {
    pub fn draw(&mut self, ctx: &egui::Context) {
        if !self.visible {
            return;
        }

        let mut visible = self.visible;
        egui::Window::new("📦 Batch Processing")
            .open(&mut visible)
            .resize(|r| r.default_size([600.0, 400.0]))
            .show(ctx, |ui| {
                self.draw_content(ui);
            });
        self.visible = visible;
    }

    fn draw_content(&mut self, ui: &mut egui::Ui) {
        ui.heading("Batch Processing");
        ui.separator();

        // Configuration Section
        ui.group(|ui| {
            ui.horizontal(|ui| {
                if ui.button("📂 Add Files...").clicked() {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter("WAV", &["wav"])
                        .pick_files()
                    {
                        for path in paths {
                            if !self.queue.iter().any(|item| item.path == path) {
                                self.queue.push(BatchItem {
                                    path,
                                    status: BatchStatus::Pending,
                                });
                            }
                        }
                    }
                }

                if ui.button("📂 Select Output Dir...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_dir = Some(path);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Output:");
                if let Some(dir) = &self.output_dir {
                    ui.monospace(dir.to_string_lossy());
                } else {
                    ui.colored_label(egui::Color32::RED, "Not selected");
                }
            });

            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_label("")
                    .selected_text(match self.selected_mode {
                        DecoderMode::BinaryGrayscale => "Binary (B/W)",
                        DecoderMode::PseudoColor => "PseudoColor",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.selected_mode,
                            DecoderMode::BinaryGrayscale,
                            "Binary (B/W)",
                        );
                        ui.selectable_value(
                            &mut self.selected_mode,
                            DecoderMode::PseudoColor,
                            "PseudoColor",
                        );
                    });
            });
        });

        ui.add_space(10.0);

        // Queue Section
        ui.heading(format!("Queue ({})", self.queue.len()));
        ui.separator();

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                let mut to_remove = None;
                for (i, item) in self.queue.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}.", i + 1));
                        ui.monospace(item.path.file_name().unwrap_or_default().to_string_lossy());

                        match &item.status {
                            BatchStatus::Pending => {
                                ui.label("⏳ Pending");
                                if !self.is_processing && ui.button("✖").clicked() {
                                    to_remove = Some(i);
                                }
                            }
                            BatchStatus::Processing => {
                                ui.spinner();
                                ui.label("Processing...");
                            }
                            BatchStatus::Done => {
                                ui.colored_label(egui::Color32::GREEN, "✅ Done");
                            }
                            BatchStatus::Error(e) => {
                                ui.colored_label(egui::Color32::RED, format!("❌ Error: {}", e));
                            }
                        }
                    });
                }

                if let Some(i) = to_remove {
                    self.queue.remove(i);
                }
            });

        ui.add_space(10.0);
        ui.separator();

        // Actions
        ui.horizontal(|ui| {
            let can_start =
                !self.queue.is_empty() && self.output_dir.is_some() && !self.is_processing;

            if ui
                .add_enabled(can_start, egui::Button::new("▶ Start Batch"))
                .clicked()
            {
                self.start_processing();
            }

            if self.is_processing {
                if ui.button("⏹ Stop").clicked() {
                    // Set cancellation flag
                    if let Some(flag) = &self.cancel_flag {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }

                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
            }
        });
    }

    fn start_processing(&mut self) {
        self.is_processing = true;
        self.current_index = 0;
        self.progress = 0.0;
        self.cancel_flag = Some(Arc::new(AtomicBool::new(false)));

        // Reset statuses
        for item in &mut self.queue {
            item.status = BatchStatus::Pending;
        }

        // Actual processing logic will be driven by the main app loop or a thread
        // For now, we just set the flag.
    }
}
