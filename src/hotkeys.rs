use crate::host::needs_host_spawn;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use eframe::egui;
use evdev::KeyCode;
use futures_util::StreamExt;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PORTAL_SHORTCUT_ID: &str = "save_clip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalBackend {
    None,
    Portal,
    X11,
    Evdev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EvdevSetup {
    Active { keyboards: usize },
    PermissionDenied { attempted: usize },
    NoKeyboards,
    Skipped,
}

pub struct HotkeyService {
    pub session: SessionKind,
    pub global_backend: GlobalBackend,
    pub status: String,
    manager: Option<GlobalHotKeyManager>,
    hotkey_id: Option<u32>,
    /// Set by background listeners (portal / evdev).
    triggered: Arc<AtomicBool>,
    current_spec: String,
    evdev_setup: EvdevSetup,
    in_toolbox: bool,
    portal_stop: Option<Arc<AtomicBool>>,
    portal_supported: Option<bool>,
    portal_trigger: Option<String>,
}

impl HotkeyService {
    pub fn start(spec: &str, portal_enabled: bool) -> Self {
        let session = detect_session();
        let in_toolbox = needs_host_spawn();
        let triggered = Arc::new(AtomicBool::new(false));

        let mut service = Self {
            session,
            global_backend: GlobalBackend::None,
            status: String::new(),
            manager: None,
            hotkey_id: None,
            triggered: triggered.clone(),
            current_spec: spec.to_string(),
            evdev_setup: EvdevSetup::Skipped,
            in_toolbox,
            portal_stop: None,
            portal_supported: None,
            portal_trigger: None,
        };

        // Prefer portal when the user has opted in (Wayland-friendly, no sudo).
        // Restore via ListShortcuts only — never Bind here (that would risk a
        // startup dialog). Fresh bind happens from Settings → Enable.
        if portal_enabled {
            match service.attach_portal(spec, false) {
                Ok(_) => {
                    service.refresh_status(spec);
                    return service;
                }
                Err(error) => {
                    eprintln!("Portal hotkey attach failed: {error}");
                }
            }
        }

        // X11 global-hotkey is only meaningful on real X11 sessions.
        // On Wayland, registration may succeed via XWayland but never receive in-game keys.
        let use_x11_global = session == SessionKind::X11;
        if use_x11_global {
            match GlobalHotKeyManager::new() {
                Ok(manager) => {
                    if let Some(hotkey) = parse_hotkey(spec) {
                        let id = hotkey.id();
                        match manager.register(hotkey) {
                            Ok(()) => {
                                service.hotkey_id = Some(id);
                                service.manager = Some(manager);
                                service.global_backend = GlobalBackend::X11;
                            }
                            Err(error) => {
                                eprintln!("X11 hotkey register failed: {error}");
                                service.manager = Some(manager);
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("GlobalHotKeyManager unavailable: {error}");
                }
            }
        }

        // Wayland always tries evdev. On X11, try evdev only if X11 global failed.
        // Skip when portal is already active (attach_portal returns early above).
        let try_evdev = session == SessionKind::Wayland
            || session == SessionKind::Unknown
            || service.global_backend == GlobalBackend::None;

        if try_evdev {
            let setup = spawn_evdev_listener(spec, triggered);
            service.evdev_setup = setup.clone();
            if let EvdevSetup::Active { .. } = setup {
                service.global_backend = GlobalBackend::Evdev;
            }
        }

        service.refresh_status(spec);
        service
    }

    pub fn rebind(&mut self, spec: &str, portal_enabled: bool) {
        let was_portal = self.global_backend == GlobalBackend::Portal;
        self.stop_portal();
        if was_portal && portal_enabled {
            // Re-bind so the preferred trigger tracks the Settings combo.
            *self = Self::start(spec, false);
            if let Err(error) = self.enable_portal(spec) {
                eprintln!("Portal rebind failed: {error}");
                *self = Self::start(spec, true);
            }
        } else {
            *self = Self::start(spec, portal_enabled);
        }
    }

    pub fn is_portal_active(&self) -> bool {
        self.global_backend == GlobalBackend::Portal
    }

    /// Bind (may show a portal dialog), then listen for Activated.
    pub fn enable_portal(&mut self, spec: &str) -> Result<String, String> {
        self.stop_portal();
        // Tear down competing backends so we don't double-fire.
        self.manager = None;
        self.hotkey_id = None;
        self.evdev_setup = EvdevSetup::Skipped;
        self.global_backend = GlobalBackend::None;

        self.attach_portal(spec, true)?;
        self.refresh_status(spec);
        Ok(self
            .portal_trigger
            .clone()
            .unwrap_or_else(|| format!("bound ({spec})")))
    }

    /// Re-open portal UI to configure the bound shortcut (via BindShortcuts on ashpd 0.11).
    pub fn configure_portal(&mut self) -> Result<(), String> {
        if self.global_backend != GlobalBackend::Portal {
            return Err("Portal global hotkey is not active".into());
        }
        let spec = self.current_spec.clone();
        self.enable_portal(&spec).map(|_| ())
    }

    fn attach_portal(&mut self, spec: &str, force_bind: bool) -> Result<(), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let triggered = self.triggered.clone();
        let result = spawn_portal_listener(spec.to_string(), force_bind, triggered, stop.clone());
        match result {
            Ok(trigger) => {
                self.portal_supported = Some(true);
                self.portal_stop = Some(stop);
                self.portal_trigger = Some(trigger);
                self.global_backend = GlobalBackend::Portal;
                self.evdev_setup = EvdevSetup::Skipped;
                self.current_spec = spec.to_string();
                Ok(())
            }
            Err(error) => {
                let unsupported = error.to_ascii_lowercase().contains("unknown method")
                    || error.to_ascii_lowercase().contains("does not exist")
                    || error.to_ascii_lowercase().contains("not supported")
                    || error.to_ascii_lowercase().contains("no such interface");
                if unsupported {
                    self.portal_supported = Some(false);
                } else if self.portal_supported.is_none() {
                    // Soft probe: portal may exist but nothing bound yet.
                    self.portal_supported = Some(true);
                }
                Err(error)
            }
        }
    }

    fn stop_portal(&mut self) {
        if let Some(stop) = self.portal_stop.take() {
            stop.store(true, Ordering::Release);
        }
        self.portal_trigger = None;
        if self.global_backend == GlobalBackend::Portal {
            self.global_backend = GlobalBackend::None;
        }
    }

    pub fn refresh_status(&mut self, spec: &str) {
        self.current_spec = spec.to_string();
        let toolbox_note = if self.in_toolbox {
            " If you are in Toolbox/Distrobox, run ReplayForge on the host so it can read /dev/input."
        } else {
            ""
        };

        self.status = match (self.session, self.global_backend, &self.evdev_setup) {
            (_, GlobalBackend::Portal, _) => {
                let trigger = self.portal_trigger.as_deref().unwrap_or(spec);
                format!("Global hotkey {spec} active (portal: {trigger})")
            }
            (_, GlobalBackend::Evdev, EvdevSetup::Active { keyboards }) => {
                format!(
                    "Global hotkey {spec} active (evdev, {keyboards} keyboard{})",
                    if *keyboards == 1 { "" } else { "s" }
                )
            }
            (SessionKind::X11, GlobalBackend::X11, _) => {
                format!("Global hotkey {spec} active (X11)")
            }
            (SessionKind::Wayland, _, EvdevSetup::PermissionDenied { attempted }) => {
                format!(
                    "Hotkey {spec}: focused-only. Global keys blocked ({attempted} device(s) denied). \
                     Prefer Settings → Enable global hotkey (portal). \
                     Advanced: sudo usermod -aG input $USER then re-login.{toolbox_note}"
                )
            }
            (SessionKind::Wayland, _, EvdevSetup::NoKeyboards) => {
                format!(
                    "Hotkey {spec}: focused-only. No readable keyboards found under /dev/input. \
                     Prefer Settings → Enable global hotkey (portal).{toolbox_note}"
                )
            }
            (SessionKind::Wayland, _, _) => {
                format!(
                    "Hotkey {spec}: works when ReplayForge is focused. \
                     For in-game keys: Settings → Enable global hotkey (portal). \
                     Advanced fallback: sudo usermod -aG input $USER then re-login.{toolbox_note}"
                )
            }
            (_, GlobalBackend::None, EvdevSetup::PermissionDenied { .. }) => {
                format!(
                    "Hotkey {spec}: focused-only. Prefer portal in Settings, or grant /dev/input: \
                     sudo usermod -aG input $USER then re-login.{toolbox_note}"
                )
            }
            _ => format!("Hotkey {spec}: works when ReplayForge is focused"),
        };
    }

    /// Returns true once per press (global portal / X11 / evdev). Callers also check egui.
    pub fn poll_global_pressed(&self) -> bool {
        let mut pressed = false;

        // Only consume X11 events when X11 is the active global backend.
        if self.global_backend == GlobalBackend::X11 {
            if let Some(id) = self.hotkey_id {
                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.id == id && event.state == HotKeyState::Pressed {
                        pressed = true;
                    }
                }
            }
        }

        if self
            .triggered
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            pressed = true;
        }

        pressed
    }

