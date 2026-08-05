use crate::config::Backend;
use crate::host::host_output;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const FLATPAK_GSR_ID: &str = "com.dec05eba.gpu_screen_recorder";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackend {
    Host,
    Flatpak,
}

#[derive(Debug, Clone)]
pub struct Monitor {
    pub name: String,
    pub resolution: Option<String>,
}

impl Monitor {
    pub fn label(&self) -> String {
        match &self.resolution {
            Some(res) => format!("{} ({})", self.name, res),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub backend: Option<ResolvedBackend>,
    pub monitors: Vec<Monitor>,
    pub error: Option<String>,
    pub host_gsr: bool,
    pub flatpak_gsr: bool,
}

impl Detection {
    pub fn refresh(preferred: Backend) -> Self {
        let host_gsr = host_has_gsr();
        let flatpak_gsr = flatpak_has_gsr();

        let backend = match preferred {
            Backend::Host if host_gsr => Some(ResolvedBackend::Host),
            Backend::Flatpak if flatpak_gsr => Some(ResolvedBackend::Flatpak),
            Backend::Auto if host_gsr => Some(ResolvedBackend::Host),
            Backend::Auto if flatpak_gsr => Some(ResolvedBackend::Flatpak),
            Backend::Host | Backend::Flatpak | Backend::Auto => None,
        };

        let mut detection = Self {
            backend,
            monitors: Vec::new(),
            error: None,
            host_gsr,
            flatpak_gsr,
        };

        if detection.backend.is_none() {
            detection.error = Some(
                "gpu-screen-recorder not found. Install it from your distro or Flatpak \
                 (com.dec05eba.gpu_screen_recorder)."
                    .into(),
            );
            return detection;
        }

        match list_monitors(detection.backend.unwrap()) {
            Ok(monitors) => {
                if monitors.is_empty() {
                    detection.monitors = vec![Monitor {
                        name: "screen".into(),
                        resolution: None,
                    }];
                } else {
                    detection.monitors = monitors;
                }
            }
            Err(error) => {
                detection.error = Some(error);
                detection.monitors = vec![Monitor {
                    name: "screen".into(),
                    resolution: None,
                }];
            }
        }

        detection
    }
}

fn which_exists(program: &str) -> bool {
    Command::new("which")
        .arg(program)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn host_has_gsr() -> bool {
    which_exists("gpu-screen-recorder")
        || host_output("which", &["gpu-screen-recorder"])
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn flatpak_has_gsr() -> bool {
    host_output("flatpak", &["info", FLATPAK_GSR_ID])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn list_monitors(backend: ResolvedBackend) -> Result<Vec<Monitor>, String> {
    let output = match backend {
        ResolvedBackend::Host => host_output("gpu-screen-recorder", &["--list-monitors"]),
        ResolvedBackend::Flatpak => host_output(
            "flatpak",
            &[
                "run",
                "--command=gpu-screen-recorder",
                FLATPAK_GSR_ID,
                "--list-monitors",
            ],
        ),
    }
    .map_err(|e| format!("Failed to list monitors: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to list monitors: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut monitors = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, resolution) = match line.split_once('|') {
            Some((name, res)) => (name.trim().to_string(), Some(res.trim().to_string())),
            None => (line.to_string(), None),
        };
        monitors.push(Monitor { name, resolution });
    }

    if !monitors.iter().any(|m| m.name == "screen") {
        monitors.insert(
            0,
            Monitor {
                name: "screen".into(),
                resolution: None,
            },
        );
    }

    Ok(monitors)
}

pub fn clip_duration_secs(path: &PathBuf) -> Option<f64> {
    let path_str = path.to_string_lossy();
    let output = host_output(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            &path_str,
        ],
    )
    .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
}

pub fn format_duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn probe_clip_meta(path: &PathBuf) -> (String, String) {
    let size_label = fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let duration_label = clip_duration_secs(path)
        .map(format_duration)
        .unwrap_or_else(|| "--:--".into());
    (duration_label, size_label)
}
