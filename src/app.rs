//! ReplayForge egui application shell (home, clips, settings, hotkeys).
use crate::clips::{extract_filmstrip_jpeg, extract_frame_png, filmstrip_frame_count, trim_clip};
use crate::config::{
    Backend, Config, SystemAudioMode, codec_choices, hotkey_choices, path_display, quality_choices,
    set_autostart,
};
use crate::detect::{
    Detection, clip_duration_secs, format_duration, friendly_audio_app_label, probe_clip_meta,
};
use crate::host::{notify_desktop, open_path};
use crate::hotkeys::HotkeyService;
use crate::recorder::Recorder;
use crate::theme;
use crate::tray::{TrayCommand, TrayHandle};
use crate::trim_playback::{TrimFrame, TrimPlayback};
use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[derive(PartialEq)]
enum Page {
    Home,
    Clips,
    Settings,
    Trim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipSort {
    Newest,
    Name,
    Largest,
}

struct Toast {
    message: String,
    expires: Instant,
}

struct RenameState {
    path: PathBuf,
    text: String,
}

#[derive(Clone)]
struct TrimState {
    path: PathBuf,
    duration_secs: f64,
    start_secs: f64,
    end_secs: f64,
    preview_secs: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrimHandle {
    Start,
    End,
    Playhead,
}

pub struct ReplayForge {
    config: Config,
    recorder: Arc<Mutex<Recorder>>,
    page: Page,
    textures: HashMap<PathBuf, egui::TextureHandle>,
    clip_meta: HashMap<PathBuf, (String, String)>,
    hotkeys: HotkeyService,
    detection: Detection,
    status: Option<Toast>,
    rename: Option<RenameState>,
    clips_dirty: bool,
    tray: Option<TrayHandle>,
    show_first_run: bool,
    settings_dirty: bool,
    quit_requested: bool,
    saving: bool,
    save_rx: Option<Receiver<Result<PathBuf, String>>>,
    trim: Option<TrimState>,
    trimming: bool,
    trim_rx: Option<Receiver<Result<PathBuf, String>>>,
    trim_preview_rx: Option<Receiver<Result<(f64, Vec<u8>), String>>>,
    trim_preview_pending: bool,
    trim_preview_last_request: Instant,
    trim_preview_texture: Option<egui::TextureHandle>,
    trim_loaded_preview: Option<f64>,
    trim_preview_error: Option<String>,
    trim_drag_handle: Option<TrimHandle>,
    trim_playback: Option<TrimPlayback>,
    trim_play_start: Option<Instant>,
    trim_filmstrip_texture: Option<egui::TextureHandle>,
    trim_filmstrip_rx: Option<Receiver<Result<Vec<u8>, String>>>,
    trim_filmstrip_pending: bool,
    trim_filmstrip_width: f32,
    trim_filmstrip_target_width: f32,
    clip_sort: ClipSort,
    clip_filter: String,
}

impl ReplayForge {
    pub fn new() -> Self {
        let config = Config::load();
        let detection = Detection::refresh(config.backend);

        if config.display == "screen" && detection.monitors.iter().any(|m| m.name != "screen") {
            // Keep "screen" as a valid default for first run.
        }

        let show_first_run = config.is_first_run();
        let hotkeys = HotkeyService::start(&config.hotkey, config.portal_hotkey_enabled);

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

        let mut app = Self {
            config,
            recorder: Arc::new(Mutex::new(Recorder::default())),
            page: Page::Home,
            textures: HashMap::new(),
            clip_meta: HashMap::new(),
            hotkeys,
            detection,
            status: None,
            rename: None,
            clips_dirty: true,
            tray,
            show_first_run,
            settings_dirty: false,
            quit_requested: false,
            saving: false,
            save_rx: None,
            trim: None,
            trimming: false,
            trim_rx: None,
            trim_preview_rx: None,
            trim_preview_pending: false,
            trim_preview_last_request: Instant::now(),
            trim_preview_texture: None,
            trim_loaded_preview: None,
            trim_preview_error: None,
            trim_drag_handle: None,
            trim_playback: None,
            trim_play_start: None,
            trim_filmstrip_texture: None,
            trim_filmstrip_rx: None,
            trim_filmstrip_pending: false,
            trim_filmstrip_width: 0.0,
            trim_filmstrip_target_width: 0.0,
            clip_sort: ClipSort::Newest,
            clip_filter: String::new(),
        };

        if app.config.auto_start_replay && !app.show_first_run {
            app.start_replay();
        }

        app
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
        if self.saving {
            self.toast("Save already in progress…");
            return;
        }

        let running = self.recorder.lock().unwrap().is_running();
        if !running {
            let msg = self
                .recorder
                .lock()
                .unwrap()
                .last_error()
                .unwrap_or("Cannot save clip: replay is not running. Press Start Replay first.")
                .to_string();
            self.toast(msg);
            return;
        }

        self.saving = true;
        self.toast("Saving clip…");

        let recorder = Arc::clone(&self.recorder);
        let output_dir = self.config.output_dir.clone();
        let (tx, rx) = mpsc::channel();
        self.save_rx = Some(rx);

        thread::spawn(move || {
            let result = recorder.lock().unwrap().save_clip(&output_dir);
            let _ = tx.send(result);
        });
    }

