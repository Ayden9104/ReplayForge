use crate::config::Config;
use crate::detect::{FLATPAK_GSR_ID, ResolvedBackend};
use crate::host::{host_command, host_output};
use chrono::Local;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub struct Recorder {
    process: Option<Child>,
    backend: Option<ResolvedBackend>,
    display: String,
    last_error: Option<String>,
    /// Lines from GSR stdout (saved clip paths are printed here).
    stdout_rx: Option<Receiver<String>>,
    /// True while we expect GSR to stay alive (set on start, cleared on stop).
    expect_running: bool,
    /// True after we observed the process exit unexpectedly.
    crashed: bool,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            process: None,
            backend: None,
            display: String::new(),
            last_error: None,
            stdout_rx: None,
            expect_running: false,
            crashed: false,
        }
    }
}

impl Recorder {
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn take_crash_notice(&mut self) -> Option<String> {
        if self.crashed {
            self.crashed = false;
            let display = if self.display.is_empty() {
                "your display".to_string()
            } else {
                self.display.clone()
            };
            Some(format!(
                "Replay buffer crashed or exited unexpectedly (display: {display}). Check Settings, then start again."
            ))
        } else {
            None
        }
    }

    pub fn start(&mut self, config: &Config, backend: ResolvedBackend) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }

        if config.output_dir.as_os_str().is_empty() {
            let msg = "Output folder is not set. Pick one in Settings.".to_string();
            self.last_error = Some(msg.clone());
            return Err(msg);
        }

        config.ensure_output_dir().map_err(|e| {
            self.last_error = Some(e.clone());
            e
        })?;

        // Avoid stacking multiple GSR instances.
        let _ = signal_gsr("-INT");

        let fps = config.fps.to_string();
        let buffer = config.buffer_seconds.to_string();
        let output = config.output_dir.to_string_lossy().to_string();
        let gsr_args = build_gsr_args(config, &fps, &buffer, &output);
        let gsr_arg_refs: Vec<&str> = gsr_args.iter().map(String::as_str).collect();

        let mut cmd = match backend {
            ResolvedBackend::Host => host_command("gpu-screen-recorder", &gsr_arg_refs),
            ResolvedBackend::Flatpak => {
                let mut flatpak_args = vec!["run", "--command=gpu-screen-recorder", FLATPAK_GSR_ID];
                flatpak_args.extend(gsr_arg_refs.iter().copied());
                host_command("flatpak", &flatpak_args)
            }
        };

        // Inherit stdout so GSR can print saved paths without pipe buffering issues.
        // Keep stderr piped so we can diagnose immediate failures, then drain it.
        cmd.stdout(Stdio::inherit()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            let msg =
                format!("Failed to launch gpu-screen-recorder: {e}. Is it installed and runnable?");
            self.last_error = Some(msg.clone());
            msg
        })?;

        let stderr = child.stderr.take();

        // Give GSR a moment; if it exits, read stderr for a useful message.
        thread::sleep(Duration::from_millis(600));
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr_text = stderr
                    .map(|s| {
                        let mut buf = String::new();
                        let mut reader = BufReader::new(s);
                        let _ = reader.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                let msg = diagnose_start_failure(
                    &config.display,
                    &config.codec,
                    &config.output_dir,
                    status.code(),
                    &stderr_text,
                );
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
            Ok(None) => {}
            Err(error) => {
                let msg = format!("Failed to check recorder process: {error}");
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
        }

        if let Some(stderr) = stderr {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    eprintln!("[gsr] {line}");
                }
            });
        }

        self.process = Some(child);
        self.backend = Some(backend);
        self.display = config.display.clone();
        self.stdout_rx = None;
        self.last_error = None;
        self.expect_running = true;
        self.crashed = false;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if !self.expect_running && self.process.is_none() {
            return Ok(());
        }

        match self.backend {
            Some(ResolvedBackend::Flatpak) => {
                let _ = host_command("flatpak", &["kill", FLATPAK_GSR_ID]).status();
                let _ = signal_gsr("-INT");
            }
            _ => {
                let _ = signal_gsr("-INT");
            }
        }

        if let Some(mut child) = self.process.take() {
            thread::sleep(Duration::from_millis(250));
            match child.try_wait() {
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                _ => {}
            }
        }

        self.process = None;
        self.stdout_rx = None;
        self.backend = None;
        self.expect_running = false;
        self.crashed = false;
        self.last_error = None;
        Ok(())
    }

    /// Save the replay buffer into `output_dir` (always pass config.output_dir).
    pub fn save_clip(&mut self, output_dir: &Path) -> Result<PathBuf, String> {
        if !self.is_running() {
            let msg = if self.crashed {
                "Cannot save clip: replay buffer crashed. Start it again from Home.".to_string()
            } else {
                "Cannot save clip: replay is not running. Press Start Replay first.".to_string()
            };
            self.last_error = Some(msg.clone());
            return Err(msg);
        }

        if output_dir.as_os_str().is_empty() {
            let msg = "Output folder is not set. Pick one in Settings.".to_string();
            self.last_error = Some(msg.clone());
            return Err(msg);
        }

        fs::create_dir_all(output_dir).map_err(|e| {
            let msg = format!("Cannot write to {}: {e}", output_dir.display());
            self.last_error = Some(msg.clone());
            msg
        })?;

        self.drain_stdout();

        let save_started = SystemTime::now();
        let before = list_mp4_snapshot(output_dir);

        // Signal GSR the same way the official scripts do.
        let signaled = signal_gsr_save();
        if !signaled {
            // If our tracked child is still alive, the broad pkill may have missed —
            // still wait briefly for a file in case a signal landed.
            if !self.child_alive() && !gsr_process_alive() {
                let msg =
                    "Save failed: could not signal gpu-screen-recorder (is replay still running?)"
                        .to_string();
                self.process = None;
                self.expect_running = false;
                self.crashed = true;
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut candidate: Option<PathBuf> = None;

        while Instant::now() < deadline {
            if let Some(path) = self.poll_saved_path_from_stdout() {
                if path.exists() {
                    candidate = Some(path);
                    break;
                }
            }

            if let Some(path) = find_new_or_updated_mp4(output_dir, &before, save_started) {
                candidate = Some(path);
                break;
            }

            thread::sleep(Duration::from_millis(100));
        }

        let clip = candidate.ok_or_else(|| {
            let msg = format!(
                "Save signaled, but no new clip appeared in {} within 10s. \
                 Wait a few seconds after starting replay so the buffer fills, \
                 then try again.",
                output_dir.display()
            );
            self.last_error = Some(msg.clone());
            msg
        })?;

        let stable = wait_until_stable(&clip, Duration::from_secs(5)).map_err(|e| {
            let msg = format!("Clip appeared but did not finish writing: {e}");
            self.last_error = Some(msg.clone());
            msg
        })?;

        // Skip rename if we already use ReplayForge_ naming.
        let renamed = if stable
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("ReplayForge_"))
        {
            stable
        } else {
            Self::rename_clip(stable)?
        };

        if let Err(error) = crate::clips::generate_clip_thumbnail(&renamed) {
            eprintln!("{error}");
        }

        self.last_error = None;
        Ok(renamed)
    }

    /// True only for a session we started. Orphan GSR processes do not count.
    pub fn is_running(&mut self) -> bool {
        if !self.expect_running {
            return false;
        }

        let child_alive = self.child_alive();
        if child_alive {
            return true;
        }

        // Flatpak/host wrappers sometimes exit while GSR keeps running.
        if gsr_process_alive() {
            return true;
        }

        self.expect_running = false;
        self.crashed = true;
        self.backend = None;
        self.stdout_rx = None;
        self.process = None;
        self.last_error =
            Some("Replay buffer exited unexpectedly. Start it again from Home.".into());
        false
    }

    pub fn restart(&mut self, config: &Config, backend: ResolvedBackend) -> Result<(), String> {
        if self.is_running() {
            self.stop()?;
            thread::sleep(Duration::from_millis(300));
        }
        self.start(config, backend)
    }

    fn child_alive(&mut self) -> bool {
        if let Some(process) = self.process.as_mut() {
            match process.try_wait() {
                Ok(Some(_)) => {
                    self.process = None;
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.process = None;
                    false
                }
            }
        } else {
            false
        }
    }

    fn drain_stdout(&mut self) {
        if let Some(rx) = &self.stdout_rx {
            while rx.try_recv().is_ok() {}
        }
    }

    fn poll_saved_path_from_stdout(&mut self) -> Option<PathBuf> {
        let rx = self.stdout_rx.as_ref()?;
        loop {
            match rx.try_recv() {
                Ok(line) => {
                    let trimmed = line.trim();
                    // GSR may print absolute paths or "Saved to /path".
                    if let Some(path) = extract_mp4_path(trimmed) {
                        return Some(path);
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.stdout_rx = None;
                    return None;
                }
            }
        }
    }

    fn rename_clip(path: PathBuf) -> Result<PathBuf, String> {
        let now = Local::now();
        let new_name = format!("ReplayForge_{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
        let new_path = path
            .parent()
            .ok_or_else(|| "Clip path has no parent directory".to_string())?
            .join(&new_name);

        if path == new_path {
            return Ok(path);
        }

        let final_path = if new_path.exists() {
            let alt = format!("ReplayForge_{}.mp4", now.format("%Y-%m-%d_%H-%M-%S_%f"));
            new_path.with_file_name(alt)
        } else {
            new_path
        };

        fs::rename(&path, &final_path)
            .map_err(|e| format!("Failed to rename clip to {}: {e}", final_path.display()))?;
        Ok(final_path)
    }
}

fn signal_gsr(signal: &str) -> bool {
    // Prefer the anchored pattern used by GSR's own scripts.
    host_command("pkill", &[signal, "-f", "^gpu-screen-recorder"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_gsr_args(config: &Config, fps: &str, buffer: &str, output: &str) -> Vec<String> {
    let mut args = vec![
        "-w".into(),
        config.display.clone(),
        "-f".into(),
        fps.to_string(),
        "-r".into(),
        buffer.to_string(),
        "-c".into(),
        "mp4".into(),
        "-replay-storage".into(),
        "ram".into(),
        "-k".into(),
        config.codec.clone(),
        "-bm".into(),
        "cbr".into(),
        "-q".into(),
        config.quality.bitrate_kbps(&config.resolution).to_string(),
        "-o".into(),
        output.to_string(),
    ];
    if config.resolution != "native" && !config.resolution.is_empty() {
        args.push("-s".into());
        args.push(config.resolution.clone());
    }
    let mut audio_sources = Vec::new();
    if config.capture_system_audio {
        let use_apps = config.system_audio_mode == crate::config::SystemAudioMode::Apps
            && !config.audio_apps.is_empty();
        if use_apps {
            for name in &config.audio_apps {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                audio_sources.push(format!("app:{trimmed}"));
            }
        }
        if audio_sources.is_empty() {
            // All mode, or Apps mode with nothing selected → full desktop audio.
            audio_sources.push("default_output".into());
        }
    }
    if config.capture_microphone {
        audio_sources.push("default_input".into());
    }
    if !audio_sources.is_empty() {
        args.push("-a".into());
        args.push(audio_sources.join("|"));
    }
    args
}

fn signal_gsr_save() -> bool {
    // Official GSR docs: killall -SIGUSR1 gpu-screen-recorder
    let killall_ok = host_command("killall", &["-SIGUSR1", "gpu-screen-recorder"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if killall_ok {
        return true;
    }
    signal_gsr("-SIGUSR1")
}

fn gsr_process_alive() -> bool {
    // Use exact-ish process name match first to avoid matching our own shell/commands.
    if host_output("pgrep", &["-x", "gpu-screen-recorder"])
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    host_output("pgrep", &["-f", "^gpu-screen-recorder"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn extract_mp4_path(line: &str) -> Option<PathBuf> {
    if line.ends_with(".mp4") {
        let path = PathBuf::from(line);
        if path.is_absolute() {
            return Some(path);
        }
    }
    for token in line.split_whitespace() {
        if token.ends_with(".mp4") {
            let path = PathBuf::from(token);
            if path.is_absolute() || path.exists() {
                return Some(path);
            }
        }
    }
    None
}

fn diagnose_start_failure(
    display: &str,
    codec: &str,
    output_dir: &Path,
    code: Option<i32>,
    stderr: &str,
) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("no such file")
        || (lower.contains("cannot find") && lower.contains("display"))
    {
        return format!(
            "Could not capture display '{display}'. Open Settings and pick a valid monitor."
        );
    }
    if lower.contains("permission") || lower.contains("portal") {
        return "Screen capture permission was denied. Allow screen sharing when prompted, then try again."
            .into();
    }
    if lower.contains("codec") || lower.contains("encoder") || lower.contains("nvenc") {
        return format!(
            "Codec '{codec}' failed to start. Try h264 in Settings, or check GPU driver support."
        );
    }
    if lower.contains("no space") || lower.contains("read-only") {
        return format!(
            "Cannot write clips to {}. Pick another output folder in Settings.",
            output_dir.display()
        );
    }
    if !stderr.trim().is_empty() {
        let short = stderr.lines().next().unwrap_or(stderr).trim();
        return format!(
            "gpu-screen-recorder exited{}: {short}",
            code.map(|c| format!(" (code {c})")).unwrap_or_default()
        );
    }
    format!(
        "gpu-screen-recorder exited immediately{}. Check display '{display}', codec '{codec}', and that the output folder exists.",
        code.map(|c| format!(" (code {c})")).unwrap_or_default()
    )
}

#[derive(Clone)]
struct Mp4Snapshot {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
}

fn list_mp4_snapshot(folder: &Path) -> Vec<Mp4Snapshot> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                out.push(Mp4Snapshot {
                    path,
                    modified,
                    len: meta.len(),
                });
            }
        }
    }
    out
}

fn find_new_or_updated_mp4(
    folder: &Path,
    before: &[Mp4Snapshot],
    save_started: SystemTime,
) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, SystemTime)> = None;

    for entry in list_mp4_snapshot(folder) {
        let was_known = before.iter().any(|old| {
            old.path == entry.path && old.modified == entry.modified && old.len == entry.len
        });
        if was_known {
            continue;
        }

        // Accept brand-new paths, or existing paths that grew/changed after save.
        let is_new_path = !before.iter().any(|old| old.path == entry.path);
        let touched_after_save = entry
            .modified
            .duration_since(
                save_started
                    .checked_sub(Duration::from_secs(2))
                    .unwrap_or(save_started),
            )
            .is_ok();

        if is_new_path || touched_after_save {
            match &best {
                Some((_, t)) if entry.modified <= *t => {}
                _ => best = Some((entry.path, entry.modified)),
            }
        }
    }

    best.map(|(p, _)| p)
}

fn wait_until_stable(path: &Path, timeout: Duration) -> Result<PathBuf, String> {
    let deadline = Instant::now() + timeout;
    let mut last_size = None;
    let mut stable_hits = 0u32;

    while Instant::now() < deadline {
        let meta =
            fs::metadata(path).map_err(|e| format!("Cannot stat {}: {e}", path.display()))?;
        let size = meta.len();
        if size == 0 {
            stable_hits = 0;
        } else if last_size == Some(size) {
            stable_hits += 1;
            if stable_hits >= 3 {
                return Ok(path.to_path_buf());
            }
        } else {
            stable_hits = 0;
            last_size = Some(size);
        }
        thread::sleep(Duration::from_millis(150));
    }

    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > 0 {
        Ok(path.to_path_buf())
    } else {
        Err(format!("{} stayed empty/unstable", path.display()))
    }
}