    pub fn matches_egui(&self, ctx: &egui::Context) -> bool {
        matches_egui_hotkey(ctx, &self.current_spec)
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stop_portal();
    }
}

pub fn detect_session() -> SessionKind {
    match std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "wayland" => SessionKind::Wayland,
        "x11" => SessionKind::X11,
        _ => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                SessionKind::Wayland
            } else if std::env::var_os("DISPLAY").is_some() {
                SessionKind::X11
            } else {
                SessionKind::Unknown
            }
        }
    }
}

pub fn parse_hotkey(spec: &str) -> Option<HotKey> {
    let (modifiers, code) = parse_spec(spec)?;
    let mods = if modifiers.is_empty() {
        None
    } else {
        Some(modifiers)
    };
    Some(HotKey::new(mods, code))
}

/// Map config hotkey (e.g. `Shift+F8`) to an XDG shortcuts preferred trigger (`<Shift>F8`).
pub fn portal_preferred_trigger(spec: &str) -> Option<String> {
    let (modifiers, code) = parse_spec(spec)?;
    let key = match code {
        Code::F1 => "F1",
        Code::F2 => "F2",
        Code::F3 => "F3",
        Code::F4 => "F4",
        Code::F5 => "F5",
        Code::F6 => "F6",
        Code::F7 => "F7",
        Code::F8 => "F8",
        Code::F9 => "F9",
        Code::F10 => "F10",
        Code::F11 => "F11",
        Code::F12 => "F12",
        _ => return None,
    };
    let mut out = String::new();
    if modifiers.contains(Modifiers::CONTROL) {
        out.push_str("<Control>");
    }
    if modifiers.contains(Modifiers::ALT) {
        out.push_str("<Alt>");
    }
    if modifiers.contains(Modifiers::SHIFT) {
        out.push_str("<Shift>");
    }
    if modifiers.contains(Modifiers::SUPER) {
        out.push_str("<Super>");
    }
    out.push_str(key);
    Some(out)
}

