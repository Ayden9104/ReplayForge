use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::*;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Hide,
    SaveClip,
    ToggleReplay,
    Quit,
}

struct ReplayTray {
    shared: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    commands: Vec<TrayCommand>,
    running: bool,
}

impl ksni::Tray for ReplayTray {
    fn id(&self) -> String {
        "replayforge".into()
    }

    fn icon_name(&self) -> String {
        "video-display".into()
    }

    fn title(&self) -> String {
        "ReplayForge".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let running = self.shared.lock().unwrap().running;
        let status = if running {
            "Replay running"
        } else {
            "Replay stopped"
        };
        ksni::ToolTip {
            title: "ReplayForge".into(),
            description: status.into(),
            icon_name: "video-display".into(),
            icon_pixmap: vec![],
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let push = |shared: Arc<Mutex<Shared>>, cmd: TrayCommand| {
            Box::new(move |_this: &mut Self| {
                shared.lock().unwrap().commands.push(cmd);
            }) as Box<dyn Fn(&mut Self) + Send>
        };

        vec![
            StandardItem {
                label: "Show ReplayForge".into(),
                activate: push(self.shared.clone(), TrayCommand::Show),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide Window".into(),
                activate: push(self.shared.clone(), TrayCommand::Hide),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Save Clip".into(),
                activate: push(self.shared.clone(), TrayCommand::SaveClip),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start/Stop Replay".into(),
                activate: push(self.shared.clone(), TrayCommand::ToggleReplay),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: push(self.shared.clone(), TrayCommand::Quit),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.lock().unwrap().commands.push(TrayCommand::Show);
    }
}

pub struct TrayHandle {
    shared: Arc<Mutex<Shared>>,
    _handle: Handle<ReplayTray>,
}

impl TrayHandle {
    pub fn poll(&self) -> Vec<TrayCommand> {
        std::mem::take(&mut self.shared.lock().unwrap().commands)
    }

    pub fn set_running(&self, running: bool) {
        self.shared.lock().unwrap().running = running;
    }
}

pub fn create_tray() -> Result<TrayHandle, String> {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let tray = ReplayTray {
        shared: shared.clone(),
    };
    let handle = tray
        .spawn()
        .map_err(|e| format!("Failed to create tray icon: {e}"))?;

    Ok(TrayHandle {
        shared,
        _handle: handle,
    })
}