    fn poll_save_result(&mut self) {
        let Some(rx) = &self.save_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.save_rx = None;
                self.saving = false;
                match result {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("clip")
                            .to_string();
                        let msg = format!("Saved {name}");
                        self.toast(msg.clone());
                        notify_desktop("Clip saved", &format!("{name}\n{}", path.display()));
                        self.clips_dirty = true;
                        self.textures.clear();
                        self.clip_meta.clear();
                        self.page = Page::Clips;
                    }
                    Err(error) => self.toast(error),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.save_rx = None;
                self.saving = false;
                self.toast("Save failed: worker disconnected");
            }
        }
    }

    fn open_trim(&mut self, path: PathBuf) {
        let Some(duration) = clip_duration_secs(&path) else {
            self.toast("Could not read clip duration for trim");
            return;
        };
        if duration < 0.5 {
            self.toast("Clip is too short to trim");
            return;
        }
        self.clear_trim_previews();
        self.trim = Some(TrimState {
            path,
            duration_secs: duration,
            start_secs: 0.0,
            end_secs: duration,
            preview_secs: 0.0,
        });
        self.trim_preview_last_request = Instant::now() - Duration::from_millis(200);
        self.trim_preview_error = None;
        self.page = Page::Trim;
        self.trim_filmstrip_width = 0.0;
        self.trim_filmstrip_target_width = 0.0;
    }

    fn schedule_trim_filmstrip(&mut self, timeline_width: f32) {
        let Some(state) = &self.trim else {
            return;
        };
        let path = state.path.clone();
        let duration = state.duration_secs;
        let frame_count = filmstrip_frame_count(timeline_width);
        self.trim_filmstrip_target_width = timeline_width;
        let (tx, rx) = mpsc::channel();
        self.trim_filmstrip_rx = Some(rx);
        self.trim_filmstrip_pending = true;

        thread::spawn(move || {
            let result = extract_filmstrip_jpeg(&path, duration, frame_count);
            let _ = tx.send(result);
        });
    }