fn spawn_portal_listener(
    spec: String,
    force_bind: bool,
    triggered: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();

    thread::Builder::new()
        .name("replayforge-portal-hotkey".into())
        .spawn(move || {
            let outcome = async_std::task::block_on(async {
                run_portal_session(spec, force_bind, triggered, stop, &tx).await
            });
            if let Err(error) = outcome {
                // If ready was never sent, report the failure.
                let _ = tx.send(Err(error));
            }
        })
        .map_err(|e| format!("Failed to spawn portal thread: {e}"))?;

    // Bind / re-attach can wait on a user dialog; allow a long timeout.
    // First-launch opt-in is gated by portal_hotkey_enabled (default false).
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(Ok(trigger)) => Ok(trigger),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("Timed out waiting for portal global shortcuts".into()),
    }
}

async fn run_portal_session(
    spec: String,
    force_bind: bool,
    triggered: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ready_tx: &std::sync::mpsc::Sender<Result<String, String>>,
) -> Result<(), String> {
    let portal = GlobalShortcuts::new()
        .await
        .map_err(|e| format!("GlobalShortcuts portal unavailable: {e}"))?;
    let session = portal
        .create_session()
        .await
        .map_err(|e| format!("CreateSession failed: {e}"))?;

    let trigger_description = if force_bind {
        bind_save_clip(&portal, &session, &spec).await?
    } else {
        let request = portal
            .list_shortcuts(&session)
            .await
            .map_err(|e| format!("ListShortcuts failed: {e}"))?;
        let listed = request
            .response()
            .map_err(|e| format!("ListShortcuts rejected: {e}"))?;
        if listed
            .shortcuts()
            .iter()
            .all(|s| s.id() != PORTAL_SHORTCUT_ID)
        {
            return Err(
                "No portal shortcut bound yet — use Settings → Enable global hotkey (portal)"
                    .into(),
            );
        }
        // Re-attach to this session. Already-known shortcuts usually skip the dialog.
        bind_save_clip(&portal, &session, &spec).await?
    };

    // Keep session alive for the lifetime of the listener.
    let _session = session;

    ready_tx
        .send(Ok(trigger_description))
        .map_err(|_| "Portal ready channel closed".to_string())?;

    let mut stream = portal
        .receive_activated()
        .await
        .map_err(|e| format!("receive_activated failed: {e}"))?;

    while !stop.load(Ordering::Acquire) {
        match async_std::future::timeout(Duration::from_millis(400), stream.next()).await {
            Ok(Some(event)) => {
                if event.shortcut_id() == PORTAL_SHORTCUT_ID {
                    triggered.store(true, Ordering::Release);
                }
            }
            Ok(None) => break,
            Err(_) => {
                // Timeout — loop to re-check stop.
            }
        }
    }

    Ok(())
}

