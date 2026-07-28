use crate::recorder::Recorder;
use eframe::egui;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
                        let mut clips: Vec<PathBuf> = entries
                            .filter_map(Result::ok)
                            .map(|entry| entry.path())
                            .filter(|path| {
                                path.extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
                            })
                            .collect();

                        clips.sort();
                        clips.reverse();

                        if clips.is_empty() {
                            ui.label("No clips yet.");
                        } else {
                            for clip_path in clips {
                                let clip_name = clip_path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("Unknown clip");

                                ui.horizontal(|ui| {
                                    ui.label(clip_name);

                                    if ui.button("Open").clicked() {
                                        let result = Command::new("flatpak-spawn")
                                            .arg("--host")
                                            .arg("xdg-open")
                                            .arg(&clip_path)
                                            .spawn();

                                        if let Err(error) = result {
                                            eprintln!("Failed to open clip: {error}");
                                        }
                                    }
                                });
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
