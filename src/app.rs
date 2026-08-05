use crate::config::{
    codec_choices, hotkey_choices, path_display, set_autostart, Backend, Config,
};
use crate::detect::{probe_clip_meta, Detection};
use crate::host::open_path;
use crate::hotkeys::parse_hotkey;
use crate::recorder::Recorder;
use crate::tray::{TrayCommand, TrayHandle};
use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum Page {
    Home,
    Clips,
    Settings,
}

struct Toast {
    message: String,
    expires: Instant,
}

struct RenameState {
    path: PathBuf,
    text: String,
}

pub struct ReplayForge {
    config: Config,
    recorder: Arc<Mutex<Recorder>>,
    page: Page,
    textures: HashMap<PathBuf, egui::TextureHandle>,
    clip_meta: HashMap<PathBuf, (String, String)>,
    hotkey_manager: GlobalHotKeyManager,
    save_clip_hotkey_id: Option<u32>,
    detection: Detection,
    status: Option<Toast>,
    rename: Option<RenameState>,
    clips_dirty: bool,
    tray: Option<TrayHandle>,
    show_first_run: bool,
    settings_dirty: bool,
    quit_requested: bool,
}

impl ReplayForge {
    pub fn new() -> Self {
        let config = Config::load();
        let detection = Detection::refresh(config.backend);

        if config.display == "screen"
            && detection
                .monitors
                .iter()
                .any(|m| m.name != "screen")
        {
            // Keep "screen" as a valid default for first run.
        }

        let show_first_run = config.is_first_run();

        let hotkey_manager =
            GlobalHotKeyManager::new().expect("failed to create global hotkey manager");
        let save_clip_hotkey_id = register_hotkey(&hotkey_manager, &config.hotkey);

        let tray = match crate::tray::create_tray() {
            Ok(tray) => Some(tray),
            Err(error) => {
                eprintln!("Tray unavailable: {error}");
                None
            }
        };

        if let Err(error) = config.ensure_output_dir() {
            eprintln!("{error}");
        }

        // Sync autostart file with config on launch.
        let _ = set_autostart(config.autostart);

        Self {
            config,
            recorder: Arc::new(Mutex::new(Recorder::default())),
            page: Page::Home,
            textures: HashMap::new(),
            clip_meta: HashMap::new(),
            hotkey_manager,
            save_clip_hotkey_id,
            detection,
            status: None,
            rename: None,
            clips_dirty: true,
            tray,
            show_first_run,
            settings_dirty: false,
            quit_requested: false,
        }
    }

    fn toast(&mut self, message: impl Into<String>) {
        self.status = Some(Toast {
            message: message.into(),
            expires: Instant::now() + Duration::from_secs(4),
        });
    }

    fn persist_config(&mut self) {
        if let Err(error) = self.config.save() {
            self.toast(error);
        }
    }

    fn resolved_backend(&self) -> Option<crate::detect::ResolvedBackend> {
        self.detection.backend
    }

    fn start_replay(&mut self) {
        let Some(backend) = self.resolved_backend() else {
            self.toast(
                self.detection
                    .error
                    .clone()
                    .unwrap_or_else(|| "gpu-screen-recorder is not available".into()),
            );
            return;
        };

        let result = {
            let mut recorder = self.recorder.lock().unwrap();
            recorder.start(&self.config, backend)
        };
        match result {
            Ok(()) => self.toast("Replay buffer started"),
            Err(error) => self.toast(error),
        }
    }

    fn stop_replay(&mut self) {
        let result = self.recorder.lock().unwrap().stop();
        match result {
            Ok(()) => self.toast("Replay buffer stopped"),
            Err(error) => self.toast(error),
        }
    }

