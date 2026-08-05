use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalBackend {
    None,
    X11,
    Evdev,
}

pub struct HotkeyService {
    pub session: SessionKind,
    pub global_backend: GlobalBackend,
    pub status: String,
    manager: Option<GlobalHotKeyManager>,
    hotkey_id: Option<u32>,
    /// Set by background listeners (evdev / future portals).
    triggered: Arc<AtomicBool>,
    current_spec: String,
}

impl HotkeyService {
    pub fn start(spec: &str) -> Self {
        let session = detect_session();
        let triggered = Arc::new(AtomicBool::new(false));

        let mut service = Self {
            session,
            global_backend: GlobalBackend::None,
            status: String::new(),
            manager: None,
            hotkey_id: None,
            triggered: triggered.clone(),
            current_spec: spec.to_string(),
        };

        // Always try X11 global hotkeys when a display is available.
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

        // On Wayland (or if X11 global isn't active), try evdev for true global keys.
        if service.global_backend != GlobalBackend::X11 || session == SessionKind::Wayland {
            if let Some(count) = spawn_evdev_listener(spec, triggered) {
                if count > 0 {
                    // Prefer reporting evdev on Wayland even if X11 registered (X11 often inert).
                    if session == SessionKind::Wayland || service.global_backend == GlobalBackend::None
                    {
                        service.global_backend = GlobalBackend::Evdev;
                    }
                    service.status = format!(
                        "Global hotkey {} via evdev ({} keyboard{})",
                        spec,
                        count,
                        if count == 1 { "" } else { "s" }
                    );
                }
            }
        }

        service.refresh_status(spec);
        service
    }

    pub fn rebind(&mut self, spec: &str) {
        *self = Self::start(spec);
    }

    pub fn refresh_status(&mut self, spec: &str) {
        self.current_spec = spec.to_string();
        self.status = match (self.session, self.global_backend) {
            (SessionKind::Wayland, GlobalBackend::Evdev) => {
                format!("Global hotkey {spec} active (evdev / Wayland)")
            }
            (SessionKind::Wayland, GlobalBackend::X11) => {
                format!(
                    "Hotkey {spec}: X11 bind registered, but this is Wayland — \
                     use focused window or grant /dev/input access for global keys"
                )
            }
            (SessionKind::Wayland, GlobalBackend::None) => {
                format!(
                    "Hotkey {spec}: works when ReplayForge is focused. \
                     For global keys on Wayland, add your user to the input group \
                     (then re-login): sudo usermod -aG input $USER"
                )
            }
            (_, GlobalBackend::X11) => format!("Global hotkey {spec} active (X11)"),
            (_, GlobalBackend::Evdev) => format!("Global hotkey {spec} active (evdev)"),
            (_, GlobalBackend::None) => {
                format!("Hotkey {spec}: works when ReplayForge is focused")
            }
        };
    }

    /// Returns true once per press (global X11, evdev, or callers also check egui).
    pub fn poll_global_pressed(&self) -> bool {
        let mut pressed = false;

        if let Some(id) = self.hotkey_id {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.id == id && event.state == HotKeyState::Pressed {
                    pressed = true;
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
    // Linux input event KEY_* values.
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

fn spawn_evdev_listener(spec: &str, triggered: Arc<AtomicBool>) -> Option<usize> {
    let (modifiers, code) = parse_spec(spec)?;
    let key_code = linux_key_code(code)?;

    let want_ctrl = modifiers.contains(Modifiers::CONTROL);
    let want_shift = modifiers.contains(Modifiers::SHIFT);
    let want_alt = modifiers.contains(Modifiers::ALT);
    let want_super = modifiers.contains(Modifiers::SUPER);

    let mut opened: Vec<(PathBuf, evdev::Device)> = Vec::new();
    for (path, device) in evdev::enumerate() {
        let is_keyboard = device
            .supported_events()
            .contains(evdev::EventType::KEY)
            && device
                .supported_keys()
                .is_some_and(|keys| keys.iter().count() > 20);

        if !is_keyboard {
            continue;
        }

        match evdev::Device::open(&path) {
            Ok(dev) => {
                println!("Hotkey listener on {}", path.display());
                opened.push((path, dev));
            }
            Err(error) => {
                eprintln!("Cannot open {}: {error}", path.display());
            }
        }
    }

    let count = opened.len();
    if count == 0 {
        return Some(0);
    }

    thread::spawn(move || {
        let mut shift = false;
        let mut ctrl = false;
        let mut alt = false;
        let mut super_key = false;

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
                    let pressed = event.value() != 0;
                    // Modifier tracking (left/right variants).
                    match code {
                        29 | 97 => ctrl = pressed,     // CTRL
                        42 | 54 => shift = pressed,    // SHIFT
                        56 | 100 => alt = pressed,     // ALT
                        125 | 126 => super_key = pressed, // META
                        _ => {}
                    }

                    if code == key_code && event.value() == 1 {
                        let mods_ok = ctrl == want_ctrl
                            && shift == want_shift
                            && alt == want_alt
                            && super_key == want_super;
                        if mods_ok {
                            triggered.store(true, Ordering::Release);
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    Some(count)
}

use eframe::egui;
