use crate::config::Config;
use crate::detect::{FLATPAK_GSR_ID, ResolvedBackend};
use crate::host::{host_command, host_output};
use chrono::Local;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime};

pub struct Recorder {
    process: Option<Child>,
    backend: Option<ResolvedBackend>,
    output_dir: PathBuf,
    last_error: Option<String>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            process: None,
            backend: None,
            output_dir: PathBuf::new(),
            last_error: None,
        }
    }
}

impl Recorder {
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn start(&mut self, config: &Config, backend: ResolvedBackend) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }

        config.ensure_output_dir()?;

        let fps = config.fps.to_string();
        let buffer = config.buffer_seconds.to_string();
        let output = config.output_dir.to_string_lossy().to_string();

        let mut child = match backend {
            ResolvedBackend::Host => {
                let mut cmd = host_command(
                    "gpu-screen-recorder",
                    &[
                        "-w",
                        &config.display,
                        "-f",
                        &fps,
                        "-r",
                        &buffer,
                        "-c",
                        "mp4",
                        "-replay-storage",
                        "ram",
                        "-k",
                        &config.codec,
                        "-o",
                        &output,
                    ],
                );
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                cmd.spawn()
                    .map_err(|e| format!("Failed to start gpu-screen-recorder: {e}"))?
            }
            ResolvedBackend::Flatpak => {
                let mut cmd = host_command(
                    "flatpak",
                    &[
                        "run",
                        "--command=gpu-screen-recorder",
                        FLATPAK_GSR_ID,
                        "-w",
                        &config.display,
                        "-f",
                        &fps,
                        "-r",
                        &buffer,
                        "-c",
                        "mp4",
                        "-replay-storage",
                        "ram",
                        "-k",
                        &config.codec,
                        "-o",
                        &output,
                    ],
                );
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                cmd.spawn()
                    .map_err(|e| format!("Failed to start Flatpak gpu-screen-recorder: {e}"))?
            }
        };

        // Give it a moment; if it exits immediately, surface the failure.
        thread::sleep(Duration::from_millis(400));
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "gpu-screen-recorder exited immediately ({status}). Check display/codec settings."
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(format!("Failed to check recorder process: {error}"));
            }
        }

        self.process = Some(child);
        self.backend = Some(backend);
        self.output_dir = config.output_dir.clone();
        self.last_error = None;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if self.process.is_none() {
            return Ok(());
        }

        let result = match self.backend {
            Some(ResolvedBackend::Flatpak) => {
                host_command("flatpak", &["kill", FLATPAK_GSR_ID]).status()
            }
            _ => {
                // Prefer signaling our child; fall back to pkill.
                if let Some(child) = self.process.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                host_command("pkill", &["-INT", "-f", "^gpu-screen-recorder"]).status()
            }
        };

        match result {
            Ok(_) => {
                self.process = None;
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                let msg = format!("Failed to stop replay buffer: {error}");
                self.last_error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    pub fn save_clip(&mut self) -> Result<PathBuf, String> {
        if !self.is_running() {
            let msg = "Cannot save clip: replay buffer is not running".to_string();
            self.last_error = Some(msg.clone());
            return Err(msg);
        }

        let before = Self::newest_mp4(&self.output_dir);

        let status = host_command("pkill", &["-SIGUSR1", "-f", "^gpu-screen-recorder"])
            .status()
            .map_err(|e| {
                let msg = format!("Failed to run save command: {e}");
                self.last_error = Some(msg.clone());
                msg
            })?;

        if !status.success() {
            let msg = format!("Failed to save clip: {status}");
            self.last_error = Some(msg.clone());
            return Err(msg);
        }

        // Wait for the new file to appear.
        let mut clip = None;
        for _ in 0..40 {
            thread::sleep(Duration::from_millis(100));
            if let Some(newest) = Self::newest_mp4(&self.output_dir) {
                let is_new = match &before {
                    Some(prev) => newest != *prev,
                    None => true,
                };
                if is_new {
                    clip = Some(newest);
                    break;
                }
            }
        }

        let clip = clip.ok_or_else(|| {
            let msg = "Clip save signaled, but no new file appeared".to_string();
            self.last_error = Some(msg.clone());
            msg
        })?;

        let renamed = Self::rename_clip(clip);
        Self::generate_thumbnail(&renamed);
        self.last_error = None;
        Ok(renamed)
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(process) = self.process.as_mut() {
            match process.try_wait() {
                Ok(Some(_)) => {
                    self.process = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    fn newest_mp4(folder: &PathBuf) -> Option<PathBuf> {
        let mut newest: Option<(PathBuf, SystemTime)> = None;

        let entries = fs::read_dir(folder).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let is_mp4 = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
            if !is_mp4 {
                continue;
            }
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    match &newest {
                        Some((_, newest_time)) if modified <= *newest_time => {}
                        _ => newest = Some((path, modified)),
                    }
                }
            }
        }

        newest.map(|(path, _)| path)
    }

    fn rename_clip(path: PathBuf) -> PathBuf {
        let now = Local::now();
        let new_name = format!("ReplayForge_{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
        let new_path = path.parent().unwrap().join(new_name);

        if path == new_path {
            return path;
        }

        match fs::rename(&path, &new_path) {
            Ok(()) => new_path,
            Err(error) => {
                eprintln!("Failed to rename clip: {error}");
                path
            }
        }
    }

    fn generate_thumbnail(path: &PathBuf) {
        let thumbnail = path.with_extension("png");
        let input = path.to_string_lossy();
        let output = thumbnail.to_string_lossy();

        let result = host_command(
            "ffmpeg",
            &[
                "-y",
                "-ss",
                "0",
                "-i",
                &input,
                "-frames:v",
                "1",
                "-update",
                "1",
                &output,
            ],
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

        match result {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("Thumbnail generation failed: {status}"),
            Err(error) => eprintln!("Failed to run ffmpeg: {error}"),
        }
    }

    pub fn restart(&mut self, config: &Config, backend: ResolvedBackend) -> Result<(), String> {
        if self.is_running() {
            self.stop()?;
            thread::sleep(Duration::from_millis(300));
        }
        self.start(config, backend)
    }
}

/// Soft-check that a binary exists for messaging only.
#[allow(dead_code)]
pub fn command_exists(name: &str) -> bool {
    host_output("which", &[name])
        .map(|o| o.status.success())
        .unwrap_or_else(|_| {
            Command::new("which")
                .arg(name)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
}