async fn bind_save_clip(
    portal: &GlobalShortcuts<'_>,
    session: &ashpd::desktop::Session<'_, GlobalShortcuts<'_>>,
    spec: &str,
) -> Result<String, String> {
    let preferred = portal_preferred_trigger(spec);
    let mut shortcut = NewShortcut::new(PORTAL_SHORTCUT_ID, "Save ReplayForge clip");
    if let Some(ref trigger) = preferred {
        shortcut = shortcut.preferred_trigger(trigger.as_str());
    }
    let request = portal
        .bind_shortcuts(session, &[shortcut], None)
        .await
        .map_err(|e| format!("BindShortcuts failed: {e}"))?;
    let bound = request
        .response()
        .map_err(|e| format!("BindShortcuts rejected: {e}"))?;
    Ok(bound
        .shortcuts()
        .iter()
        .find(|s| s.id() == PORTAL_SHORTCUT_ID)
        .map(|s| s.trigger_description().to_string())
        .or(preferred)
        .unwrap_or_else(|| spec.to_string()))
}

fn parse_spec(spec: &str) -> Option<(Modifiers, Code)> {
    let mut modifiers = Modifiers::empty();
    let mut key: Option<Code> = None;

    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            "super" | "meta" | "win" => modifiers |= Modifiers::SUPER,
            other => key = code_from_str(other),
        }
    }

    Some((modifiers, key?))
}

fn code_from_str(s: &str) -> Option<Code> {
    Some(match s.to_ascii_uppercase().as_str() {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        _ => return None,
    })
}

fn matches_egui_hotkey(ctx: &egui::Context, spec: &str) -> bool {
    let Some((modifiers, code)) = parse_spec(spec) else {
        return false;
    };
    let Some(egui_key) = code_to_egui(code) else {
        return false;
    };

    let wants_ctrl = modifiers.contains(Modifiers::CONTROL);
    let wants_shift = modifiers.contains(Modifiers::SHIFT);
    let wants_alt = modifiers.contains(Modifiers::ALT);
    let wants_super = modifiers.contains(Modifiers::SUPER);

    ctx.input(|i| {
        if !i.key_pressed(egui_key) {
            return false;
        }
        let mods = &i.modifiers;
        mods.ctrl == wants_ctrl
            && mods.shift == wants_shift
            && mods.alt == wants_alt
            && mods.mac_cmd == wants_super
    })
}

fn code_to_egui(code: Code) -> Option<egui::Key> {
    Some(match code {
        Code::F1 => egui::Key::F1,
        Code::F2 => egui::Key::F2,
        Code::F3 => egui::Key::F3,
        Code::F4 => egui::Key::F4,
        Code::F5 => egui::Key::F5,
        Code::F6 => egui::Key::F6,
        Code::F7 => egui::Key::F7,
        Code::F8 => egui::Key::F8,
        Code::F9 => egui::Key::F9,
        Code::F10 => egui::Key::F10,
        Code::F11 => egui::Key::F11,
        Code::F12 => egui::Key::F12,
        _ => return None,
    })
}

fn linux_key_code(code: Code) -> Option<u16> {
    Some(match code {
        Code::F1 => 59,
        Code::F2 => 60,
        Code::F3 => 61,
        Code::F4 => 62,
        Code::F5 => 63,
        Code::F6 => 64,
        Code::F7 => 65,
        Code::F8 => 66,
        Code::F9 => 67,
        Code::F10 => 68,
        Code::F11 => 87,
        Code::F12 => 88,
        _ => return None,
    })
}

