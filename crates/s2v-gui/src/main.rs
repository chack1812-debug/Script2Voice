#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio_play;
mod fonts;
mod jobs;
mod history;
mod logbuf;
mod presets;
mod scene_line;
mod script_model;

fn main() -> eframe::Result {
    let log = logbuf::LogBuffer::new(500);
    logbuf::init_tracing(log.clone());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Script2Voice",
        options,
        Box::new(move |cc| {
            fonts::install_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(app::App::new(cc, log)))
        }),
    )
}
