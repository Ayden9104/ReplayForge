//! ReplayForge egui application shell (home, clips, settings, hotkeys).
use crate::audio_volume::apply_config_volumes;
use crate::clips::{
    extract_filmstrip_jpeg, extract_frame_png, extract_waveform_peaks, filmstrip_frame_count,
    trim_clip, waveform_peak_count, TrimSaveMode,
};
use crate::config::{
    AppTheme, Backend, Config, SystemAudioMode, codec_choices, hotkey_choices, path_display,
    quality_choices, resolution_choices, set_autostart,
};
use crate::detect::{
    Detection, clip_duration_secs, format_bytes, format_duration, friendly_audio_app_label,
    probe_clip_meta,
};
use crate::host::{
    notify_desktop, notify_desktop_with_urgency, open_path, open_url, reveal_in_file_manager,
};
use crate::hotkeys::HotkeyService;
use crate::recorder::Recorder;
use crate::sfx;
use crate::share;
use crate::share_links::ShareLinkStore;
use crate::theme;
use crate::tray::{TrayCommand, TrayHandle};
use crate::trim_playback::TrimPlayback;
use crate::update::{self, UpdateInfo};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const CLIP_LOAD_MAX_INFLIGHT: usize = 3;
/// Auto tray recreate delays after first failure (SNI may start late).
const TRAY_RETRY_DELAYS_SECS: &[u64] = &[2, 8];

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
    audio_gain: f32,
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
    sharing: bool,
    share_rx: Option<Receiver<Result<String, String>>>,
    /// Clip path for the in-flight share upload (for persisting the URL).
    pending_share_path: Option<PathBuf>,
    share_links: ShareLinkStore,
    /// Brief green "Copied" flash on Clips Copy link (path + until).
    copy_flash: Option<(PathBuf, Instant)>,
    checking_update: bool,
    update_rx: Option<Receiver<Result<UpdateInfo, String>>>,
    pending_update: Option<UpdateInfo>,
    installing_update: bool,
    update_install_rx: Option<Receiver<(Result<String, String>, String)>>,
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
    trim_waveform: Option<Vec<f32>>,
    trim_waveform_rx: Option<Receiver<Result<Vec<f32>, String>>>,
    trim_waveform_pending: bool,
    trim_muted: bool,
    trim_audio_error: Option<String>,
    clip_meta_tx: Sender<(PathBuf, (String, String))>,
    clip_meta_rx: Receiver<(PathBuf, (String, String))>,
    clip_meta_inflight: HashSet<PathBuf>,
    clip_thumb_tx: Sender<(PathBuf, Result<(u32, u32, Vec<u8>), String>)>,
    clip_thumb_rx: Receiver<(PathBuf, Result<(u32, u32, Vec<u8>), String>)>,
    clip_thumb_inflight: HashSet<PathBuf>,
    /// Newest saved clip to highlight / scroll to in the library.
    clip_focus: Option<PathBuf>,
    /// Scroll to `clip_focus` once on the next Clips render.
    clip_focus_scroll_pending: bool,
    /// Set when StatusNotifier tray creation fails (Bazzite/session without SNI).
    tray_unavailable_reason: Option<String>,
    /// When the first tray create failed (for scheduled retries).
    tray_fail_at: Option<Instant>,
    /// Index into auto-retry delay schedule (`0` = first retry at +2s, `1` = second at +8s).
    tray_retry_index: u8,
    /// When set, retry tray creation at this instant.
    tray_retry_at: Option<Instant>,
    clip_sort: ClipSort,
    clip_filter: String,
    /// Clip + thumbnail paths awaiting delete confirmation.
    pending_delete: Option<(PathBuf, PathBuf)>,
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

        let (tray, tray_unavailable_reason, tray_fail_at, tray_retry_index, tray_retry_at) =
            match crate::tray::create_tray() {
                Ok(tray) => (Some(tray), None, None, 0, None),
                Err(error) => {
                    eprintln!("Tray unavailable: {error}");
                    let fail_at = Instant::now();
                    (
                        None,
                        Some(error),
                        Some(fail_at),
                        0,
                        Some(fail_at + Duration::from_secs(TRAY_RETRY_DELAYS_SECS[0])),
                    )
                }
            };

        if let Err(error) = config.ensure_output_dir() {
            eprintln!("{error}");
        }

        // Sync autostart file with config on launch.
        let _ = set_autostart(config.autostart);

        let (clip_meta_tx, clip_meta_rx) = mpsc::channel();
        let (clip_thumb_tx, clip_thumb_rx) = mpsc::channel();

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
            sharing: false,
            share_rx: None,
            pending_share_path: None,
            share_links: ShareLinkStore::load(),
            copy_flash: None,
            checking_update: false,
            update_rx: None,
            pending_update: None,
            installing_update: false,
            update_install_rx: None,
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
            trim_waveform: None,
            trim_waveform_rx: None,
            trim_waveform_pending: false,
            trim_muted: false,
            trim_audio_error: None,
            clip_meta_tx,
            clip_meta_rx,
            clip_meta_inflight: HashSet::new(),
            clip_thumb_tx,
            clip_thumb_rx,
            clip_thumb_inflight: HashSet::new(),
            clip_focus: None,
            clip_focus_scroll_pending: false,
            tray_unavailable_reason,
            tray_fail_at,
            tray_retry_index,
            tray_retry_at,
            clip_sort: ClipSort::Newest,
            clip_filter: String::new(),
            pending_delete: None,
        };

        if app.config.auto_start_replay && !app.show_first_run {
            app.start_replay();
        }

        app
    }

    pub fn apply_configured_theme(&self, ctx: &egui::Context) {
        theme::apply_theme(ctx, self.config.theme);
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

    /// Recreate the system tray. `manual` resets the auto-retry schedule.
    fn try_recreate_tray(&mut self, manual: bool) {
        if manual {
            self.tray_fail_at = Some(Instant::now());
            self.tray_retry_index = 0;
            self.tray_retry_at = None;
        }

        match crate::tray::create_tray() {
            Ok(tray) => {
                if manual || self.tray.is_none() {
                    eprintln!(
                        "Tray available{}",
                        if manual {
                            " (manual retry)"
                        } else {
                            " after retry"
                        }
                    );
                }
                self.tray = Some(tray);
                self.tray_unavailable_reason = None;
                self.tray_fail_at = None;
                self.tray_retry_index = 0;
                self.tray_retry_at = None;
                if manual {
                    self.toast("System tray connected");
                }
            }
            Err(error) => {
                eprintln!("Tray recreate failed: {error}");
                self.tray = None;
                self.tray_unavailable_reason = Some(error.clone());
                if manual {
                    self.toast(format!("Tray still unavailable: {error}"));
                    // Schedule auto retries from this manual attempt.
                    let fail_at = Instant::now();
                    self.tray_fail_at = Some(fail_at);
                    self.tray_retry_index = 0;
                    self.tray_retry_at =
                        Some(fail_at + Duration::from_secs(TRAY_RETRY_DELAYS_SECS[0]));
                } else {
                    let fail_at = self.tray_fail_at.unwrap_or_else(Instant::now);
                    self.tray_fail_at = Some(fail_at);
                    let next = self.tray_retry_index as usize + 1;
                    if next < TRAY_RETRY_DELAYS_SECS.len() {
                        self.tray_retry_index = next as u8;
                        self.tray_retry_at =
                            Some(fail_at + Duration::from_secs(TRAY_RETRY_DELAYS_SECS[next]));
                    } else {
                        self.tray_retry_at = None;
                    }
                }
            }
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
        sfx::play_clip_saved(
            self.config.clip_sound_path.as_deref(),
            self.config.sfx_volume,
        );
        notify_desktop_with_urgency("Saving clip…", "Capturing your replay buffer", "low", 2500);

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
                        self.toast(format!("{} — {name}", chill_toast(ChillKind::ClipReady)));
                        notify_desktop_with_urgency(
                            "Clip ready",
                            &format!("{name}\nOpen ReplayForge → Clips to review or trim."),
                            if self.config.clip_ready_notify_critical {
                                "critical"
                            } else {
                                "normal"
                            },
                            5000,
                        );
                        self.clips_dirty = true;
                        self.clear_clip_caches();
                        self.clip_focus = Some(path.clone());
                        self.clip_focus_scroll_pending = true;
                        if self.config.open_trim_after_save {
                            self.open_trim(path);
                        } else {
                            self.page = Page::Clips;
                        }
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

    fn create_share_link_action(&mut self, path: PathBuf) {
        if self.share_links.has_live(&path) {
            return;
        }
        self.start_share_upload(path);
    }

    fn copy_share_link_action(&mut self, ctx: &egui::Context, path: PathBuf) {
        match self.share_links.take_live_or_clear_stale(&path) {
            Some(url) => {
                ctx.copy_text(url);
                self.copy_flash = Some((path, Instant::now() + Duration::from_millis(1500)));
                self.toast("Link copied");
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            None => {
                self.toast("Link expired — create a new one");
            }
        }
    }

    fn new_share_link_action(&mut self, path: PathBuf) {
        self.start_share_upload(path);
    }

    fn start_share_upload(&mut self, path: PathBuf) {
        if self.sharing {
            self.toast("Share already in progress…");
            return;
        }
        if !path.is_file() {
            self.toast("Clip file not found");
            return;
        }
        let api_base = self.config.share_api_base.trim().to_string();
        if api_base.is_empty() {
            self.toast("Share is disabled — enable ReplayForge cloud in Settings → Sharing");
            self.page = Page::Settings;
            return;
        }

        self.sharing = true;
        self.pending_share_path = Some(path.clone());
        self.toast("Uploading share link…");
        let (tx, rx) = mpsc::channel();
        self.share_rx = Some(rx);

        thread::spawn(move || {
            let result = share::upload_share_link(&path, &api_base);
            let _ = tx.send(result);
        });
    }

    fn poll_share_result(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.share_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.share_rx = None;
                self.sharing = false;
                let pending_path = self.pending_share_path.take();
                match result {
                    Ok(url) => {
                        if let Some(path) = pending_path {
                            self.share_links.put(&path, url.clone());
                        }
                        let note = share::share_link_note(&url);
                        ctx.copy_text(url.clone());
                        self.toast(format!("{} — {note}", chill_toast(ChillKind::Share)));
                        notify_desktop("Share link ready", &format!("{note}\n{url}"));
                    }
                    Err(error) => self.toast(error),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.share_rx = None;
                self.sharing = false;
                self.pending_share_path = None;
                self.toast("Share failed: worker disconnected");
            }
        }
    }

    fn check_for_updates_action(&mut self) {
        if self.checking_update {
            self.toast("Update check already in progress…");
            return;
        }

        self.checking_update = true;
        self.toast("Checking for updates…");
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);

        thread::spawn(move || {
            let result = update::check_latest();
            let _ = tx.send(result);
        });
    }

    fn poll_update_result(&mut self) {
        let Some(rx) = &self.update_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.update_rx = None;
                self.checking_update = false;
                match result {
                    Ok(info) => {
                        let current = update::current_version();
                        if info.newer {
                            self.toast(format!(
                                "Update available: v{} (you have v{current})",
                                info.latest
                            ));
                            self.pending_update = Some(info);
                        } else {
                            self.pending_update = None;
                            self.toast(format!("You're on the latest version (v{current})"));
                        }
                    }
                    Err(error) => self.toast(error),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_rx = None;
                self.checking_update = false;
                self.toast("Update check failed: worker disconnected");
            }
        }
    }

    fn install_update_action(&mut self) {
        if self.installing_update {
            self.toast("Update install already in progress…");
            return;
        }
        let Some(info) = self.pending_update.clone() else {
            self.toast("No pending update — check for updates first");
            return;
        };

        self.installing_update = true;
        self.toast(format!("Downloading and installing v{}…", info.latest));
        let (tx, rx) = mpsc::channel();
        self.update_install_rx = Some(rx);
        let latest = info.latest.clone();
        let html_url = info.html_url.clone();

        thread::spawn(move || {
            let result = update::install_update(&info).map(|_| latest);
            let _ = tx.send((result, html_url));
        });
    }

    fn poll_update_install_result(&mut self) {
        let Some(rx) = &self.update_install_rx else {
            return;
        };

        match rx.try_recv() {
            Ok((result, html_url)) => {
                self.update_install_rx = None;
                self.installing_update = false;
                match result {
                    Ok(latest) => {
                        self.pending_update = None;
                        match update::relaunch_installed() {
                            Ok(()) => {
                                self.toast(format!("Installed v{latest} — restarting…"));
                            }
                            Err(error) => {
                                self.toast(format!(
                                    "Installed v{latest}, but relaunch failed ({error}). Open ReplayForge from the app menu."
                                ));
                            }
                        }
                        self.quit_requested = true;
                    }
                    Err(error) => {
                        self.toast(error);
                        open_url(&html_url);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_install_rx = None;
                self.installing_update = false;
                self.toast("Update install failed: worker disconnected");
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
            path: path.clone(),
            duration_secs: duration,
            start_secs: 0.0,
            end_secs: duration,
            preview_secs: 0.0,
            audio_gain: 1.0,
        });
        self.trim_preview_last_request = Instant::now() - Duration::from_millis(200);
        self.trim_preview_error = None;
        self.trim_audio_error = None;
        self.page = Page::Trim;
        self.trim_filmstrip_width = 0.0;
        self.trim_filmstrip_target_width = 0.0;
        self.schedule_trim_waveform(duration);
    }

    fn schedule_trim_waveform(&mut self, duration: f64) {
        let Some(state) = &self.trim else {
            return;
        };
        let path = state.path.clone();
        // Use a mid-size peak count; redraw scales visually even if peak count differs.
        let peak_count = waveform_peak_count(800.0);
        let (tx, rx) = mpsc::channel();
        self.trim_waveform_rx = Some(rx);
        self.trim_waveform_pending = true;
        self.trim_waveform = None;

        thread::spawn(move || {
            let result = extract_waveform_peaks(&path, duration, peak_count);
            let _ = tx.send(result);
        });
    }

    fn poll_trim_waveform(&mut self) {
        if let Some(rx) = &self.trim_waveform_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.trim_waveform_rx = None;
                    self.trim_waveform_pending = false;
                    match result {
                        Ok(peaks) => {
                            self.trim_waveform = Some(peaks);
                        }
                        Err(error) => {
                            eprintln!("Waveform: {error}");
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.trim_waveform_rx = None;
                    self.trim_waveform_pending = false;
                }
            }
        }
    }

    fn trim_effective_volume(&self) -> f32 {
        if self.trim_muted { 0.0 } else { 1.0 }
    }

    fn apply_trim_volume(&self) {
        if let Some(playback) = &self.trim_playback {
            playback.set_volume(self.trim_effective_volume());
        }
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

        // Play from playhead, clamped into the keep range.
        let mut play_from = state.preview_secs.clamp(state.start_secs, state.end_secs);
        if play_from >= state.end_secs - 0.05 {
            play_from = state.start_secs;
        }
        if state.end_secs - play_from < 0.05 {
            self.toast("Nothing left to play in selection");
            return;
        }

        match TrimPlayback::start(&state.path, play_from, state.end_secs, state.audio_gain) {
            Ok(playback) => {
                if !playback.audio_enabled {
                    let reason = playback
                        .audio_error
                        .clone()
                        .unwrap_or_else(|| "unknown error".into());
                    self.trim_audio_error = Some(reason.clone());
                    self.toast(format!("Audio unavailable — playing video only ({reason})"));
                } else {
                    self.trim_audio_error = None;
                }
                playback.set_volume(self.trim_effective_volume());
                if let Some(trim) = &mut self.trim {
                    trim.preview_secs = play_from;
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
        self.trim_waveform = None;
        self.trim_waveform_rx = None;
        self.trim_waveform_pending = false;
        self.trim_audio_error = None;
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

        let (should_stop, end_secs) = {
            let Some(playback) = &self.trim_playback else {
                return;
            };
            (
                elapsed >= playback.selection_secs,
                playback.start_secs + playback.selection_secs,
            )
        };

        if should_stop {
            if let Some(state) = &mut self.trim {
                state.preview_secs = end_secs;
            }
            self.trim_loaded_preview = Some(end_secs);
            self.stop_trim_playback();
            return;
        }

        if let Some(playback) = &mut self.trim_playback {
            let target =
                (playback.start_secs + elapsed).min(playback.start_secs + playback.selection_secs);
            if let Some(state) = &mut self.trim {
                state.preview_secs = target;
            }

            if let Some(frame) = playback.take_frame_for_time(target) {
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
        waveform: Option<&[f32]>,
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

        if let Some(peaks) = waveform {
            if !peaks.is_empty() {
                let wave_color = egui::Color32::from_rgba_unmultiplied(100, 200, 255, 110);
                let mid_y = rect.center().y;
                let max_amp = (rect.height() * 0.42).max(4.0);
                let n = peaks.len();
                for (i, &peak) in peaks.iter().enumerate() {
                    let x0 = rect.left() + rect.width() * (i as f32 / n as f32);
                    let x1 = rect.left() + rect.width() * ((i + 1) as f32 / n as f32);
                    let bar_w = (x1 - x0).max(1.0);
                    let amp = peak.clamp(0.0, 1.0) * max_amp;
                    let bar = egui::Rect::from_center_size(
                        egui::pos2(x0 + bar_w * 0.5, mid_y),
                        egui::vec2(bar_w * 0.85, amp * 2.0),
                    );
                    painter.rect_filled(bar, 1.0, wave_color);
                }
            }
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

    fn start_trim_job(&mut self, mode: TrimSaveMode) {
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
        self.toast(match mode {
            TrimSaveMode::ReplaceOriginal => "Trimming clip…",
            TrimSaveMode::SaveCopy => "Saving trim copy…",
        });

        let path = state.path.clone();
        let start = state.start_secs;
        let end = state.end_secs;
        let audio_gain = state.audio_gain;
        let (tx, rx) = mpsc::channel();
        self.trim_rx = Some(rx);

        thread::spawn(move || {
            let result = trim_clip(&path, start, end, audio_gain, mode);
            let _ = tx.send(result);
        });
    }

    fn apply_trim(&mut self) {
        self.start_trim_job(TrimSaveMode::ReplaceOriginal);
    }

    fn save_trim_copy(&mut self) {
        self.start_trim_job(TrimSaveMode::SaveCopy);
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
                        let saved_copy = self
                            .trim
                            .as_ref()
                            .is_some_and(|state| state.path != path);
                        if saved_copy {
                            self.toast(format!("Saved copy — {name}"));
                            notify_desktop("Trim copy saved", &name);
                            self.clip_focus = Some(path.clone());
                            self.clip_focus_scroll_pending = true;
                        } else {
                            self.toast(format!("{} — {name}", chill_toast(ChillKind::Trim)));
                            notify_desktop("Clip trimmed", &name);
                        }
                        self.trim = None;
                        self.clear_trim_previews();
                        self.page = Page::Clips;
                        self.clips_dirty = true;
                        self.clear_clip_caches();
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
        if self.tray.is_none() {
            if let Some(retry_at) = self.tray_retry_at {
                if Instant::now() >= retry_at {
                    self.tray_retry_at = None;
                    self.try_recreate_tray(false);
                } else {
                    ctx.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
                }
            }
        }

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
        self.poll_share_result(ctx);
        self.poll_update_result();
        self.poll_update_install_result();
        self.poll_trim_result();
        self.poll_trim_filmstrip(ctx);
        self.poll_trim_waveform();
        self.poll_trim_playback(ctx);
        self.poll_trim_preview(ctx);
        self.poll_clip_loads(ctx);

        if self.quit_requested {
            let _ = self.recorder.lock().unwrap().stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Minimize on close (tray when available; otherwise hide window and keep buffer).
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.config.minimize_to_tray && !self.quit_requested {
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
                    let mut hint =
                        String::from("Space play/pause · ←/→ scrub · Drag handles · Escape back");
                    if self.trim_muted {
                        hint.push_str(" · Muted");
                    } else if let Some(err) = &self.trim_audio_error {
                        hint.push_str(&format!(" · Audio off ({err})"));
                    }
                    ui.label(hint);
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
                    .fill(theme::panel_fill())
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

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(4.0);
                    if ui
                        .add(theme::secondary_button("Quit"))
                        .on_hover_text("Stop replay and exit ReplayForge")
                        .clicked()
                    {
                        self.quit_requested = true;
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Home => self.ui_home(ui),
            Page::Clips => self.ui_clips(ui),
            Page::Settings => self.ui_settings(ui),
            Page::Trim => {}
        });

        self.ui_delete_confirm(ctx);
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
        let replay_running = self.recorder.lock().unwrap().is_running();
        let last_error = self
            .recorder
            .lock()
            .unwrap()
            .last_error()
            .map(str::to_string);
        let last_clip = self.resolve_last_clip();

        let mut open_last = false;
        let mut trim_last = false;
        let mut copy_last = false;
        let mut go_settings = false;

        ui.add_space(20.0);
        theme::home_section_frame(replay_running).show(ui, |ui| {
            ui.set_max_width(480.0);
            let card_w = ui.available_width();

            if let Some(error) = &last_error {
                ui.colored_label(
                    theme::error(),
                    egui::RichText::new(error).size(13.0).strong(),
                );
                ui.add_space(10.0);
            }

            // Status + hero
            if self.saving {
                ui.horizontal(|ui| {
                    let (dot_rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 28.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(dot_rect.center(), 6.0, theme::accent_bright());
                    ui.label(
                        egui::RichText::new("Saving clip…")
                            .size(22.0)
                            .strong()
                            .color(theme::accent_bright()),
                    );
                });
            } else if replay_running {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
                ui.horizontal(|ui| {
                    let t = ui.input(|i| i.time);
                    let pulse =
                        0.45 + 0.55 * (0.5 + 0.5 * (t * std::f64::consts::TAU * 1.1).sin()) as f32;
                    let base = theme::status_running();
                    let status_color = egui::Color32::from_rgba_unmultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        (pulse * 255.0) as u8,
                    );
                    let (dot_rect, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(dot_rect.center(), 5.0, status_color);
                    ui.label(
                        egui::RichText::new("Live")
                            .size(13.0)
                            .color(theme::text_muted()),
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{}", self.config.buffer_seconds))
                            .size(44.0)
                            .strong()
                            .color(theme::text_primary()),
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        ui.label(
                            egui::RichText::new("s buffer")
                                .size(14.0)
                                .color(theme::text_muted()),
                        );
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("Press {} to save", self.config.hotkey))
                        .color(theme::text_muted())
                        .size(12.0),
                );
            } else {
                ui.horizontal(|ui| {
                    let (dot_rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 28.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(dot_rect.center(), 6.0, theme::text_muted());
                    ui.label(
                        egui::RichText::new("Ready")
                            .size(22.0)
                            .strong()
                            .color(theme::text_muted()),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Ready when you are")
                        .color(theme::text_muted())
                        .size(14.0),
                );
            }

            if self.tray_unavailable_reason.is_some() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Tray unavailable — use Quit in the sidebar.")
                        .color(theme::text_muted())
                        .size(12.0),
                )
                .on_hover_text(
                    self.tray_unavailable_reason
                        .as_deref()
                        .unwrap_or("Tray unavailable"),
                );
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let summary = if replay_running && !self.saving {
                    format!("{} · {} FPS", self.config.display, self.config.fps)
                } else {
                    format!(
                        "{} · {} FPS · {}s",
                        self.config.display, self.config.fps, self.config.buffer_seconds
                    )
                };
                ui.label(
                    egui::RichText::new(summary)
                        .color(theme::text_muted())
                        .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Settings")
                                    .size(12.0)
                                    .color(theme::accent_bright()),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        go_settings = true;
                    }
                });
            });

            ui.add_space(14.0);

            if replay_running {
                let save_label = if self.saving {
                    "Saving…"
                } else {
                    "Save Clip"
                };
                if ui
                    .add_enabled(
                        !self.saving,
                        theme::primary_button(save_label).min_size(egui::vec2(card_w, 48.0)),
                    )
                    .clicked()
                {
                    self.save_clip_action();
                }
                ui.add_space(10.0);
                if ui
                    .add_sized([card_w, 44.0], theme::secondary_button("Stop Replay"))
                    .clicked()
                {
                    self.stop_replay();
                }
            } else if ui
                .add_sized([card_w, 48.0], theme::primary_button("Start Replay"))
                .clicked()
            {
                self.start_replay();
            }

            if let Some(ref clip_path) = last_clip {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Last clip")
                        .size(11.0)
                        .color(theme::text_muted()),
                );
                ui.add_space(6.0);

                let thumb_path = clip_path.with_extension("png");
                let name = clip_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("clip");
                let display_name = if name.chars().count() > 36 {
                    format!("{}…", name.chars().take(35).collect::<String>())
                } else {
                    name.to_string()
                };
                let has_live_share = self.share_links.has_live(clip_path);

                theme::home_last_clip_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let thumb_size = egui::vec2(80.0, 45.0);
                        if let Some(texture) = self.textures.get(&thumb_path) {
                            let response = ui.add(
                                egui::Image::new(texture)
                                    .fit_to_exact_size(thumb_size)
                                    .corner_radius(6.0)
                                    .bg_fill(theme::surface_dim()),
                            );
                            let r = response.rect;
                            ui.painter().rect_stroke(
                                r,
                                6.0,
                                egui::Stroke::new(1.0_f32, theme::stroke_subtle()),
                                egui::StrokeKind::Outside,
                            );
                            if response.clicked() {
                                open_last = true;
                            }
                        } else {
                            let (rect, response) =
                                ui.allocate_exact_size(thumb_size, egui::Sense::click());
                            ui.painter().rect_filled(rect, 6.0, theme::surface());
                            ui.painter().rect_stroke(
                                rect,
                                6.0,
                                egui::Stroke::new(1.0_f32, theme::stroke_subtle()),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                if thumb_path.exists() { "…" } else { "—" },
                                egui::FontId::proportional(12.0),
                                theme::text_muted(),
                            );
                            if thumb_path.exists() && ui.is_rect_visible(rect) {
                                self.schedule_clip_thumb(thumb_path.clone());
                            }
                            if response.clicked() {
                                open_last = true;
                            }
                        }

                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(display_name).size(15.0).strong());
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui.add(theme::secondary_button("Open")).clicked() {
                                    open_last = true;
                                }
                                if ui.add(theme::secondary_button("Trim")).clicked() {
                                    trim_last = true;
                                }
                                if has_live_share
                                    && ui.add(theme::secondary_button("Copy link")).clicked()
                                {
                                    copy_last = true;
                                }
                            });
                        });
                    });
                });
            }
        });

        if go_settings {
            self.page = Page::Settings;
        }
        if let Some(path) = last_clip {
            if open_last {
                let _ = open_path(&path);
            }
            if trim_last {
                self.open_trim(path.clone());
            }
            if copy_last {
                self.copy_share_link_action(ui.ctx(), path);
            }
        }
    }

    /// Most recent clip for Home: focused save if it still exists, else newest by mtime.
    fn resolve_last_clip(&self) -> Option<PathBuf> {
        if let Some(path) = &self.clip_focus {
            if path.is_file() {
                return Some(path.clone());
            }
        }
        let dir = &self.config.output_dir;
        let Ok(entries) = fs::read_dir(dir) else {
            return None;
        };
        let mut best: Option<(SystemTime, PathBuf)> = None;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
            {
                continue;
            }
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            match &best {
                Some((t, _)) if modified <= *t => {}
                _ => best = Some((modified, path)),
            }
        }
        best.map(|(_, p)| p)
    }

    fn clear_clip_caches(&mut self) {
        self.textures.clear();
        self.clip_meta.clear();
        self.clip_meta_inflight.clear();
        self.clip_thumb_inflight.clear();
    }

    fn clip_load_inflight_count(&self) -> usize {
        self.clip_meta_inflight.len() + self.clip_thumb_inflight.len()
    }

    fn schedule_clip_meta(&mut self, path: PathBuf) {
        if self.clip_meta.contains_key(&path) || self.clip_meta_inflight.contains(&path) {
            return;
        }
        if self.clip_load_inflight_count() >= CLIP_LOAD_MAX_INFLIGHT {
            return;
        }
        self.clip_meta_inflight.insert(path.clone());
        let tx = self.clip_meta_tx.clone();
        thread::spawn(move || {
            let meta = probe_clip_meta(&path);
            let _ = tx.send((path, meta));
        });
    }

    fn schedule_clip_thumb(&mut self, thumb_path: PathBuf) {
        if self.textures.contains_key(&thumb_path) || self.clip_thumb_inflight.contains(&thumb_path)
        {
            return;
        }
        if !thumb_path.exists() {
            return;
        }
        if self.clip_load_inflight_count() >= CLIP_LOAD_MAX_INFLIGHT {
            return;
        }
        self.clip_thumb_inflight.insert(thumb_path.clone());
        let tx = self.clip_thumb_tx.clone();
        thread::spawn(move || {
            let result = image::open(&thumb_path)
                .map(|image| {
                    let rgba = image.to_rgba8();
                    (rgba.width(), rgba.height(), rgba.into_raw())
                })
                .map_err(|e| e.to_string());
            let _ = tx.send((thumb_path, result));
        });
    }

    fn poll_clip_loads(&mut self, ctx: &egui::Context) {
        let mut got_any = false;

        while let Ok((path, meta)) = self.clip_meta_rx.try_recv() {
            self.clip_meta_inflight.remove(&path);
            self.clip_meta.insert(path, meta);
            got_any = true;
        }

        while let Ok((thumb_path, result)) = self.clip_thumb_rx.try_recv() {
            self.clip_thumb_inflight.remove(&thumb_path);
            match result {
                Ok((width, height, rgba)) => {
                    let texture = ctx.load_texture(
                        thumb_path.to_string_lossy(),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &rgba,
                        ),
                        Default::default(),
                    );
                    self.textures.insert(thumb_path, texture);
                }
                Err(error) => {
                    eprintln!("Clip thumbnail decode failed: {error}");
                }
            }
            got_any = true;
        }

        if got_any {
            ctx.request_repaint();
        }
    }

    fn ui_clips(&mut self, ui: &mut egui::Ui) {
        ui.heading("Clips");
        ui.add_space(8.0);

        let clips_folder = self.config.output_dir.clone();
        let _ = self.clips_dirty;
        self.clips_dirty = false;

        let (all_clips, read_error) = match fs::read_dir(&clips_folder) {
            Ok(entries) => {
                let clips: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
                    })
                    .collect();
                (clips, None)
            }
            Err(error) => (Vec::new(), Some(error.to_string())),
        };

        if let Some(error) = read_error {
            ui.colored_label(theme::error(), format!("Cannot read clips folder: {error}"));
            return;
        }

        let library_count = all_clips.len();
        let library_bytes: u64 = all_clips.iter().map(|p| clip_storage_bytes(p)).sum();

        let filter = self.clip_filter.trim().to_ascii_lowercase();
        let mut clips = all_clips;
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

        let visible_count = clips.len();
        let visible_bytes: u64 = clips.iter().map(|p| clip_storage_bytes(p)).sum();
        let filter_active = !filter.is_empty();

        theme::section_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add(theme::secondary_button("Refresh")).clicked() {
                    self.clear_clip_caches();
                    self.clips_dirty = true;
                }
                if ui.add(theme::secondary_button("Open Folder")).clicked() {
                    let _ = self.config.ensure_output_dir();
                    let _ = open_path(&self.config.output_dir);
                }
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Sort").color(theme::text_muted()));
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
                ui.label(egui::RichText::new("Filter").color(theme::text_muted()));
                ui.add(
                    egui::TextEdit::singleline(&mut self.clip_filter)
                        .desired_width(180.0)
                        .hint_text("Search filename…"),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_clips_storage_stats(
                        ui,
                        visible_count,
                        visible_bytes,
                        library_count,
                        library_bytes,
                        filter_active,
                    );
                });
            });
        });

        ui.add_space(12.0);

        if clips.is_empty() {
            theme::section_frame().show(ui, |ui| {
                ui.set_max_width(420.0);
                if filter.is_empty() {
                    ui.label(
                        egui::RichText::new("No clips yet. Start replay and hit Save Clip.")
                            .color(theme::text_muted()),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("No clips match that filter.")
                            .color(theme::text_muted()),
                    );
                }
            });
            return;
        }

        let mut open_path_req: Option<PathBuf> = None;
        let mut copy_path_req: Option<PathBuf> = None;
        let mut reveal_req: Option<PathBuf> = None;
        let mut share_create_req: Option<PathBuf> = None;
        let mut share_copy_req: Option<PathBuf> = None;
        let mut share_new_req: Option<PathBuf> = None;
        let mut start_trim_req: Option<PathBuf> = None;
        let mut start_rename: Option<PathBuf> = None;
        let mut finish_rename: Option<(PathBuf, String)> = None;
        let mut cancel_rename = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let gap = 16.0;
            let min_card_outer = 300.0;
            // Matches theme::card_frame() Margin::same(14) on left+right.
            let frame_pad_x = 28.0;
            let available = ui.available_width();
            let columns = ((available + gap) / (min_card_outer + gap))
                .floor()
                .max(1.0) as usize;
            let card_outer = (available - gap * columns.saturating_sub(1) as f32) / columns as f32;
            let card_inner = (card_outer - frame_pad_x).max(160.0);
            let thumb_w = card_inner;
            let thumb_h = thumb_w * 9.0 / 16.0;

            egui::Grid::new("clips_grid")
                .num_columns(columns)
                .spacing([gap, gap])
                .show(ui, |ui| {
                    for (index, clip_path) in clips.iter().enumerate() {
                        let clip_name = clip_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Unknown clip")
                            .to_string();

                        let thumbnail_path = clip_path.with_extension("png");
                        let meta_cached = self.clip_meta.get(clip_path).cloned();
                        let size_fallback = fs::metadata(clip_path)
                            .map(|m| format_bytes(m.len()))
                            .unwrap_or_else(|_| "?".into());

                        let focused = self.clip_focus.as_ref() == Some(clip_path);
                        let card = if focused {
                            theme::card_frame_focused()
                        } else {
                            theme::card_frame()
                        };

                        let card_response = card.show(ui, |ui| {
                            ui.set_min_width(card_inner);
                            ui.set_max_width(card_inner);
                            ui.vertical(|ui| {
                                let has_texture = self.textures.contains_key(&thumbnail_path);
                                let thumb_exists = thumbnail_path.exists();

                                if has_texture {
                                    if let Some(texture) = self.textures.get(&thumbnail_path) {
                                        let response = ui
                                            .add(
                                                egui::Image::new(texture)
                                                    .fit_to_exact_size(egui::vec2(thumb_w, thumb_h))
                                                    .corner_radius(6.0),
                                            )
                                            .on_hover_text("Open in default app");
                                        if response.clicked() || response.double_clicked() {
                                            open_path_req = Some(clip_path.clone());
                                        }
                                        if ui.is_rect_visible(response.rect)
                                            && meta_cached.is_none()
                                        {
                                            self.schedule_clip_meta(clip_path.clone());
                                        }
                                    }
                                } else {
                                    let (thumb_rect, thumb_response) = ui.allocate_exact_size(
                                        egui::vec2(thumb_w, thumb_h),
                                        egui::Sense::click(),
                                    );
                                    let thumb_response =
                                        thumb_response.on_hover_text("Open in default app");
                                    ui.painter().rect_filled(
                                        thumb_rect,
                                        6.0,
                                        theme::surface_track(),
                                    );
                                    let placeholder = if thumb_exists {
                                        if self.clip_thumb_inflight.contains(&thumbnail_path) {
                                            "Loading…"
                                        } else {
                                            "…"
                                        }
                                    } else {
                                        "No thumbnail"
                                    };
                                    ui.painter().text(
                                        thumb_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        placeholder,
                                        egui::FontId::proportional(13.0),
                                        theme::text_muted(),
                                    );
                                    if thumb_response.clicked() || thumb_response.double_clicked() {
                                        open_path_req = Some(clip_path.clone());
                                    }
                                    if !has_texture && thumb_exists {
                                        ui.label(
                                            egui::RichText::new("Click Open to play")
                                                .color(theme::text_muted())
                                                .size(11.0),
                                        );
                                    }
                                    let visible = ui.is_rect_visible(thumb_rect);
                                    if visible && thumb_exists {
                                        self.schedule_clip_thumb(thumbnail_path.clone());
                                    }
                                    if visible && meta_cached.is_none() {
                                        self.schedule_clip_meta(clip_path.clone());
                                    }
                                }

                                ui.add_space(8.0);

                                let renaming =
                                    self.rename.as_ref().is_some_and(|r| r.path == *clip_path);

                                if renaming {
                                    if let Some(state) = self.rename.as_mut() {
                                        ui.text_edit_singleline(&mut state.text);
                                        ui.horizontal(|ui| {
                                            if ui.add(theme::primary_button("Save")).clicked() {
                                                finish_rename =
                                                    Some((state.path.clone(), state.text.clone()));
                                            }
                                            if ui.add(theme::secondary_button("Cancel")).clicked() {
                                                cancel_rename = true;
                                            }
                                        });
                                    }
                                } else {
                                    if focused {
                                        ui.label(
                                            egui::RichText::new("Just saved")
                                                .color(theme::accent_bright())
                                                .size(11.0)
                                                .strong(),
                                        );
                                    }
                                    ui.label(egui::RichText::new(&clip_name).strong().size(14.0));
                                    let meta_line = if let Some((duration, size)) = &meta_cached {
                                        format!("{duration} · {size}")
                                    } else if self.clip_meta_inflight.contains(clip_path) {
                                        format!("Loading… · {size_fallback}")
                                    } else {
                                        format!("--:-- · {size_fallback}")
                                    };
                                    ui.label(
                                        egui::RichText::new(meta_line)
                                            .color(theme::text_muted())
                                            .size(12.0),
                                    );
                                    ui.add_space(4.0);
                                    let has_live_share = self.share_links.has_live(clip_path);
                                    ui.horizontal(|ui| {
                                        if focused {
                                            if ui.add(theme::primary_button("Trim")).clicked() {
                                                start_trim_req = Some(clip_path.clone());
                                            }
                                        }

                                        if has_live_share {
                                            let copy_flash_active = self
                                                .copy_flash
                                                .as_ref()
                                                .is_some_and(|(p, until)| {
                                                    p == clip_path && Instant::now() < *until
                                                });
                                            if copy_flash_active {
                                                ui.ctx().request_repaint_after(
                                                    Duration::from_millis(50),
                                                );
                                            } else if self
                                                .copy_flash
                                                .as_ref()
                                                .is_some_and(|(p, _)| p == clip_path)
                                            {
                                                self.copy_flash = None;
                                            }
                                            let copy_label = if copy_flash_active {
                                                "Copied"
                                            } else {
                                                "Copy link"
                                            };
                                            let copy_btn = if copy_flash_active {
                                                (if focused {
                                                    theme::secondary_button(copy_label)
                                                } else {
                                                    theme::primary_button(copy_label)
                                                })
                                                .fill(theme::success())
                                            } else if focused {
                                                theme::secondary_button(copy_label)
                                            } else {
                                                theme::primary_button(copy_label)
                                            };
                                            if ui
                                                .add_enabled(!self.sharing, copy_btn)
                                                .on_hover_text(
                                                    "Copy the existing cloud link (no re-upload)",
                                                )
                                                .clicked()
                                            {
                                                share_copy_req = Some(clip_path.clone());
                                            }
                                        } else {
                                            let create_label = if self.sharing {
                                                "Sharing…"
                                            } else {
                                                "Create link"
                                            };
                                            let create_btn = if focused {
                                                theme::secondary_button(create_label)
                                            } else {
                                                theme::primary_button(create_label)
                                            };
                                            if ui
                                                .add_enabled(!self.sharing, create_btn)
                                                .on_hover_text(
                                                    "Upload to ReplayForge cloud and copy link",
                                                )
                                                .clicked()
                                            {
                                                share_create_req = Some(clip_path.clone());
                                            }
                                        }

                                        if ui
                                            .add(theme::secondary_button("Open"))
                                            .on_hover_text("Open in default app")
                                            .clicked()
                                        {
                                            open_path_req = Some(clip_path.clone());
                                        }

                                        ui.menu_button("⋯", |ui| {
                                            if ui.button("Open").clicked() {
                                                open_path_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if !focused && ui.button("Trim").clicked() {
                                                start_trim_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if ui.button("Rename").clicked() {
                                                start_rename = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if ui
                                                .button("Show in folder")
                                                .on_hover_text(
                                                    "Open the clips folder in your file manager",
                                                )
                                                .clicked()
                                            {
                                                reveal_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if ui
                                                .button("Copy path")
                                                .on_hover_text("Copy full path to clipboard")
                                                .clicked()
                                            {
                                                copy_path_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if !has_live_share
                                                && ui
                                                    .add_enabled(
                                                        !self.sharing,
                                                        egui::Button::new(if self.sharing {
                                                            "Sharing…"
                                                        } else {
                                                            "Create link"
                                                        }),
                                                    )
                                                    .on_hover_text(
                                                        "Upload to ReplayForge cloud and copy link",
                                                    )
                                                    .clicked()
                                            {
                                                share_create_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if has_live_share
                                                && ui
                                                    .add_enabled(
                                                        !self.sharing,
                                                        egui::Button::new(if self.sharing {
                                                            "Sharing…"
                                                        } else {
                                                            "New link"
                                                        }),
                                                    )
                                                    .on_hover_text(
                                                        "Upload again and replace the saved link",
                                                    )
                                                    .clicked()
                                            {
                                                share_new_req = Some(clip_path.clone());
                                                ui.close();
                                            }
                                            if ui
                                                .button(
                                                    egui::RichText::new("Delete")
                                                        .color(theme::error()),
                                                )
                                                .clicked()
                                            {
                                                self.pending_delete = Some((
                                                    clip_path.clone(),
                                                    thumbnail_path.clone(),
                                                ));
                                                ui.close();
                                            }
                                        });
                                    });
                                }
                            });
                        });

                        if focused && self.clip_focus_scroll_pending {
                            card_response
                                .response
                                .scroll_to_me(Some(egui::Align::Center));
                            self.clip_focus_scroll_pending = false;
                        }

                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }

                    if !clips.is_empty() && clips.len() % columns != 0 {
                        ui.end_row();
                    }
                });
        });

        if let Some(path) = open_path_req {
            if open_path(&path).is_err() {
                self.toast("Could not open clip");
            }
        }

        if let Some(path) = copy_path_req {
            ui.ctx().copy_text(path.display().to_string());
            self.toast("Path copied");
        }

        if let Some(path) = reveal_req {
            reveal_in_file_manager(&path);
        }

        if let Some(path) = share_create_req {
            self.create_share_link_action(path);
        }
        if let Some(path) = share_copy_req {
            self.copy_share_link_action(ui.ctx(), path);
        }
        if let Some(path) = share_new_req {
            self.new_share_link_action(path);
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
                if !is_safe_clip_stem(new_stem) {
                    self.toast("Invalid name (no path separators or ..)");
                } else {
                    let new_path = old_path.with_file_name(format!("{new_stem}.mp4"));
                    let old_thumb = old_path.with_extension("png");
                    let new_thumb = new_path.with_extension("png");
                    match fs::rename(&old_path, &new_path) {
                        Ok(()) => {
                            if old_thumb.exists() {
                                let _ = fs::rename(&old_thumb, &new_thumb);
                            }
                            self.share_links.rename_path(&old_path, &new_path);
                            if self.clip_focus.as_ref() == Some(&old_path) {
                                self.clip_focus = Some(new_path);
                            }
                            self.clear_clip_caches();
                            self.toast("Clip renamed");
                        }
                        Err(error) => self.toast(format!("Rename failed: {error}")),
                    }
                }
            }
            self.rename = None;
        }
    }

    fn ui_delete_confirm(&mut self, ctx: &egui::Context) {
        let Some((clip_path, thumbnail_path)) = self.pending_delete.clone() else {
            return;
        };

        let name = clip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("clip");

        let mut confirm = false;
        let mut cancel = false;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }

        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("delete_confirm_dim"))
            .fixed_pos(screen.min)
            .order(egui::Order::Middle)
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
                let response = ui.allocate_response(screen.size(), egui::Sense::click());
                if response.clicked() {
                    cancel = true;
                }
            });

        egui::Window::new("Delete clip?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Delete \"{name}\"? This cannot be undone."));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::secondary_button("Cancel")).clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(egui::RichText::new("Delete").color(theme::error()))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });

        if cancel {
            self.pending_delete = None;
        } else if confirm {
            self.pending_delete = None;
            if let Err(error) = fs::remove_file(&clip_path) {
                self.toast(format!("Failed to delete clip: {error}"));
            } else {
                self.share_links.remove(&clip_path);
                if thumbnail_path.exists() {
                    let _ = fs::remove_file(&thumbnail_path);
                }
                if self.clip_focus.as_ref() == Some(&clip_path) {
                    self.clip_focus = None;
                }
                self.clear_clip_caches();
                self.toast(chill_toast(ChillKind::Delete));
            }
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section_frame().show(ui, |ui| {
                ui.heading("Capture");
                ui.add_space(8.0);

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
                    if ui
                        .add(theme::secondary_button("Refresh"))
                        .clicked()
                    {
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
                    ui.label("Resolution");
                    let selected_label = resolution_choices()
                        .iter()
                        .find(|(value, _)| *value == self.config.resolution)
                        .map(|(_, label)| *label)
                        .unwrap_or(self.config.resolution.as_str());
                    egui::ComboBox::from_id_salt("settings_resolution")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (value, label) in resolution_choices() {
                                if ui
                                    .selectable_value(
                                        &mut self.config.resolution,
                                        (*value).to_string(),
                                        *label,
                                    )
                                    .changed()
                                {
                                    self.settings_dirty = true;
                                }
                            }
                        });
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
                    let res = self.config.resolution.as_str();
                    egui::ComboBox::from_id_salt("settings_quality")
                        .selected_text(format!(
                            "{} ({} kbps)",
                            self.config.quality.label(),
                            self.config.quality.bitrate_kbps(res)
                        ))
                        .show_ui(ui, |ui| {
                            for preset in quality_choices() {
                                if ui
                                    .selectable_value(
                                        &mut self.config.quality,
                                        *preset,
                                        format!(
                                            "{} ({} kbps)",
                                            preset.label(),
                                            preset.bitrate_kbps(res)
                                        ),
                                    )
                                    .changed()
                                {
                                    self.settings_dirty = true;
                                }
                            }
                        });
                });
                ui.label(
                    egui::RichText::new(
                        "Quality uses GSR constant bitrate scaled for the selected resolution \
                         (recommended for replay buffer).",
                    )
                    .color(theme::text_muted())
                    .size(12.0),
                );

                ui.add_space(8.0);
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
                ui.label(
                    egui::RichText::new(format!(
                        "Detected: host={}, flatpak={}",
                        self.detection.host_gsr, self.detection.flatpak_gsr
                    ))
                    .color(theme::text_muted())
                    .size(12.0),
                );
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Audio");
                ui.add_space(8.0);

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

                ui.add_enabled_ui(self.config.capture_microphone, |ui| {
                    ui.label("Mic volume");
                    let slider = theme::volume_slider(ui, &mut self.config.mic_volume).on_hover_text(
                        "Adjusts the default mic level in PipeWire before capture. \
                         May also affect other apps using the same mic.",
                    );
                    if slider.drag_stopped() || slider.lost_focus() {
                        let pct = (self.config.mic_volume * 100.0).round() as u32;
                        let errors = apply_config_volumes(&self.config);
                        if errors.is_empty() {
                            self.toast(format!("Mic volume set to {pct}%"));
                        } else {
                            for error in errors {
                                self.toast(error);
                            }
                        }
                        self.persist_config();
                    }
                });

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

                    let desktop_volume_enabled =
                        self.config.system_audio_mode == SystemAudioMode::All;
                    ui.add_enabled_ui(desktop_volume_enabled, |ui| {
                        ui.label("Desktop audio volume");
                        let slider = theme::volume_slider(
                            ui,
                            &mut self.config.desktop_audio_volume,
                        )
                        .on_hover_text(
                            "Adjusts the default output monitor level in PipeWire before capture. \
                             May also affect desktop audio heard by other apps.",
                        );
                        if slider.drag_stopped() || slider.lost_focus() {
                            let pct = (self.config.desktop_audio_volume * 100.0).round() as u32;
                            let errors = apply_config_volumes(&self.config);
                            if errors.is_empty() {
                                self.toast(format!("Desktop audio volume set to {pct}%"));
                            } else {
                                for error in errors {
                                    self.toast(error);
                                }
                            }
                            self.persist_config();
                        }
                    });
                    if !desktop_volume_enabled {
                        ui.label(
                            egui::RichText::new(
                                "Volume control applies to All system audio mode.",
                            )
                            .color(theme::text_muted())
                            .size(12.0),
                        );
                    }

                    if self.config.system_audio_mode == SystemAudioMode::Apps {
                        ui.horizontal(|ui| {
                            ui.label("Applications");
                            if ui
                                .add(theme::secondary_button("Refresh apps"))
                                .clicked()
                            {
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
                            ui.colored_label(theme::error(), format!("Could not list apps: {error}"));
                        } else if self.detection.audio_apps.is_empty() {
                            ui.label(
                                egui::RichText::new(
                                    "No apps listed yet. Play audio in the game (and Discord), then Refresh. \
                                     Names are PipeWire clients — e.g. Discord often shows as “webrtc voiceengine”.",
                                )
                                .color(theme::text_muted())
                                .size(12.0),
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
                                            if !self.config.audio_apps.iter().any(|a| a == &app_name)
                                            {
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
                                egui::RichText::new(
                                    "No apps selected — using all system audio until you pick apps.",
                                )
                                .color(theme::text_muted())
                                .size(12.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Capturing {} selected app(s) (+ mic if enabled).",
                                    self.config.audio_apps.len()
                                ))
                                .color(theme::text_muted())
                                .size(12.0),
                            );
                        }
                    }
                });
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Output");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(path_display(&self.config.output_dir));
                    if ui
                        .add(theme::secondary_button("Browse…"))
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.config.output_dir = path;
                            self.settings_dirty = true;
                        }
                    }
                });
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Hotkey");
                ui.add_space(8.0);
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
                ui.label(
                    egui::RichText::new(&self.hotkeys.status)
                        .color(theme::text_muted())
                        .size(12.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(theme::primary_button("Enable global hotkey (portal)"))
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
                            theme::secondary_button("Configure global hotkey…"),
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
                    egui::RichText::new(
                        "Focused hotkey always works. For in-game keys on Wayland, use Enable global hotkey (portal). \
                         Advanced: input group / evdev — see status above.",
                    )
                    .color(theme::text_muted())
                    .size(12.0),
                );
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Appearance");
                ui.add_space(8.0);
                let mut theme_choice = self.config.theme;
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    theme::paint_swatch(ui, theme::swatch_colors(theme_choice));
                    egui::ComboBox::from_id_salt("app_theme")
                        .selected_text(theme_choice.label())
                        .show_ui(ui, |ui| {
                            for option in [
                                AppTheme::Classic,
                                AppTheme::Arma3,
                                AppTheme::NightOps,
                                AppTheme::Pirate,
                            ] {
                                ui.horizontal(|ui| {
                                    theme::paint_swatch(ui, theme::swatch_colors(option));
                                    if ui
                                        .selectable_label(
                                            theme_choice == option,
                                            option.label(),
                                        )
                                        .clicked()
                                    {
                                        theme_choice = option;
                                    }
                                });
                            }
                        });
                });
                if theme_choice != self.config.theme {
                    self.config.theme = theme_choice;
                    self.persist_config();
                    theme::apply_theme(ui.ctx(), self.config.theme);
                }
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Desktop");
                ui.add_space(8.0);
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
                    .on_hover_text(
                        "Hides the window instead of quitting. Works even if the system tray \
                         icon is unavailable — reopen from the app menu, or use Quit in the sidebar.",
                    )
                    .changed()
                {
                    self.persist_config();
                }

                ui.horizontal(|ui| {
                    let status = if self.tray.is_some() {
                        "Available".to_string()
                    } else {
                        self.tray_unavailable_reason
                            .as_deref()
                            .map(|e| {
                                let short = e.chars().take(80).collect::<String>();
                                if e.len() > 80 {
                                    format!("{short}…")
                                } else {
                                    short
                                }
                            })
                            .unwrap_or_else(|| "Unavailable".into())
                    };
                    ui.label(format!("Tray: {status}"));
                    if self.tray.is_none()
                        && ui
                            .add(theme::secondary_button("Retry tray"))
                            .on_hover_text("Try creating the system tray icon again")
                            .clicked()
                    {
                        self.try_recreate_tray(true);
                    }
                });

                if ui
                    .checkbox(
                        &mut self.config.open_trim_after_save,
                        "Open trim editor after saving a clip",
                    )
                    .on_hover_text(
                        "When enabled, save jumps straight into Trim for the new clip \
                         instead of highlighting it in Clips.",
                    )
                    .changed()
                {
                    self.persist_config();
                }
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Sound & notifications");
                ui.add_space(8.0);

                ui.label("Clip save sound");
                ui.horizontal(|ui| {
                    let label = self
                        .config
                        .clip_sound_path
                        .as_ref()
                        .map(|p| path_display(p))
                        .unwrap_or_else(|| "Bundled default".to_string());
                    ui.label(label);
                    if ui.add(theme::secondary_button("Browse…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "ogg", "flac", "mp3"])
                            .pick_file()
                        {
                            self.config.clip_sound_path = Some(path);
                            self.persist_config();
                        }
                    }
                    if ui
                        .add_enabled(
                            self.config.clip_sound_path.is_some(),
                            theme::secondary_button("Clear"),
                        )
                        .clicked()
                    {
                        self.config.clip_sound_path = None;
                        self.persist_config();
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Volume");
                    let mut volume = self.config.sfx_volume;
                    let response = ui.add(
                        egui::DragValue::new(&mut volume)
                            .speed(0.05)
                            .range(0.0..=2.0)
                            .suffix("×"),
                    );
                    if response.changed() {
                        self.config.sfx_volume = volume;
                        self.persist_config();
                    }
                });

                if ui
                    .checkbox(
                        &mut self.config.clip_ready_notify_critical,
                        "Critical urgency for Clip ready notification",
                    )
                    .on_hover_text(
                        "Critical helps the notification show over fullscreen games on some \
                         desktops (e.g. KDE). Turn off for quieter normal urgency.",
                    )
                    .changed()
                {
                    self.persist_config();
                }

                if ui
                    .add(theme::secondary_button("Test sound"))
                    .clicked()
                {
                    sfx::play_clip_saved(
                        self.config.clip_sound_path.as_deref(),
                        self.config.sfx_volume,
                    );
                }
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("Sharing");
                ui.add_space(8.0);

                let default_base = crate::config::default_share_api_base();
                let current = self.config.share_api_base.trim();
                let using_cloud = current == default_base.trim();
                let disabled = current.is_empty();

                if disabled {
                    ui.label(
                        egui::RichText::new("Share link is disabled.")
                            .color(theme::text_muted()),
                    );
                } else if using_cloud {
                    ui.label(
                        egui::RichText::new("Using ReplayForge cloud (Cloudflare).")
                            .color(theme::success()),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("Using a custom Share API base.")
                            .color(theme::text_muted_light()),
                    );
                }

                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("API base (advanced)")
                        .color(theme::text_muted())
                        .size(12.0),
                );
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.config.share_api_base)
                            .desired_width(ui.available_width().min(480.0).max(220.0))
                            .hint_text(&default_base),
                    );
                    if response.changed() {
                        self.persist_config();
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add(theme::secondary_button("Use ReplayForge cloud"))
                        .clicked()
                    {
                        self.config.share_api_base = default_base;
                        self.persist_config();
                        self.toast("Share set to ReplayForge cloud");
                    }
                    if ui
                        .add_enabled(
                            !self.config.share_api_base.trim().is_empty(),
                            theme::secondary_button("Disable"),
                        )
                        .clicked()
                    {
                        self.config.share_api_base.clear();
                        self.persist_config();
                        self.toast("Share disabled");
                    }
                });
                ui.label(
                    egui::RichText::new(
                        "Create link uploads a clip once to ReplayForge cloud (max ~500 MB). \
                         Copy link reuses that URL; the share page shows the expiry date (~7 days). \
                         New link uploads again. Requires curl on PATH.",
                    )
                    .color(theme::text_muted())
                    .size(12.0),
                );
            });

            ui.add_space(12.0);

            theme::section_frame().show(ui, |ui| {
                ui.heading("About");
                ui.add_space(8.0);
                ui.label(format!("ReplayForge v{}", update::current_version()));
                ui.add_space(6.0);
                let checking = self.checking_update;
                let installing = self.installing_update;
                if ui
                    .add_enabled(
                        !checking && !installing,
                        theme::secondary_button(if checking {
                            "Checking…"
                        } else {
                            "Check for updates"
                        }),
                    )
                    .clicked()
                {
                    self.check_for_updates_action();
                }
                if let Some(pending) = self.pending_update.clone() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("v{} is ready to install", pending.latest))
                            .color(theme::success()),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !installing,
                                theme::primary_button(if installing {
                                    "Installing…"
                                } else {
                                    "Install update"
                                }),
                            )
                            .clicked()
                        {
                            self.install_update_action();
                        }
                        if ui
                            .add(theme::secondary_button("Open release notes"))
                            .clicked()
                        {
                            open_url(&pending.html_url);
                        }
                    });
                }
                ui.label(
                    egui::RichText::new(
                        "Checks GitHub for a newer release, then can download and install \
                         into ~/.local. Restart ReplayForge after installing.",
                    )
                    .color(theme::text_muted())
                    .size(12.0),
                );
            });

            ui.add_space(16.0);
            if self.settings_dirty {
                if ui
                    .add_sized([220.0, 36.0], theme::primary_button("Apply & Save"))
                    .clicked()
                {
                    self.apply_capture_settings();
                }
            } else if ui
                .add(theme::secondary_button("Save settings"))
                .clicked()
            {
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
        let mut save_copy = false;
        let mut back = false;
        let mut play_clicked = false;
        let mut nudge: Option<f64> = None;
        let mut space_toggle = false;

        if !self.trimming {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Escape) {
                    back = true;
                }
                if i.key_pressed(egui::Key::Space) {
                    space_toggle = true;
                }
                let step = if i.modifiers.shift { 2.0 } else { 0.5 };
                if i.key_pressed(egui::Key::ArrowLeft) {
                    nudge = Some(-step);
                } else if i.key_pressed(egui::Key::ArrowRight) {
                    nudge = Some(step);
                }
            });
        }

        if back {
            self.cancel_trim();
            return;
        }

        if let Some(delta) = nudge {
            self.stop_trim_playback();
            state.preview_secs =
                (state.preview_secs + delta).clamp(state.start_secs, state.end_secs);
        }

        if space_toggle {
            self.trim = Some(state.clone());
            self.toggle_trim_playback();
            if let Some(updated) = self.trim.clone() {
                state = updated;
            }
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
                        .add_enabled(!self.trimming, theme::secondary_button("← Back"))
                        .clicked()
                    {
                        back = true;
                    }
                    ui.heading(format!("Trim — {clip_name}"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_apply = range_valid && !self.trimming && !self.saving;
                        if ui
                            .add_enabled(can_apply, theme::primary_button("Apply trim"))
                            .on_hover_text("Replace this clip with the trimmed selection")
                            .clicked()
                        {
                            apply = true;
                        }
                        if ui
                            .add_enabled(can_apply, theme::secondary_button("Save copy"))
                            .on_hover_text("Keep the original and save the trim as a new clip")
                            .clicked()
                        {
                            save_copy = true;
                        }
                    });
                });
                ui.label(
                    egui::RichText::new("Save copy keeps the original clip.")
                        .color(theme::text_muted())
                        .size(11.0),
                );
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                const MAX_PREVIEW_W: f32 = 1400.0;
                // theme::section_frame() uses Margin::same(20) on each side.
                const FRAME_PAD: f32 = 40.0;
                let gap = 12.0_f32;
                let transport_h = 44.0_f32;
                let meta_h = 28.0_f32;
                let timeline_reserve = 88.0_f32;
                let spacing_reserve = gap * 3.0 + 8.0;

                let avail_w =
                    (ui.available_width() - 32.0 - FRAME_PAD).clamp(320.0, MAX_PREVIEW_W);
                let reserved =
                    transport_h + timeline_reserve + meta_h + spacing_reserve + FRAME_PAD;
                let avail_h = (ui.available_height() - reserved).max(180.0);

                let height_if_full_width = avail_w * 9.0 / 16.0;
                let (preview_width, preview_height) = if height_if_full_width <= avail_h {
                    (avail_w, height_if_full_width)
                } else {
                    let width_from_height = (avail_h * 16.0 / 9.0).min(avail_w);
                    (width_from_height, width_from_height * 9.0 / 16.0)
                };
                let preview_size = egui::vec2(preview_width, preview_height);
                let timeline_height = (preview_width * 0.06).clamp(56.0, 88.0);

                if !self.trim_filmstrip_pending {
                    if self.trim_filmstrip_texture.is_none() {
                        self.schedule_trim_filmstrip(preview_width);
                    } else if (preview_width - self.trim_filmstrip_width).abs() > 48.0 {
                        self.trim_filmstrip_texture = None;
                        self.schedule_trim_filmstrip(preview_width);
                    }
                }

                theme::section_frame().show(ui, |ui| {
                    ui.set_width(preview_width);
                    ui.vertical_centered(|ui| {
                        let preview_frame = egui::Frame::default()
                            .fill(theme::surface_track())
                            .corner_radius(theme::corner_radius());

                        preview_frame.show(ui, |ui| {
                            ui.set_width(preview_width);
                            ui.set_height(preview_height);
                            ui.centered_and_justified(|ui| {
                                if let Some(texture) = &self.trim_preview_texture {
                                    ui.add(
                                        egui::Image::new(texture).fit_to_exact_size(preview_size),
                                    );
                                } else if self.trim_preview_pending {
                                    ui.label(
                                        egui::RichText::new("Loading preview…")
                                            .color(theme::text_muted()),
                                    );
                                } else if let Some(error) = &self.trim_preview_error {
                                    ui.colored_label(theme::error(), error);
                                } else {
                                    ui.label(
                                        egui::RichText::new("Preview unavailable")
                                            .color(theme::text_muted()),
                                    );
                                }
                            });
                        });

                        ui.add_space(gap);

                        ui.horizontal(|ui| {
                            ui.set_width(preview_width);
                            if Self::trim_transport_button(ui, playing, !self.trimming) {
                                play_clicked = true;
                            }
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(&preview_label).size(18.0).strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!(" / {total_label}"))
                                    .size(18.0)
                                    .color(theme::text_muted()),
                            );

                            ui.add_space(16.0);

                            let mute_label = if self.trim_muted { "Unmute" } else { "Mute" };
                            if ui
                                .add(theme::secondary_button(mute_label))
                                .on_hover_text("Toggle preview audio")
                                .clicked()
                            {
                                self.trim_muted = !self.trim_muted;
                                self.apply_trim_volume();
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Gain")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                            let gain_slider = ui.add(
                                egui::Slider::new(&mut state.audio_gain, 0.0..=2.0)
                                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                                    .trailing_fill(true),
                            );
                            if gain_slider.changed() {
                                if state.audio_gain > 0.0 {
                                    self.trim_muted = false;
                                }
                                if self.trim_is_playing() {
                                    self.stop_trim_playback();
                                }
                            }
                        });
                        ui.label(
                            egui::RichText::new("Applied on export")
                                .color(theme::text_muted())
                                .size(11.0),
                        );

                        if self.config.capture_microphone {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Mic volume")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                );
                                let slider = theme::volume_slider(ui, &mut self.config.mic_volume).on_hover_text(
                                    "Adjusts the mic level in PipeWire for future recordings.",
                                );
                                if slider.drag_stopped() || slider.lost_focus() {
                                    let pct = (self.config.mic_volume * 100.0).round() as u32;
                                    let errors = apply_config_volumes(&self.config);
                                    if errors.is_empty() {
                                        self.toast(format!("Mic volume set to {pct}%"));
                                    } else {
                                        for error in errors {
                                            self.toast(error);
                                        }
                                    }
                                    self.persist_config();
                                }
                            });
                            ui.label(
                                egui::RichText::new("Affects future recordings (PipeWire)")
                                    .color(theme::text_muted())
                                    .size(11.0),
                            );
                        }

                        if self.trim_audio_error.is_some() {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Audio off — video-only preview")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                        } else if self.trim_muted {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Muted")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                        }

                        ui.add_space(8.0);

                        let filmstrip = self.trim_filmstrip_texture.clone();
                        let filmstrip_loading = self.trim_filmstrip_pending;
                        let waveform = self.trim_waveform.clone();
                        ui.allocate_ui_with_layout(
                            egui::vec2(preview_width, timeline_height),
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
                                    waveform.as_deref(),
                                    preview_width,
                                    timeline_height,
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
            });
        });

        self.trim = Some(state);

        if play_clicked {
            self.toggle_trim_playback();
        }

        if back {
            self.cancel_trim();
        } else if apply {
            self.apply_trim();
        } else if save_copy {
            self.save_trim_copy();
        }
    }
}

