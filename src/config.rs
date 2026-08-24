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

    /// Base GSR CBR bitrate in kbps at 1080p (replay-friendly).
    fn base_bitrate_kbps(self) -> u32 {
        match self {
            Self::Balanced => 8_000,
            Self::High => 15_000,
            Self::Ultra => 25_000,
        }
    }

    /// GSR CBR bitrate in kbps, scaled by output resolution area vs 1080p.
    pub fn bitrate_kbps(self, resolution: &str) -> u32 {
        let base = self.base_bitrate_kbps() as f64;
        let area_1080 = 1920.0 * 1080.0;
        let area = resolution_pixel_area(resolution).unwrap_or(area_1080);
        let scaled = (base * area / area_1080).round() as u32;
        scaled.clamp(4_000, 60_000)
    }
}

/// Pixel area for known resolution presets. `native` / unknown → `None` (treat as 1080p).
fn resolution_pixel_area(resolution: &str) -> Option<f64> {
    match resolution {
        "native" | "" => None,
        "1280x720" => Some(1280.0 * 720.0),
        "1920x1080" => Some(1920.0 * 1080.0),
        "2560x1440" => Some(2560.0 * 1440.0),
        "3840x2160" => Some(3840.0 * 2160.0),
        other => {
            let (w, h) = other.split_once('x')?;
            let w: f64 = w.parse().ok()?;
            let h: f64 = h.parse().ok()?;
            if w > 0.0 && h > 0.0 {
                Some(w * h)
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppTheme {
    #[default]
    Classic,
    Arma3,
}

impl AppTheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Arma3 => "ArmA 3",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub output_dir: PathBuf,
    pub display: String,
    pub fps: u32,
    /// Output resolution limit for GSR `-s` (`native` = monitor size).
    #[serde(default = "default_resolution")]
    pub resolution: String,
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
    /// After saving a clip, open the trim page for it automatically.
    #[serde(default)]
    pub open_trim_after_save: bool,
    /// Optional override for the clip-save cue; `None` uses the bundled WAV.
    #[serde(default)]
    pub clip_sound_path: Option<PathBuf>,
    /// Linear gain for clip SFX (bundled, custom, and fallback). `1.0` is default loudness.
    #[serde(default = "default_sfx_volume")]
    pub sfx_volume: f32,
    /// When true, "Clip ready" desktop notifications use critical urgency.
    #[serde(default = "default_true")]
    pub clip_ready_notify_critical: bool,
    /// Base URL of the share Worker (empty disables Share).
    #[serde(default = "default_share_api_base")]
    pub share_api_base: String,
    /// UI theme (Classic blue utility or ArmA 3 chrome).
    #[serde(default)]
    pub theme: AppTheme,
    pub first_run_complete: bool,
}

fn default_true() -> bool {
    true
}

fn default_sfx_volume() -> f32 {
    1.0
}

fn default_resolution() -> String {
    "native".to_string()
}

/// Hosted ReplayForge share Worker (Cloudflare). Empty `share_api_base` disables Share.
pub fn default_share_api_base() -> String {
    "https://replayforge-share.holdup6699.workers.dev".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_dir: default_videos_dir(),
            display: "screen".to_string(),
            fps: 60,
            resolution: default_resolution(),
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
            open_trim_after_save: false,
            clip_sound_path: None,
            sfx_volume: default_sfx_volume(),
            clip_ready_notify_critical: true,
            share_api_base: default_share_api_base(),
            theme: AppTheme::Classic,
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

/// `(config_value, ui_label)` pairs for recording resolution presets.
pub fn resolution_choices() -> &'static [(&'static str, &'static str)] {
    &[
        ("native", "Native (monitor)"),
        ("1920x1080", "1080p"),
        ("1280x720", "720p"),
        ("2560x1440", "1440p"),
        ("3840x2160", "4K"),
    ]
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
