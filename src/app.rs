use crate::recorder::Recorder;
use eframe::egui;
use std::fs;
use std::path::PathBuf;

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

                let replay_running = self.recorder.is_running();

                let status = if replay_running {
                    "Replay is running"
                } else {
                    "Replay is stopped"
                };

                ui.label(status);
                ui.add_space(10.0);

                let button_text = if replay_running {
                    "Stop Replay"
                } else {
                    "Start Replay"
                };

                if ui
                    .add_sized([200.0, 40.0], egui::Button::new(button_text))
                    .clicked()
                {
                    if replay_running {
                        self.recorder.stop();
                    } else {
                        self.recorder.start();
                    }
                }

                if replay_running {
                    ui.add_space(10.0);

                    if ui
                        .add_sized([200.0, 40.0], egui::Button::new("Save Clip"))
                        .clicked()
                    {
                        self.recorder.save_clip();
                    }
                }
            }

            Page::Clips => {
                ui.heading("Clips");
                ui.separator();

                let clips_folder = PathBuf::from("/var/home/ayden9104/Videos/ReplayForge");

                match fs::read_dir(&clips_folder) {
                    Ok(entries) => {
                        let mut clip_names: Vec<String> = entries
                            .filter_map(Result::ok)
                            .filter_map(|entry| {
                                let path = entry.path();

                                let is_mp4 = path
                                    .extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));

                                if is_mp4 {
                                    path.file_name()
                                        .and_then(|name| name.to_str())
                                        .map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect();

                        clip_names.sort();
                        clip_names.reverse();

                        if clip_names.is_empty() {
                            ui.label("No clips yet.");
                        } else {
                            for clip_name in clip_names {
                                ui.label(clip_name);
                            }
                        }
                    }

                    Err(error) => {
                        ui.label(format!("Could not open clips folder: {error}"));
                    }
                }
            }

            Page::Settings => {
                ui.heading("Settings");
                ui.separator();
                ui.label("Settings coming soon!");
            }
        });
    }
}
