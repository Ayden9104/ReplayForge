mod app;
mod config;
mod detect;
mod host;
mod hotkeys;
mod recorder;
mod tray;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("ReplayForge"),
        ..Default::default()
    };

    eframe::run_native(
        "ReplayForge",
        options,
        Box::new(|_cc| Ok(Box::new(app::ReplayForge::new()))),
    )
}

use eframe::egui;
