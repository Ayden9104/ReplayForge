use crate::host::default_videos_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Auto,
    Host,
    Flatpak,
}

/// How non-mic desktop audio is captured when `capture_system_audio` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SystemAudioMode {
    #[default]
    All,
    Apps,
}

/// Capture quality preset mapped to GSR `-bm cbr` + `-q` bitrate (kbps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QualityPreset {
    Balanced,
    #[default]
    High,
    Ultra,
}

impl QualityPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    /// GSR CBR bitrate in kbps (replay-friendly).
    pub fn bitrate_kbps(self) -> u32 {
        match self {
            Self::Balanced => 8_000,
            Self::High => 15_000,
            Self::Ultra => 25_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub output_dir: PathBuf,
    pub display: String,
    pub fps: u32,
    pub buffer_seconds: u32,
    pub codec: String,
    pub hotkey: String,
    /// Opt-in Wayland global hotkeys via xdg-desktop-portal (no `input` group).
    #[serde(default)]
    pub portal_hotkey_enabled: bool,
    /// Include desktop/system audio in clips via GSR `-a default_output` or selected apps.
    #[serde(default = "default_true")]
    pub capture_system_audio: bool,
    /// Include microphone in clips via GSR `-a default_input` (merged with system when both on).
    #[serde(default = "default_true")]
    pub capture_microphone: bool,
    /// `All` = default_output; `Apps` = selected `audio_apps` as `app:Name`.
    #[serde(default)]
    pub system_audio_mode: SystemAudioMode,
    /// App names for GSR `app:` sources (no `app:` prefix). Used when mode is `Apps`.
    #[serde(default)]
    pub audio_apps: Vec<String>,
    #[serde(default)]
    pub quality: QualityPreset,
    pub backend: Backend,
    pub autostart: bool,
    /// Start the replay buffer automatically when ReplayForge opens (after first-run).
    #[serde(default)]
    pub auto_start_replay: bool,
    pub minimize_to_tray: bool,
    pub first_run_complete: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_dir: default_videos_dir(),
            display: "screen".to_string(),
            fps: 60,
            buffer_seconds: 60,
            codec: "h264".to_string(),
            hotkey: "F8".to_string(),
            portal_hotkey_enabled: false,
            capture_system_audio: true,
            capture_microphone: true,
            system_audio_mode: SystemAudioMode::All,
            audio_apps: Vec::new(),
            quality: QualityPreset::High,
            backend: Backend::Auto,
            autostart: false,
            auto_start_replay: false,
            minimize_to_tray: true,
            first_run_complete: false,
        }
    }
}

impl Config {
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "ReplayForge", "ReplayForge")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };

        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("Failed to parse config, using defaults: {error}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path =
            Self::config_path().ok_or_else(|| "Could not resolve config path".to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
        }
        let contents =
            toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize config: {e}"))?;
        fs::write(&path, contents).map_err(|e| format!("Failed to write config: {e}"))?;
        Ok(())
    }

    pub fn ensure_output_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.output_dir).map_err(|e| {
            format!(
                "Failed to create output dir {}: {e}",
                self.output_dir.display()
            )
        })
    }

    pub fn is_first_run(&self) -> bool {
        !self.first_run_complete
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(dirs) = directories::BaseDirs::new() else {
        return Err("Could not resolve home directory".into());
    };
    let autostart_dir = dirs.config_dir().join("autostart");
    let desktop_path = autostart_dir.join("replayforge.desktop");

    if !enabled {
        if desktop_path.exists() {
            fs::remove_file(&desktop_path)
                .map_err(|e| format!("Failed to remove autostart entry: {e}"))?;
        }
        return Ok(());
    }

    fs::create_dir_all(&autostart_dir)
        .map_err(|e| format!("Failed to create autostart dir: {e}"))?;

    let exec = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "replayforge".into());

    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=ReplayForge\n\
         Comment=Linux instant replay\n\
         Exec={exec}\n\
         Icon=replayforge\n\
         Terminal=false\n\
         Categories=AudioVideo;Recorder;\n\
         X-GNOME-Autostart-enabled=true\n"
    );

    fs::write(&desktop_path, contents).map_err(|e| format!("Failed to write autostart: {e}"))?;
    Ok(())
}

pub fn hotkey_choices() -> &'static [&'static str] {
    &[
        "F8",
        "F9",
        "F10",
        "F11",
        "Shift+F8",
        "Shift+F9",
        "Ctrl+F8",
        "Ctrl+Shift+F8",
        "Ctrl+Shift+F9",
    ]
}

pub fn codec_choices() -> &'static [&'static str] {
    &["h264", "hevc", "av1"]
}

pub fn quality_choices() -> &'static [QualityPreset] {
    &[
        QualityPreset::Balanced,
        QualityPreset::High,
        QualityPreset::Ultra,
    ]
}

pub fn path_display(path: &Path) -> String {
    path.display().to_string()
}
