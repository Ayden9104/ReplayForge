mod app;
mod audio_volume;
mod clips;
mod config;
mod detect;
mod host;
mod hotkeys;
mod recorder;
mod sfx;
mod share;
mod share_links;
mod theme;
mod tray;
mod trim_playback;
mod update;

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
        Box::new(|cc| {
            let app = app::ReplayForge::new();
            app.apply_configured_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

use eframe::egui;