fn keycode_from_u16(code: u16) -> Option<KeyCode> {
    // Map the F-keys we support onto evdev KeyCode for capability checks.
    Some(match code {
        59 => KeyCode::KEY_F1,
        60 => KeyCode::KEY_F2,
        61 => KeyCode::KEY_F3,
        62 => KeyCode::KEY_F4,
        63 => KeyCode::KEY_F5,
        64 => KeyCode::KEY_F6,
        65 => KeyCode::KEY_F7,
        66 => KeyCode::KEY_F8,
        67 => KeyCode::KEY_F9,
        68 => KeyCode::KEY_F10,
        87 => KeyCode::KEY_F11,
        88 => KeyCode::KEY_F12,
        _ => return None,
    })
}

fn spawn_evdev_listener(spec: &str, triggered: Arc<AtomicBool>) -> EvdevSetup {
    let Some((modifiers, code)) = parse_spec(spec) else {
        return EvdevSetup::NoKeyboards;
    };
    let Some(key_code) = linux_key_code(code) else {
        return EvdevSetup::NoKeyboards;
    };
    let target_key = keycode_from_u16(key_code);

    let want_ctrl = modifiers.contains(Modifiers::CONTROL);
    let want_shift = modifiers.contains(Modifiers::SHIFT);
    let want_alt = modifiers.contains(Modifiers::ALT);
    let want_super = modifiers.contains(Modifiers::SUPER);

    let mut preferred: Vec<(PathBuf, evdev::Device)> = Vec::new();
    let mut fallback: Vec<(PathBuf, evdev::Device)> = Vec::new();
    let mut attempted = 0usize;
    let mut denied = 0usize;
    let mut seen_keyboardish = 0usize;

    for (path, device) in evdev::enumerate() {
        let has_keys = device.supported_events().contains(evdev::EventType::KEY);
        if !has_keys {
            continue;
        }

        let key_count = device
            .supported_keys()
            .map(|keys| keys.iter().count())
            .unwrap_or(0);
        let supports_target = target_key
            .and_then(|k| device.supported_keys().map(|keys| keys.contains(k)))
            .unwrap_or(false);

        // Prefer real keyboards: many keys and/or the target F-key.
        let keyboardish = supports_target || key_count > 20;
        if !keyboardish {
            continue;
        }
        seen_keyboardish += 1;
        attempted += 1;

        match evdev::Device::open(&path) {
            Ok(dev) => {
                println!("Hotkey listener on {}", path.display());
                if supports_target {
                    preferred.push((path, dev));
                } else {
                    fallback.push((path, dev));
                }
            }
            Err(error) => {
                let kind = error.kind();
                if kind == std::io::ErrorKind::PermissionDenied {
                    denied += 1;
                }
                eprintln!("Cannot open {}: {error}", path.display());
            }
        }
    }

    let mut opened = preferred;
    if opened.is_empty() {
        opened = fallback;
    }

    if opened.is_empty() {
        if denied > 0 {
            return EvdevSetup::PermissionDenied {
                attempted: attempted.max(denied),
            };
        }
        if seen_keyboardish == 0 && attempted == 0 {
            // Might still be permission: enumerate can succeed while open fails on all.
            // If we saw nothing keyboardish at all, report NoKeyboards.
            return EvdevSetup::NoKeyboards;
        }
        return EvdevSetup::NoKeyboards;
    }

    let keyboards = opened.len();
    let last_fire = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));

    thread::spawn(move || {
        let mut shift = false;
        let mut ctrl = false;
        let mut alt = false;
        let mut super_key = false;
        let debounce = Duration::from_millis(300);

        loop {
            for (_, device) in &mut opened {
                let Ok(events) = device.fetch_events() else {
                    continue;
                };
                for event in events {
                    if event.event_type() != evdev::EventType::KEY {
                        continue;
                    }
                    let code = event.code();
                    let value = event.value();

                    // Modifier tracking (left/right). value 0=up, 1=down, 2=repeat.
                    let pressed = value != 0;
                    match code {
                        29 | 97 => ctrl = pressed,
                        42 | 54 => shift = pressed,
                        56 | 100 => alt = pressed,
                        125 | 126 => super_key = pressed,
                        _ => {}
                    }

                    // Key-down only (ignore release and autorepeat).
                    if code == key_code && value == 1 {
                        let mods_ok = ctrl == want_ctrl
                            && shift == want_shift
                            && alt == want_alt
                            && super_key == want_super;
                        if !mods_ok {
                            continue;
                        }

                        let now = Instant::now();
                        let mut last = last_fire.lock().unwrap();
                        if now.duration_since(*last) < debounce {
                            continue;
                        }
                        *last = now;
                        drop(last);
                        triggered.store(true, Ordering::Release);
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    EvdevSetup::Active { keyboards }
}