#[derive(Clone, Copy)]
enum ChillKind {
    ClipReady,
    Share,
    Delete,
    Trim,
}

fn is_safe_clip_stem(stem: &str) -> bool {
    if stem.is_empty() || stem == "." || stem == ".." || stem.contains("..") {
        return false;
    }
    !stem
        .chars()
        .any(|c| c == '/' || c == '\\' || c == '\0' || c.is_control())
}

fn chill_toast(kind: ChillKind) -> &'static str {
    let lines: &[&str] = match kind {
        ChillKind::ClipReady => &[
            "Caught that wave",
            "Clip's in the barrel",
            "Nice ride — saved",
            "It's all good — clip's ready",
        ],
        ChillKind::Share => &[
            "Link's out in the water",
            "Shared. Go with the flow",
            "Hang ten — link copied",
            "Chicken Joe cleared it for takeoff",
        ],
        ChillKind::Delete => &[
            "Gone with the tide",
            "That clip paddled out",
            "Wiped. Still chill",
            "No worries — it's gone",
        ],
        ChillKind::Trim => &[
            "Edited. Still gnarly",
            "Shaped that wave",
            "Cut clean — keep the best bit",
            "Trimmed. Cowabunga",
        ],
    };
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    lines[seed % lines.len()]
}

fn clip_storage_bytes(mp4: &Path) -> u64 {
    let mut n = fs::metadata(mp4).map(|m| m.len()).unwrap_or(0);
    let thumb = mp4.with_extension("png");
    if thumb.exists() {
        n += fs::metadata(&thumb).map(|m| m.len()).unwrap_or(0);
    }
    n
}

