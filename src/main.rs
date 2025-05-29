#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Voyager Golden Record Explorer",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::<MyApp>::default())
        }),
    )
}

#[derive(Default)]
struct MyApp {}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_pixels_per_point(2.0);


        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Voyager Golden Record Explorer");

            if ui.button("Quit").clicked() {
                std::process::exit(0);
            };

            if ui.button("Load Wav").clicked() {
                std::process::exit(0);
            };


            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("../assets/voyager-golden-record-cover.jpg"))
                        .corner_radius(10),
                );
            });
        });
    }
}