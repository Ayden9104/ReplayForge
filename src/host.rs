use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// True when running inside a container that needs `flatpak-spawn --host`.
pub fn needs_host_spawn() -> bool {
    which_exists("flatpak-spawn")
        && (Path::new("/run/.toolboxenv").exists()
            || std::env::var_os("container").is_some()
            || std::env::var_os("DISTROBOX_ENTER_PATH").is_some())
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

/// Build a command that runs on the host when needed (toolbox/distrobox).
pub fn host_command(program: &str, args: &[&str]) -> Command {
    if needs_host_spawn() {
        let mut cmd = Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program).args(args);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

pub fn host_output(program: &str, args: &[&str]) -> std::io::Result<Output> {
    host_command(program, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

pub fn open_path(path: &Path) {
    let path_str = path.to_string_lossy();
    if let Err(error) = host_command("xdg-open", &[&path_str]).spawn() {
        eprintln!("Failed to open {}: {error}", path.display());
    }
}

/// Best-effort desktop notification (notify-send). Never fails the caller.
pub fn notify_desktop(summary: &str, body: &str) {
    let _ = host_command(
        "notify-send",
        &["--app-name=ReplayForge", "-i", "replayforge", summary, body],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn();
}

pub fn default_videos_dir() -> PathBuf {
    if let Some(dirs) = directories::UserDirs::new() {
        if let Some(videos) = dirs.video_dir() {
            return videos.join("ReplayForge");
        }
        return dirs.home_dir().join("Videos").join("ReplayForge");
    }
    PathBuf::from("ReplayForge")
}
