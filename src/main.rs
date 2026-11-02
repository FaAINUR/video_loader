mod app;
mod diagnostics;
mod jobs;
mod media;
mod models;
mod platform;
mod progress;
mod queue;
mod rules;
mod ytdlp;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VideoLoader")
            .with_inner_size([1180.0, 1000.0])
            .with_min_inner_size([850.0, 780.0]),
        ..Default::default()
    };

    eframe::run_native(
        "VideoLoader",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