fn split_byte_label(label: &str) -> (&str, &str) {
    match label.rsplit_once(' ') {
        Some((value, unit)) => (value, unit),
        None => (label, ""),
    }
}

fn draw_clips_storage_stats(
    ui: &mut egui::Ui,
    visible_count: usize,
    visible_bytes: u64,
    library_count: usize,
    library_bytes: u64,
    filter_active: bool,
) {
    const SOFT_CAP_BYTES: f32 = 50.0 * 1024.0 * 1024.0 * 1024.0;
    let fill = (visible_bytes as f32 / SOFT_CAP_BYTES).clamp(0.0, 1.0);
    let stats_width = 160.0;
    let size_label = format_bytes(visible_bytes);
    let (value, unit) = split_byte_label(&size_label);
    let count_label = if visible_count == 1 {
        "1 clip".to_string()
    } else {
        format!("{visible_count} clips")
    };

    ui.horizontal(|ui| {
        let sep_height = if filter_active { 44.0 } else { 32.0 };
        let (sep_rect, _) =
            ui.allocate_exact_size(egui::vec2(1.0, sep_height), egui::Sense::hover());
        ui.painter()
            .rect_filled(sep_rect, 0.0, theme::stroke_subtle());
        ui.add_space(10.0);

        ui.vertical(|ui| {
            ui.set_width(stats_width);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&count_label)
                        .color(theme::text_muted())
                        .size(12.0),
                );
                ui.label(
                    egui::RichText::new("·")
                        .color(theme::text_muted())
                        .size(12.0),
                );
                if !unit.is_empty() {
                    ui.label(
                        egui::RichText::new(unit)
                            .color(theme::accent_bright())
                            .size(12.0),
                    );
                }
                ui.label(
                    egui::RichText::new(value)
                        .color(theme::accent())
                        .strong()
                        .size(20.0),
                );
            });

            if filter_active {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "of {library_count} · {}",
                            format_bytes(library_bytes)
                        ))
                        .color(theme::text_muted())
                        .size(11.0),
                    );
                });
            }

            ui.add_space(5.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(stats_width, 3.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.5, theme::surface_track());
            if fill > 0.0 {
                let mut fill_rect = rect;
                fill_rect.set_width((rect.width() * fill).max(2.0).min(rect.width()));
                ui.painter().rect_filled(fill_rect, 1.5, theme::accent());
            }
        });
    });
}
