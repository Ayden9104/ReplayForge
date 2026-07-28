use crate::recorder::Recorder;
use eframe::egui;

#[derive(PartialEq)]
enum Page {
    Home,
    Clips,
    Settings,
}

pub struct ReplayForge {
    recorder: Recorder,
    page: Page,
}

impl Default for ReplayForge {
    fn default() -> Self {
        Self {
            recorder: Recorder::default(),
            page: Page::Home,
        }
    }
}

impl eframe::App for ReplayForge {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("sidebar")
            .default_width(160.0)
            .show(ctx, |ui| {
                ui.heading("ReplayForge");
                ui.separator();

                if ui.button("Home").clicked() {
                    self.page = Page::Home;
                }

                if ui.button("Clips").clicked() {
                    self.page = Page::Clips;
                }

                if ui.button("Settings").clicked() {
                    self.page = Page::Settings;
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Home => {
                ui.heading("Home");
                ui.separator();

                let status = if self.recorder.is_running() {
                    "Replay is running"
                } else {
                    "Replay is stopped"
                };

                ui.label(status);
                ui.add_space(10.0);

                let button_text = if self.recorder.is_running() {
                    "Stop Replay"
                } else {
                    "Start Replay"
                };

                if ui
                    .add_sized([200.0, 40.0], egui::Button::new(button_text))
                    .clicked()
                {
                    if self.recorder.is_running() {
                        self.recorder.stop();
                    } else {
                        self.recorder.start();
                    }
                }
            }

            Page::Clips => {
                ui.heading("Recent Clips");
                ui.separator();
                ui.label("No clips yet.");
            }

            Page::Settings => {
                ui.heading("Settings");
                ui.separator();
                ui.label("Settings coming soon!");
            }
        });
    }
}
