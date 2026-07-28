mod app;
mod recorder;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "ReplayForge",
        options,
        Box::new(|_cc| Ok(Box::new(app::ReplayForge::default()))),
    )
}