    fn save_clip_action(&mut self) {
        let result = self.recorder.lock().unwrap().save_clip();
        match result {
            Ok(path) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("clip")
                    .to_string();
                self.toast(format!("Saved {name}"));
                self.clips_dirty = true;
                self.textures.clear();
                self.clip_meta.clear();
                self.page = Page::Clips;
            }
            Err(error) => self.toast(error),
        }
    }

    fn apply_hotkey(&mut self) {
        // Recreate manager so the previous binding is fully released.
        match GlobalHotKeyManager::new() {
            Ok(manager) => {
                self.hotkey_manager = manager;
                self.save_clip_hotkey_id =
                    register_hotkey(&self.hotkey_manager, &self.config.hotkey);
                if self.save_clip_hotkey_id.is_none() {
                    self.toast(format!(
                        "Could not register hotkey {}",
                        self.config.hotkey
                    ));
                }
            }
            Err(error) => self.toast(format!("Hotkey manager error: {error}")),
        }
    }

    fn apply_capture_settings(&mut self) {
        self.persist_config();
        let running = self.recorder.lock().unwrap().is_running();
        if running {
            if let Some(backend) = self.resolved_backend() {
                let result = {
                    let mut recorder = self.recorder.lock().unwrap();
                    recorder.restart(&self.config, backend)
                };
                match result {
                    Ok(()) => self.toast("Replay restarted with new settings"),
                    Err(error) => self.toast(error),
                }
            }
        }
        self.settings_dirty = false;
    }

    fn finish_first_run(&mut self) {
        self.config.first_run_complete = true;
        if let Err(error) = self.config.ensure_output_dir() {
            self.toast(error);
            return;
        }
        self.persist_config();
        self.show_first_run = false;
        self.toast("Setup complete — start replay from Home");
    }
}

fn register_hotkey(manager: &GlobalHotKeyManager, spec: &str) -> Option<u32> {
    let hotkey = parse_hotkey(spec)?;
    let id = hotkey.id();
    match manager.register(hotkey) {
        Ok(()) => {
            println!("Registered hotkey {spec}");
            Some(id)
        }
        Err(error) => {
            eprintln!("Hotkey registration failed for {spec}: {error}");
            None
        }
    }
}

impl eframe::App for ReplayForge {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray commands
        let tray_cmds: Vec<TrayCommand> = self
            .tray
            .as_ref()
            .map(|tray| {
                let running = self.recorder.lock().unwrap().is_running();
                tray.set_running(running);
                tray.poll()
            })
            .unwrap_or_default();

        for cmd in tray_cmds {
            match cmd {
                TrayCommand::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayCommand::Hide => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                TrayCommand::SaveClip => self.save_clip_action(),
                TrayCommand::ToggleReplay => {
                    let running = self.recorder.lock().unwrap().is_running();
                    if running {
                        self.stop_replay();
                    } else {
                        self.start_replay();
                    }
                }
                TrayCommand::Quit => {
                    self.quit_requested = true;
                }
            }
        }

        if self.quit_requested {
            let _ = self.recorder.lock().unwrap().stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Minimize to tray on close
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.config.minimize_to_tray && self.tray.is_some() && !self.quit_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                let _ = self.recorder.lock().unwrap().stop();
            }
        }

        // Global hotkeys
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if let Some(id) = self.save_clip_hotkey_id {
                if event.id == id && event.state == HotKeyState::Pressed {
                    self.save_clip_action();
                }
            }
        }

        if let Some(toast) = &self.status {
            if Instant::now() >= toast.expires {
                self.status = None;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(50));

        if self.show_first_run {
            self.ui_first_run(ctx);
            return;
        }

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if let Some(toast) = &self.status {
                ui.label(&toast.message);
            } else if let Some(error) = &self.detection.error {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), error);
            } else {
                ui.label(format!(
                    "Hotkey {} · {} · {}s buffer",
                    self.config.hotkey, self.config.display, self.config.buffer_seconds
                ));
            }
        });

        egui::SidePanel::left("sidebar")
            .default_width(160.0)
            .show(ctx, |ui| {
                ui.heading("ReplayForge");
                ui.separator();

                if ui
                    .selectable_label(self.page == Page::Home, "Home")
                    .clicked()
                {
                    self.page = Page::Home;
                }
                if ui
                    .selectable_label(self.page == Page::Clips, "Clips")
                    .clicked()
                {
                    self.page = Page::Clips;
                }
                if ui
                    .selectable_label(self.page == Page::Settings, "Settings")
                    .clicked()
                {
                    self.page = Page::Settings;
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Home => self.ui_home(ui),
            Page::Clips => self.ui_clips(ui),
            Page::Settings => self.ui_settings(ui),
        });
    }
}