    fn poll_trim_filmstrip(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.trim_filmstrip_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.trim_filmstrip_rx = None;
                    self.trim_filmstrip_pending = false;
                    match result {
                        Ok(jpeg) => {
                            if let Some(tex) = Self::load_trim_texture(ctx, "trim_filmstrip", &jpeg)
                            {
                                self.trim_filmstrip_texture = Some(tex);
                                self.trim_filmstrip_width = self.trim_filmstrip_target_width;
                            }
                        }
                        Err(error) => {
                            eprintln!("Filmstrip: {error}");
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.trim_filmstrip_rx = None;
                    self.trim_filmstrip_pending = false;
                }
            }
        }
    }

    fn stop_trim_playback(&mut self) {
        if let Some(mut playback) = self.trim_playback.take() {
            playback.stop();
        }
        self.trim_play_start = None;
    }

    fn trim_is_playing(&self) -> bool {
        self.trim_playback.as_ref().is_some_and(|pb| pb.is_active())
    }

    fn toggle_trim_playback(&mut self) {
        if self.trim_is_playing() {
            self.stop_trim_playback();
            return;
        }
        let Some(state) = self.trim.clone() else {
            return;
        };
        if state.end_secs <= state.start_secs {
            self.toast("Invalid trim range");
            return;
        }
        match TrimPlayback::start(&state.path, state.start_secs, state.end_secs) {
            Ok(playback) => {
                if !playback.audio_enabled {
                    self.toast("Audio unavailable — playing video only");
                }
                self.trim_play_start = Some(Instant::now());
                self.trim_playback = Some(playback);
            }
            Err(error) => self.toast(error),
        }
    }

    fn cancel_trim(&mut self) {
        if self.trimming {
            return;
        }
        self.trim = None;
        self.clear_trim_previews();
        self.page = Page::Clips;
    }

    fn clear_trim_previews(&mut self) {
        self.stop_trim_playback();
        self.trim_preview_texture = None;
        self.trim_loaded_preview = None;
        self.trim_preview_rx = None;
        self.trim_preview_pending = false;
        self.trim_preview_error = None;
        self.trim_drag_handle = None;
        self.trim_filmstrip_texture = None;
        self.trim_filmstrip_rx = None;
        self.trim_filmstrip_pending = false;
        self.trim_filmstrip_width = 0.0;
        self.trim_filmstrip_target_width = 0.0;
    }

    fn trim_preview_stale(&self) -> bool {
        let Some(state) = &self.trim else {
            return false;
        };
        match self.trim_loaded_preview {
            None => true,
            Some(loaded) => (loaded - state.preview_secs).abs() > 0.05,
        }
    }

    fn schedule_trim_preview(&mut self) {
        if self.trim.is_none() || self.trim_preview_pending || self.trim_is_playing() {
            return;
        }
        if Instant::now().duration_since(self.trim_preview_last_request)
            < Duration::from_millis(200)
        {
            return;
        }
        if !self.trim_preview_stale() {
            return;
        }
        let Some(state) = self.trim.clone() else {
            return;
        };
        let time_secs = state.preview_secs;
        let path = state.path.clone();
        let (tx, rx) = mpsc::channel();
        self.trim_preview_rx = Some(rx);
        self.trim_preview_pending = true;
        self.trim_preview_last_request = Instant::now();

        thread::spawn(move || {
            let result = extract_frame_png(&path, time_secs).map(|png| (time_secs, png));
            let _ = tx.send(result);
        });
    }

    fn load_trim_texture(ctx: &egui::Context, id: &str, png: &[u8]) -> Option<egui::TextureHandle> {
        let img = image::load_from_memory(png).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        Some(ctx.load_texture(
            id,
            egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
            Default::default(),
        ))
    }

    fn load_trim_texture_rgba(
        ctx: &egui::Context,
        id: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<egui::TextureHandle> {
        let size = [width as usize, height as usize];
        Some(ctx.load_texture(
            id,
            egui::ColorImage::from_rgba_unmultiplied(size, rgba),
            Default::default(),
        ))
    }

    fn poll_trim_playback(&mut self, ctx: &egui::Context) {
        if self.trim_playback.is_none() {
            return;
        }

        let elapsed = self
            .trim_play_start
            .map(|start| Instant::now().duration_since(start).as_secs_f64())
            .unwrap_or(0.0);

        let should_stop = {
            let Some(playback) = &self.trim_playback else {
                return;
            };
            elapsed >= playback.selection_secs
        };

        if should_stop {
            self.stop_trim_playback();
            return;
        }

        if let Some(playback) = &self.trim_playback {
            if let Some(state) = &mut self.trim {
                state.preview_secs = (playback.start_secs + elapsed)
                    .min(playback.start_secs + playback.selection_secs);
            }

            let mut latest: Option<TrimFrame> = None;
            while let Ok(frame) = playback.frame_rx.try_recv() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                if let Some(tex) = Self::load_trim_texture_rgba(
                    ctx,
                    "trim_scrub",
                    frame.width,
                    frame.height,
                    &frame.rgba,
                ) {
                    self.trim_preview_texture = Some(tex);
                    self.trim_loaded_preview = Some(frame.time_secs);
                    self.trim_preview_error = None;
                }
            }
        }

        ctx.request_repaint();
    }

    fn poll_trim_preview(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.trim_preview_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.trim_preview_rx = None;
                    self.trim_preview_pending = false;
                    match result {
                        Ok((time_secs, png)) => {
                            let Some(state) = &self.trim else {
                                return;
                            };
                            if (state.preview_secs - time_secs).abs() > 0.05 {
                                return;
                            }
                            if let Some(tex) = Self::load_trim_texture(ctx, "trim_scrub", &png) {
                                self.trim_preview_texture = Some(tex);
                                self.trim_loaded_preview = Some(time_secs);
                                self.trim_preview_error = None;
                            }
                        }
                        Err(error) => {
                            self.trim_preview_error = Some(error);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.trim_preview_rx = None;
                    self.trim_preview_pending = false;
                }
            }
        }

        if self.trim.is_some() {
            self.schedule_trim_preview();
        }
    }

    fn trim_transport_button(ui: &mut egui::Ui, playing: bool, enabled: bool) -> bool {
        const SIZE: f32 = 48.0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let accent = theme::accent();
            let fill = if enabled {
                accent
            } else {
                theme::button_disabled()
            };
            painter.circle_filled(rect.center(), SIZE * 0.44, fill);
            let icon_color = egui::Color32::WHITE;
            let c = rect.center();
            if playing {
                let bar_w = 4.0;
                let bar_h = 16.0;
                let gap = 5.0;
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(c.x - gap / 2.0, c.y),
                        egui::vec2(bar_w, bar_h),
                    ),
                    1.0,
                    icon_color,
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(c.x + gap / 2.0, c.y),
                        egui::vec2(bar_w, bar_h),
                    ),
                    1.0,
                    icon_color,
                );
            } else {
                let tri = vec![
                    egui::pos2(c.x - 5.0, c.y - 9.0),
                    egui::pos2(c.x - 5.0, c.y + 9.0),
                    egui::pos2(c.x + 10.0, c.y),
                ];
                painter.add(egui::Shape::convex_polygon(
                    tri,
                    icon_color,
                    egui::Stroke::NONE,
                ));
            }
        }
        enabled && response.clicked()
    }

    fn trim_timeline_ui(
        ui: &mut egui::Ui,
        duration_secs: f64,
        start_secs: &mut f64,
        end_secs: &mut f64,
        preview_secs: &mut f64,
        drag_handle: &mut Option<TrimHandle>,
        filmstrip: Option<&egui::TextureHandle>,
        filmstrip_loading: bool,
        timeline_width: f32,
        timeline_height: f32,
    ) -> bool {
        const HANDLE_HIT: f32 = 10.0;
        const PLAYHEAD_HIT: f32 = 14.0;
        const MIN_GAP: f64 = 0.5;
        let mut interacted = false;

        let width = timeline_width.max(200.0);

        let (response, painter) = ui.allocate_painter(
            egui::vec2(width, timeline_height),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;

        let time_at_x = |x: f32| -> f64 {
            let frac = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            frac as f64 * duration_secs
        };

        let x_at_time = |t: f64| -> f32 {
            let frac = if duration_secs > 0.0 {
                (t / duration_secs).clamp(0.0, 1.0)
            } else {
                0.0
            };
            rect.left() + rect.width() * frac as f32
        };

        if *start_secs >= *end_secs {
            *end_secs = (*start_secs + MIN_GAP).min(duration_secs);
        }
        *start_secs = start_secs.clamp(0.0, duration_secs);
        *end_secs = end_secs.clamp((*start_secs + MIN_GAP).min(duration_secs), duration_secs);

        let track_color = theme::surface_track();
        let dim_color = theme::surface_dim();
        let keep_tint = theme::keep_tint();
        let handle_color = egui::Color32::WHITE;
        let playhead_color = theme::accent_bright();

        painter.rect_filled(rect, 6.0, track_color);

        if let Some(texture) = filmstrip {
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else if filmstrip_loading {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Loading timeline…",
                egui::FontId::proportional(12.0),
                theme::text_muted(),
            );
        }

        let start_x = x_at_time(*start_secs);
        let end_x = x_at_time(*end_secs);
        let playhead_x = x_at_time(*preview_secs);

        let left_dim =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(start_x, rect.bottom()));
        let keep = egui::Rect::from_min_max(
            egui::pos2(start_x, rect.top()),
            egui::pos2(end_x, rect.bottom()),
        );
        let right_dim =
            egui::Rect::from_min_max(egui::pos2(end_x, rect.top()), rect.right_bottom());

        painter.rect_filled(left_dim, 0.0, dim_color);
        painter.rect_filled(keep, 0.0, keep_tint);
        painter.rect_filled(right_dim, 0.0, dim_color);

        let handle_w = 4.0;
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(start_x, rect.center().y),
                egui::vec2(handle_w, rect.height() - 4.0),
            ),
            2.0,
            handle_color,
        );
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(end_x, rect.center().y),
                egui::vec2(handle_w, rect.height() - 4.0),
            ),
            2.0,
            handle_color,
        );

        painter.line_segment(
            [
                egui::pos2(playhead_x, rect.top()),
                egui::pos2(playhead_x, rect.bottom()),
            ],
            egui::Stroke::new(2.0_f32, playhead_color),
        );
        painter.circle_filled(
            egui::pos2(playhead_x, rect.top() + 5.0),
            5.0,
            playhead_color,
        );
        painter.circle_stroke(
            egui::pos2(playhead_x, rect.top() + 5.0),
            5.0,
            egui::Stroke::new(1.5_f32, egui::Color32::WHITE),
        );

        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() {
                if (pos.x - start_x).abs() <= HANDLE_HIT {
                    *drag_handle = Some(TrimHandle::Start);
                } else if (pos.x - end_x).abs() <= HANDLE_HIT {
                    *drag_handle = Some(TrimHandle::End);
                } else if (pos.x - playhead_x).abs() <= PLAYHEAD_HIT {
                    *drag_handle = Some(TrimHandle::Playhead);
                } else {
                    *drag_handle = None;
                }
            }

            if response.dragged() {
                interacted = true;
                let t = time_at_x(pos.x);
                match drag_handle {
                    Some(TrimHandle::Start) => {
                        *start_secs = t.clamp(0.0, *end_secs - MIN_GAP);
                        *preview_secs = *start_secs;
                    }
                    Some(TrimHandle::End) => {
                        *end_secs = t.clamp(*start_secs + MIN_GAP, duration_secs);
                        *preview_secs = *end_secs;
                    }
                    Some(TrimHandle::Playhead) => {
                        *preview_secs = t.clamp(0.0, duration_secs);
                    }
                    None => {}
                }
            } else if response.clicked() {
                let near_start = (pos.x - start_x).abs() <= HANDLE_HIT;
                let near_end = (pos.x - end_x).abs() <= HANDLE_HIT;
                let near_playhead = (pos.x - playhead_x).abs() <= PLAYHEAD_HIT;
                if !near_start && !near_end && !near_playhead {
                    interacted = true;
                    *preview_secs = time_at_x(pos.x).clamp(0.0, duration_secs);
                }
            }
        }

        if response.drag_stopped() {
            *drag_handle = None;
        }

        ui.ctx().set_cursor_icon(if response.hovered() {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });

        interacted
    }

    fn apply_trim(&mut self) {
        if self.trimming {
            self.toast("Trim already in progress…");
            return;
        }
        if self.saving {
            self.toast("Wait for save to finish before trimming");
            return;
        }
        let Some(state) = self.trim.clone() else {
            return;
        };
        if state.end_secs <= state.start_secs {
            self.toast("Invalid trim range");
            return;
        }
        if state.end_secs - state.start_secs < 0.5 {
            self.toast("Trimmed clip must be at least 0.5s");
            return;
        }

        self.trimming = true;
        self.toast("Trimming clip…");

        let path = state.path.clone();
        let start = state.start_secs;
        let end = state.end_secs;
        let (tx, rx) = mpsc::channel();
        self.trim_rx = Some(rx);

        thread::spawn(move || {
            let result = trim_clip(&path, start, end).map(|()| path);
            let _ = tx.send(result);
        });
    }

    fn poll_trim_result(&mut self) {
        let Some(rx) = &self.trim_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.trim_rx = None;
                self.trimming = false;
                match result {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("clip")
                            .to_string();
                        self.toast(format!("Trimmed {name}"));
                        notify_desktop("Clip trimmed", &name);
                        self.trim = None;
                        self.clear_trim_previews();
                        self.page = Page::Clips;
                        self.clips_dirty = true;
                        self.textures.clear();
                        self.clip_meta.clear();
                    }
                    Err(error) => self.toast(error),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.trim_rx = None;
                self.trimming = false;
                self.toast("Trim failed: worker disconnected");
            }
        }
    }

    fn apply_hotkey(&mut self) {
        self.hotkeys
            .rebind(&self.config.hotkey, self.config.portal_hotkey_enabled);
        self.toast(self.hotkeys.status.clone());
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

        // Crash / unexpected exit notices.
        let crash_notice = self.recorder.lock().unwrap().take_crash_notice();
        if let Some(notice) = crash_notice {
            self.toast(notice);
        }

        self.poll_save_result();
        self.poll_trim_result();
        self.poll_trim_filmstrip(ctx);
        self.poll_trim_playback(ctx);
        self.poll_trim_preview(ctx);

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

        // Hotkeys: X11 global / evdev global / focused-window egui fallback.
        if self.hotkeys.poll_global_pressed() || self.hotkeys.matches_egui(ctx) {
            self.save_clip_action();
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

        if self.page == Page::Trim && self.trim.is_none() {
            self.page = Page::Clips;
        }

        if self.page == Page::Trim {
            self.ui_trim_page(ctx);
            egui::TopBottomPanel::bottom("status_bar_trim").show(ctx, |ui| {
                if let Some(toast) = &self.status {
                    ui.label(&toast.message);
                } else if self.trimming {
                    ui.label("Trimming…");
                } else {
                    ui.label("Drag handles or click timeline · Escape to go back");
                }
            });
            return;
        }

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if let Some(toast) = &self.status {
                ui.label(&toast.message);
            } else if let Some(error) = &self.detection.error {
                ui.colored_label(theme::error(), error);
            } else {
                ui.label(format!(
                    "{} · {} · {}s",
                    self.hotkeys.status, self.config.display, self.config.buffer_seconds
                ));
            }
        });

        egui::SidePanel::left("sidebar")
            .default_width(200.0)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_gray(20))
                    .inner_margin(egui::Margin::symmetric(12, 16)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("ReplayForge")
                        .size(20.0)
                        .strong()
                        .color(theme::accent_bright()),
                );
                ui.label(
                    egui::RichText::new("Instant replay")
                        .size(12.0)
                        .color(theme::text_muted()),
                );
                ui.add_space(16.0);

                if theme::nav_item(ui, "Home", self.page == Page::Home) {
                    self.page = Page::Home;
                }
                if theme::nav_item(ui, "Clips", self.page == Page::Clips) {
                    self.page = Page::Clips;
                }
                if theme::nav_item(ui, "Settings", self.page == Page::Settings) {
                    self.page = Page::Settings;
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Home => self.ui_home(ui),
            Page::Clips => self.ui_clips(ui),
            Page::Settings => self.ui_settings(ui),
            Page::Trim => {}
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
                ui.heading("3. Audio");
                ui.checkbox(
                    &mut self.config.capture_system_audio,
                    "Capture system / game audio",
                );
                ui.checkbox(&mut self.config.capture_microphone, "Capture microphone");
                ui.label("You can switch to per-app audio later in Settings.");

                ui.add_space(12.0);
                ui.heading("4. In-game hotkey (Wayland)");
                ui.label(
                    "For F8 while a game is focused, enable the desktop portal hotkey \
                     (no sudo). You can also do this later in Settings.",
                );
                if ui.button("Enable global hotkey (portal)…").clicked() {
                    match self.hotkeys.enable_portal(&self.config.hotkey) {
                        Ok(trigger) => {
                            self.config.portal_hotkey_enabled = true;
                            self.toast(format!("Portal hotkey enabled ({trigger})"));
                        }
                        Err(error) => self.toast(format!("Portal hotkey failed: {error}")),
                    }
                }

                ui.add_space(12.0);
                if let Some(error) = &self.detection.error {
                    ui.colored_label(theme::error(), error);
                } else {
                    let backend = match self.detection.backend {
                        Some(crate::detect::ResolvedBackend::Host) => "host gpu-screen-recorder",
                        Some(crate::detect::ResolvedBackend::Flatpak) => {
                            "Flatpak gpu-screen-recorder"
                        }
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
        ui.add_space(8.0);

        let replay_running = self.recorder.lock().unwrap().is_running();

        theme::section_frame().show(ui, |ui| {
            ui.set_max_width(420.0);

            ui.horizontal(|ui| {
                let (status_color, status_text) = if self.saving {
                    (theme::accent(), "Saving clip…")
                } else if replay_running {
                    (theme::status_running(), "Replay running")
                } else {
                    (theme::text_muted(), "Replay stopped")
                };
                let (dot_rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 16.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot_rect.center(), 5.0, status_color);
                ui.label(egui::RichText::new(status_text).size(16.0).strong());
            });

            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!(
                    "Display: {} · {} FPS · {}s buffer · {}",
                    self.config.display,
                    self.config.fps,
                    self.config.buffer_seconds,
                    self.config.codec
                ))
                .color(theme::text_muted())
                .size(13.0),
            );

            if let Some(error) = self.recorder.lock().unwrap().last_error() {
                ui.add_space(8.0);
                ui.colored_label(theme::error(), error.to_string());
            }

            ui.add_space(16.0);

            let button_text = if replay_running {
                "Stop Replay"
            } else {
                "Start Replay"
            };

            if ui
                .add_sized([240.0, 42.0], theme::primary_button(button_text))
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
                let save_label = if self.saving {
                    "Saving…"
                } else {
                    "Save Clip"
                };
                if ui
                    .add_enabled(
                        !self.saving,
                        theme::secondary_button(save_label).min_size(egui::vec2(240.0, 42.0)),
                    )
                    .clicked()
                {
                    self.save_clip_action();
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Or press {} (global or while focused)",
                        self.config.hotkey
                    ))
                    .color(theme::text_muted())
                    .size(12.0),
                );
            }
        });
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
        ui.horizontal(|ui| {
            ui.label("Sort");
            egui::ComboBox::from_id_salt("clip_sort")
                .selected_text(match self.clip_sort {
                    ClipSort::Newest => "Newest",
                    ClipSort::Name => "Name",
                    ClipSort::Largest => "Largest",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.clip_sort, ClipSort::Newest, "Newest");
                    ui.selectable_value(&mut self.clip_sort, ClipSort::Name, "Name");
                    ui.selectable_value(&mut self.clip_sort, ClipSort::Largest, "Largest");
                });
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.clip_filter)
                    .desired_width(180.0)
                    .hint_text("Search filename…"),
            );
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

                let filter = self.clip_filter.trim().to_ascii_lowercase();
                if !filter.is_empty() {
                    clips.retain(|path| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.to_ascii_lowercase().contains(&filter))
                            .unwrap_or(false)
                    });
                }

                match self.clip_sort {
                    ClipSort::Name => {
                        clips.sort();
                        clips.reverse();
                    }
                    ClipSort::Newest => {
                        clips.sort_by_key(|path| {
                            fs::metadata(path)
                                .and_then(|m| m.modified())
                                .unwrap_or(SystemTime::UNIX_EPOCH)
                        });
                        clips.reverse();
                    }
                    ClipSort::Largest => {
                        clips.sort_by_key(|path| fs::metadata(path).map(|m| m.len()).unwrap_or(0));
                        clips.reverse();
                    }
                }

                if clips.is_empty() {
                    if filter.is_empty() {
                        ui.label("No clips yet. Start replay and hit Save Clip.");
                    } else {
                        ui.label("No clips match that filter.");
                    }
                    return;
                }

                let mut open_path_req: Option<PathBuf> = None;
                let mut copy_path_req: Option<PathBuf> = None;
                let mut start_trim_req: Option<PathBuf> = None;
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

                                let (duration_label, size_label) =
                                    if let Some(cached) = self.clip_meta.get(clip_path) {
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
                                            ui.label(format!("{duration_label} · {size_label}"));
                                            ui.horizontal(|ui| {
                                                if ui.button("Open").clicked() {
                                                    open_path_req = Some(clip_path.clone());
                                                }
                                                if ui
                                                    .button("Copy path")
                                                    .on_hover_text("Copy full path to clipboard")
                                                    .clicked()
                                                {
                                                    copy_path_req = Some(clip_path.clone());
                                                }
                                                if ui.button("Rename").clicked() {
                                                    start_rename = Some(clip_path.clone());
                                                }
                                                if ui.button("Trim").clicked() {
                                                    start_trim_req = Some(clip_path.clone());
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

                if let Some(path) = copy_path_req {
                    ui.ctx().copy_text(path.display().to_string());
                    self.toast("Path copied");
                }

                if let Some(path) = start_trim_req {
                    self.open_trim(path);
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
                ui.label("Quality");
                egui::ComboBox::from_id_salt("settings_quality")
                    .selected_text(format!(
                        "{} ({} kbps)",
                        self.config.quality.label(),
                        self.config.quality.bitrate_kbps()
                    ))
                    .show_ui(ui, |ui| {
                        for preset in quality_choices() {
                            if ui
                                .selectable_value(
                                    &mut self.config.quality,
                                    *preset,
                                    format!("{} ({} kbps)", preset.label(), preset.bitrate_kbps()),
                                )
                                .changed()
                            {
                                self.settings_dirty = true;
                            }
                        }
                    });
            });
            ui.label("Quality uses GSR constant bitrate (recommended for replay buffer).");

            if ui
                .checkbox(
                    &mut self.config.capture_system_audio,
                    "Capture system audio",
                )
                .on_hover_text(
                    "Records desktop/game audio via GPU Screen Recorder (all output or selected apps)",
                )
                .changed()
            {
                self.apply_capture_settings();
            }
            if ui
                .checkbox(&mut self.config.capture_microphone, "Capture microphone")
                .on_hover_text(
                    "Records the default mic via GPU Screen Recorder (default_input). \
                     Merged with system audio into one track when both are enabled.",
                )
                .changed()
            {
                self.apply_capture_settings();
            }

            ui.add_enabled_ui(self.config.capture_system_audio, |ui| {
                ui.horizontal(|ui| {
                    ui.label("System audio source");
                    if ui
                        .selectable_value(
                            &mut self.config.system_audio_mode,
                            SystemAudioMode::All,
                            "All system audio",
                        )
                        .changed()
                    {
                        self.apply_capture_settings();
                    }
                    if ui
                        .selectable_value(
                            &mut self.config.system_audio_mode,
                            SystemAudioMode::Apps,
                            "Selected apps",
                        )
                        .changed()
                    {
                        self.apply_capture_settings();
                    }
                });

                if self.config.system_audio_mode == SystemAudioMode::Apps {
                    ui.horizontal(|ui| {
                        ui.label("Applications");
                        if ui.button("Refresh apps").clicked() {
                            self.detection = Detection::refresh(self.config.backend);
                            if let Some(error) = &self.detection.audio_apps_error {
                                self.toast(error.clone());
                            } else {
                                self.toast(format!(
                                    "Found {} app(s) with audio",
                                    self.detection.audio_apps.len()
                                ));
                            }
                        }
                    });

                    if let Some(error) = &self.detection.audio_apps_error {
                        ui.label(format!("Could not list apps: {error}"));
                    } else if self.detection.audio_apps.is_empty() {
                        ui.label(
                            "No apps listed yet. Play audio in the game (and Discord), then Refresh. \
                             Names are PipeWire clients — e.g. Discord often shows as “webrtc voiceengine”.",
                        );
                    }

                    // Include selected apps that are not currently listed (still valid for GSR).
                    let mut listed: Vec<String> = self.detection.audio_apps.clone();
                    for selected in &self.config.audio_apps {
                        if !listed.iter().any(|a| a == selected) {
                            listed.push(selected.clone());
                        }
                    }
                    listed.sort();

                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for app_name in listed {
                                let mut checked =
                                    self.config.audio_apps.iter().any(|a| a == &app_name);
                                let label = friendly_audio_app_label(&app_name);
                                if ui.checkbox(&mut checked, label).changed() {
                                    if checked {
                                        if !self.config.audio_apps.iter().any(|a| a == &app_name) {
                                            self.config.audio_apps.push(app_name);
                                        }
                                    } else {
                                        self.config.audio_apps.retain(|a| a != &app_name);
                                    }
                                    self.apply_capture_settings();
                                }
                            }
                        });

                    if self.config.audio_apps.is_empty() {
                        ui.label(
                            "No apps selected — using all system audio until you pick apps.",
                        );
                    } else {
                        ui.label(format!(
                            "Capturing {} selected app(s) (+ mic if enabled).",
                            self.config.audio_apps.len()
                        ));
                    }
                }
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
                                .selectable_value(&mut self.config.hotkey, (*key).to_string(), *key)
                                .changed()
                            {
                                self.apply_hotkey();
                                self.persist_config();
                            }
                        }
                    });
            });
            ui.label(&self.hotkeys.status);
            ui.horizontal(|ui| {
                if ui
                    .button("Enable global hotkey (portal)")
                    .on_hover_text(
                        "Uses the desktop permission dialog (xdg-desktop-portal). No sudo / input group.",
                    )
                    .clicked()
                {
                    match self.hotkeys.enable_portal(&self.config.hotkey) {
                        Ok(trigger) => {
                            self.config.portal_hotkey_enabled = true;
                            self.persist_config();
                            self.toast(format!("Portal global hotkey enabled ({trigger})"));
                        }
                        Err(error) => self.toast(format!("Portal hotkey failed: {error}")),
                    }
                }
                if ui
                    .add_enabled(
                        self.hotkeys.is_portal_active(),
                        egui::Button::new("Configure global hotkey…"),
                    )
                    .on_hover_text("Re-open the portal UI to change the bound shortcut")
                    .clicked()
                {
                    match self.hotkeys.configure_portal() {
                        Ok(()) => self.toast("Portal hotkey updated"),
                        Err(error) => self.toast(format!("Configure failed: {error}")),
                    }
                }
            });
            ui.label(
                "Focused hotkey always works. For in-game keys on Wayland, use Enable global hotkey (portal). \
                 Advanced: input group / evdev — see status above.",
            );

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
                    &mut self.config.auto_start_replay,
                    "Start replay buffer when ReplayForge opens",
                )
                .on_hover_text(
                    "Automatically begins recording the rolling buffer after launch \
                     (skipped during first-run setup).",
                )
                .changed()
            {
                self.persist_config();
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
                    .add_sized(
                        [220.0, 36.0],
                        theme::primary_button("Apply & Save"),
                    )
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

    fn ui_trim_page(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.trim.clone() else {
            return;
        };

        let mut apply = false;
        let mut back = false;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.trimming {
            self.cancel_trim();
            return;
        }

        let clip_name = state
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("clip")
            .to_string();
        let total_label = format_duration(state.duration_secs);
        let kept = (state.end_secs - state.start_secs).max(0.0);
        let kept_label = format_duration(kept);
        let start_label = format_duration(state.start_secs);
        let end_label = format_duration(state.end_secs);
        let range_valid =
            kept >= 0.5 && state.start_secs >= 0.0 && state.end_secs <= state.duration_secs + 0.05;
        let preview_label = format_duration(state.preview_secs);
        let playing = self.trim_is_playing();

        egui::TopBottomPanel::top("trim_header")
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.trimming, egui::Button::new("← Back"))
                        .clicked()
                    {
                        back = true;
                    }
                    ui.heading(format!("Trim — {clip_name}"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_apply = range_valid && !self.trimming && !self.saving;
                        if ui
                            .add_enabled(can_apply, theme::primary_button("Apply trim"))
                            .clicked()
                        {
                            apply = true;
                        }
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let preview_width = (ui.available_width() - 32.0).max(320.0);
                let preview_height = preview_width * 9.0 / 16.0;
                let preview_size = egui::vec2(preview_width, preview_height);

                if !self.trim_filmstrip_pending {
                    if self.trim_filmstrip_texture.is_none() {
                        self.schedule_trim_filmstrip(preview_width);
                    } else if (preview_width - self.trim_filmstrip_width).abs() > 48.0 {
                        self.trim_filmstrip_texture = None;
                        self.schedule_trim_filmstrip(preview_width);
                    }
                }

                let preview_frame = egui::Frame::default()
                    .fill(theme::surface())
                    .corner_radius(theme::CORNER_RADIUS);

                preview_frame.show(ui, |ui| {
                    ui.set_width(preview_width);
                    ui.set_height(preview_height);
                    ui.vertical_centered(|ui| {
                        if let Some(texture) = &self.trim_preview_texture {
                            ui.add(egui::Image::new(texture).fit_to_exact_size(preview_size));
                        } else if self.trim_preview_pending {
                            ui.label("Loading preview…");
                        } else if let Some(error) = &self.trim_preview_error {
                            ui.colored_label(theme::error(), error);
                        } else {
                            ui.label("Preview unavailable");
                        }
                    });
                });

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    ui.set_width(preview_width);
                    if Self::trim_transport_button(ui, playing, !self.trimming) {
                        self.toggle_trim_playback();
                    }
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(&preview_label)
                            .size(18.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!(" / {total_label}"))
                            .size(18.0)
                            .color(theme::text_muted()),
                    );
                });

                ui.add_space(8.0);

                let filmstrip = self.trim_filmstrip_texture.clone();
                let filmstrip_loading = self.trim_filmstrip_pending;
                ui.allocate_ui_with_layout(
                    egui::vec2(preview_width, 64.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let interacted = Self::trim_timeline_ui(
                            ui,
                            state.duration_secs,
                            &mut state.start_secs,
                            &mut state.end_secs,
                            &mut state.preview_secs,
                            &mut self.trim_drag_handle,
                            filmstrip.as_ref(),
                            filmstrip_loading,
                            preview_width,
                            64.0,
                        );
                        if interacted {
                            self.stop_trim_playback();
                        }
                    },
                );

                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Keeping {kept_label} of {total_label}  ·  Start {start_label}  ·  End {end_label}"
                    ))
                    .color(theme::text_muted_light()),
                );
            });
        });

        self.trim = Some(state);

        if back {
            self.cancel_trim();
        } else if apply {
            self.apply_trim();
        }
    }
}
