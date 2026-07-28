use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::SystemTime;

#[derive(Default)]
pub struct Recorder {
    process: Option<Child>,
}

impl Recorder {
    pub fn start(&mut self) {
        if self.process.is_some() {
            return;
        }

        let child = Command::new("flatpak-spawn")
            .args([
                "--host",
                "flatpak",
                "run",
                "--command=gpu-screen-recorder",
                "com.dec05eba.gpu_screen_recorder",
                "-w",
                "DP-3",
                "-f",
                "60",
                "-r",
                "60",
                "-c",
                "mp4",
                "-replay-storage",
                "ram",
                "-k",
                "h264",
                "-o",
                "/var/home/ayden9104/Videos/ReplayForge",
            ])
            .spawn();

        match child {
            Ok(process) => {
                self.process = Some(process);
                println!("Replay buffer started");
            }
            Err(error) => {
                eprintln!("Failed to start replay buffer: {error}");
            }
        }
    }

    pub fn stop(&mut self) {
        if self.process.is_none() {
            return;
        }

        let result = Command::new("flatpak-spawn")
            .args([
                "--host",
                "flatpak",
                "kill",
                "com.dec05eba.gpu_screen_recorder",
            ])
            .status();

        match result {
            Ok(status) if status.success() => {
                self.process = None;
                println!("Replay buffer stopped");
            }
            Ok(status) => {
                eprintln!("Failed to stop replay buffer: {status}");
            }
            Err(error) => {
                eprintln!("Failed to run stop command: {error}");
            }
        }
    }
    pub fn save_clip(&self) {
        if self.process.is_none() {
            eprintln!("Cannot save clip: replay buffer is not running");
            return;
        }

        let result = Command::new("flatpak-spawn")
            .args(["--host", "pkill", "-SIGUSR1", "-f", "^gpu-screen-recorder"])
            .status();

        match result {
            Ok(status) if status.success() => {
                println!("Clip saved");

                if let Some(clip) = Self::find_latest_clip() {
                    println!("Latest clip: {:?}", clip);
                }
            }
            Ok(status) => {
                eprintln!("Failed to save clip: {status}");
            }
            Err(error) => {
                eprintln!("Failed to run save command: {error}");
            }
        }
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
    fn find_latest_clip() -> Option<PathBuf> {
        let folder = PathBuf::from("/var/home/ayden9104/Videos/ReplayForge");

        let mut newest: Option<(PathBuf, SystemTime)> = None;

        if let Ok(entries) = fs::read_dir(folder) {
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
                            _ => {
                                newest = Some((path, modified));
                            }
                        }
                    }
                }
            }
        }

        newest.map(|(path, _)| path)
    }
}