impl ReplayForge {
    fn ui_first_run(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("Welcome to ReplayForge");
                ui.label("A bare-bones Medal-like instant replay for Linux.");
                ui.add_space(20.0);
            });

            ui.group(|ui| {
                ui.set_max_width(520.0);
                ui.heading("1. Choose display");
                egui::ComboBox::from_id_salt("first_run_display")
                    .selected_text(&self.config.display)
                    .show_ui(ui, |ui| {
                        for monitor in &self.detection.monitors {
                            ui.selectable_value(
                                &mut self.config.display,
                                monitor.name.clone(),
                                monitor.label(),
                            );
                        }
                    });

                ui.add_space(12.0);
                ui.heading("2. Clips folder");
                ui.horizontal(|ui| {
                    ui.label(path_display(&self.config.output_dir));
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.config.output_dir = path;
                        }
                    }
                });

                ui.add_space(12.0);
                if let Some(error) = &self.detection.error {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), error);
                } else {
                    let backend = match self.detection.backend {
                        Some(crate::detect::ResolvedBackend::Host) => "host gpu-screen-recorder",
                        Some(crate::detect::ResolvedBackend::Flatpak) => "Flatpak gpu-screen-recorder",
                        None => "not found",
                    };
                    ui.label(format!("Capture backend: {backend}"));
                }

                ui.add_space(16.0);
                if ui
                    .add_sized([200.0, 36.0], egui::Button::new("Get started"))
                    .clicked()
                {
                    self.finish_first_run();
                }
            });
        });
    }

    fn ui_home(&mut self, ui: &mut egui::Ui) {
        ui.heading("Home");
        ui.separator();

        let replay_running = self.recorder.lock().unwrap().is_running();

        ui.label(if replay_running {
            "Replay is running"
        } else {
            "Replay is stopped"
        });

        ui.add_space(6.0);
        ui.label(format!(
            "Display: {} · {} FPS · {}s · {}",
            self.config.display, self.config.fps, self.config.buffer_seconds, self.config.codec
        ));

        if let Some(error) = self.recorder.lock().unwrap().last_error() {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 80), error.to_string());
        }

        ui.add_space(12.0);

        let button_text = if replay_running {
            "Stop Replay"
        } else {
            "Start Replay"
        };

        if ui
            .add_sized([220.0, 40.0], egui::Button::new(button_text))
            .clicked()
        {
            if replay_running {
                self.stop_replay();
            } else {
                self.start_replay();
            }
        }

        if replay_running {
            ui.add_space(10.0);
            if ui
                .add_sized([220.0, 40.0], egui::Button::new("Save Clip"))
                .clicked()
            {
                self.save_clip_action();
            }
            ui.add_space(6.0);
            ui.label(format!("Or press {}", self.config.hotkey));
        }
    }

    fn ui_clips(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Clips");
            if ui.button("Refresh").clicked() {
                self.textures.clear();
                self.clip_meta.clear();
                self.clips_dirty = true;
            }
            if ui.button("Open Folder").clicked() {
                let _ = self.config.ensure_output_dir();
                open_path(&self.config.output_dir);
            }
        });
        ui.separator();

        let clips_folder = self.config.output_dir.clone();
        let _ = self.clips_dirty;
        self.clips_dirty = false;

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
                    ui.label("No clips yet. Start replay and hit Save Clip.");
                    return;
                }

                let mut open_path_req: Option<PathBuf> = None;
                let mut delete_req: Option<(PathBuf, PathBuf)> = None;
                let mut start_rename: Option<PathBuf> = None;
                let mut finish_rename: Option<(PathBuf, String)> = None;
                let mut cancel_rename = false;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("clips_grid")
                        .num_columns(2)
                        .spacing([20.0, 20.0])
                        .show(ui, |ui| {
                            for (index, clip_path) in clips.iter().enumerate() {
                                let clip_name = clip_path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("Unknown clip")
                                    .to_string();

                                let (duration_label, size_label) = if let Some(cached) =
                                    self.clip_meta.get(clip_path)
                                {
                                    cached.clone()
                                } else {
                                    let probed = probe_clip_meta(clip_path);
                                    self.clip_meta.insert(clip_path.clone(), probed.clone());
                                    probed
                                };

                                let thumbnail_path = clip_path.with_extension("png");

                                ui.group(|ui| {
                                    ui.set_min_width(360.0);
                                    ui.vertical(|ui| {
                                        if thumbnail_path.exists() {
                                            if !self.textures.contains_key(&thumbnail_path) {
                                                if let Ok(image) = image::open(&thumbnail_path) {
                                                    let size = [
                                                        image.width() as usize,
                                                        image.height() as usize,
                                                    ];
                                                    let image_buffer = image.to_rgba8();
                                                    let texture = ui.ctx().load_texture(
                                                        thumbnail_path.to_string_lossy(),
                                                        egui::ColorImage::from_rgba_unmultiplied(
                                                            size,
                                                            &image_buffer,
                                                        ),
                                                        Default::default(),
                                                    );
                                                    self.textures
                                                        .insert(thumbnail_path.clone(), texture);
                                                }
                                            }

                                            if let Some(texture) =
                                                self.textures.get(&thumbnail_path)
                                            {
                                                let response = ui.add(
                                                    egui::Image::new(texture).fit_to_exact_size(
                                                        egui::vec2(320.0, 180.0),
                                                    ),
                                                );
                                                if response.clicked() {
                                                    open_path_req = Some(clip_path.clone());
                                                }
                                            }
                                        } else {
                                            ui.label("No thumbnail");
                                        }

                                        ui.add_space(5.0);

                                        let renaming = self
                                            .rename
                                            .as_ref()
                                            .is_some_and(|r| r.path == *clip_path);

                                        if renaming {
                                            if let Some(state) = self.rename.as_mut() {
                                                ui.text_edit_singleline(&mut state.text);
                                                ui.horizontal(|ui| {
                                                    if ui.button("Save").clicked() {
                                                        finish_rename = Some((
                                                            state.path.clone(),
                                                            state.text.clone(),
                                                        ));
                                                    }
                                                    if ui.button("Cancel").clicked() {
                                                        cancel_rename = true;
                                                    }
                                                });
                                            }
                                        } else {
                                            ui.label(&clip_name);
                                            ui.label(format!(
                                                "{duration_label} · {size_label}"
                                            ));
                                            ui.horizontal(|ui| {
                                                if ui.button("Open").clicked() {
                                                    open_path_req = Some(clip_path.clone());
                                                }
                                                if ui.button("Rename").clicked() {
                                                    start_rename = Some(clip_path.clone());
                                                }
                                                if ui.button("Delete").clicked() {
                                                    delete_req = Some((
                                                        clip_path.clone(),
                                                        thumbnail_path.clone(),
                                                    ));
                                                }
                                            });
                                        }
                                    });
                                });

                                if index % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                });

                if let Some(path) = open_path_req {
                    open_path(&path);
                }

                if let Some(path) = start_rename {
                    let text = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("clip")
                        .to_string();
                    self.rename = Some(RenameState { path, text });
                }

                if cancel_rename {
                    self.rename = None;
                }

                if let Some((old_path, new_stem)) = finish_rename {
                    let new_stem = new_stem.trim();
                    if !new_stem.is_empty() {
                        let new_path = old_path.with_file_name(format!("{new_stem}.mp4"));
                        let old_thumb = old_path.with_extension("png");
                        let new_thumb = new_path.with_extension("png");
                        match fs::rename(&old_path, &new_path) {
                            Ok(()) => {
                                if old_thumb.exists() {
                                    let _ = fs::rename(&old_thumb, &new_thumb);
                                }
                                self.textures.clear();
                                self.clip_meta.clear();
                                self.toast("Clip renamed");
                            }
                            Err(error) => self.toast(format!("Rename failed: {error}")),
                        }
                    }
                    self.rename = None;
                }

                if let Some((clip_path, thumbnail_path)) = delete_req {
                    if let Err(error) = fs::remove_file(&clip_path) {
                        self.toast(format!("Failed to delete clip: {error}"));
                    } else {
                        if thumbnail_path.exists() {
                            let _ = fs::remove_file(&thumbnail_path);
                        }
                        self.textures.clear();
                        self.clip_meta.clear();
                        self.toast("Clip deleted");
                    }
                }
            }
            Err(error) => {
                ui.label(format!("Could not open clips folder: {error}"));
            }
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Capture");

            ui.horizontal(|ui| {
                ui.label("Display");
                egui::ComboBox::from_id_salt("settings_display")
                    .selected_text(&self.config.display)
                    .show_ui(ui, |ui| {
                        for monitor in &self.detection.monitors {
                            if ui
                                .selectable_value(
                                    &mut self.config.display,
                                    monitor.name.clone(),
                                    monitor.label(),
                                )
                                .changed()
                            {
                                self.settings_dirty = true;
                            }
                        }
                    });
                if ui.button("Refresh").clicked() {
                    self.detection = Detection::refresh(self.config.backend);
                    self.toast("Displays refreshed");
                }
            });

            ui.horizontal(|ui| {
                ui.label("FPS");
                if ui
                    .add(egui::DragValue::new(&mut self.config.fps).range(15..=240))
                    .changed()
                {
                    self.settings_dirty = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Buffer (seconds)");
                if ui
                    .add(egui::DragValue::new(&mut self.config.buffer_seconds).range(5..=600))
                    .changed()
                {
                    self.settings_dirty = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Codec");
                egui::ComboBox::from_id_salt("settings_codec")
                    .selected_text(&self.config.codec)
                    .show_ui(ui, |ui| {
                        for codec in codec_choices() {
                            if ui
                                .selectable_value(
                                    &mut self.config.codec,
                                    (*codec).to_string(),
                                    *codec,
                                )
                                .changed()
                            {
                                self.settings_dirty = true;
                            }
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label("Backend");
                let backend_label = match self.config.backend {
                    Backend::Auto => "Auto",
                    Backend::Host => "Host",
                    Backend::Flatpak => "Flatpak",
                };
                egui::ComboBox::from_id_salt("settings_backend")
                    .selected_text(backend_label)
                    .show_ui(ui, |ui| {
                        for (label, value) in [
                            ("Auto", Backend::Auto),
                            ("Host", Backend::Host),
                            ("Flatpak", Backend::Flatpak),
                        ] {
                            if ui
                                .selectable_value(&mut self.config.backend, value, label)
                                .changed()
                            {
                                self.detection = Detection::refresh(self.config.backend);
                                self.settings_dirty = true;
                            }
                        }
                    });
            });
            ui.label(format!(
                "Detected: host={}, flatpak={}",
                self.detection.host_gsr, self.detection.flatpak_gsr
            ));

            ui.add_space(12.0);
            ui.heading("Output");
            ui.horizontal(|ui| {
                ui.label(path_display(&self.config.output_dir));
                if ui.button("Browse…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.config.output_dir = path;
                        self.settings_dirty = true;
                    }
                }
            });

            ui.add_space(12.0);
            ui.heading("Hotkey");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("settings_hotkey")
                    .selected_text(&self.config.hotkey)
                    .show_ui(ui, |ui| {
                        for key in hotkey_choices() {
                            if ui
                                .selectable_value(
                                    &mut self.config.hotkey,
                                    (*key).to_string(),
                                    *key,
                                )
                                .changed()
                            {
                                self.apply_hotkey();
                                self.persist_config();
                            }
                        }
                    });
            });
            ui.label("Note: global hotkeys require X11 (or XWayland).");

            ui.add_space(12.0);
            ui.heading("Desktop");
            if ui
                .checkbox(&mut self.config.autostart, "Start ReplayForge on login")
                .changed()
            {
                match set_autostart(self.config.autostart) {
                    Ok(()) => self.persist_config(),
                    Err(error) => self.toast(error),
                }
            }
            if ui
                .checkbox(
                    &mut self.config.minimize_to_tray,
                    "Minimize to tray on close",
                )
                .changed()
            {
                self.persist_config();
            }

            ui.add_space(16.0);
            if self.settings_dirty {
                if ui
                    .add_sized([220.0, 36.0], egui::Button::new("Apply & Save"))
                    .clicked()
                {
                    self.apply_capture_settings();
                }
            } else if ui.button("Save settings").clicked() {
                self.persist_config();
                self.toast("Settings saved");
            }
        });
    }
}
