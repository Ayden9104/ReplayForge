# ReplayForge

Bare-bones Medal-like **instant replay** for Linux.

Start a rolling replay buffer with [GPU Screen Recorder](https://git.dec05eba.com/gpu-screen-recorder/), press a hotkey to save the last N seconds, and browse clips in a simple desktop app.

## Features

- Instant replay start/stop
- Global save hotkey (X11 / XWayland)
- Clips library with thumbnails, open, rename, delete
- Settings: display, FPS, buffer length, codec, output folder, backend
- System tray (show/hide, save, quit)
- Autostart + minimize-to-tray
- First-run setup wizard
- Config at `~/.config/replayforge/config.toml`

## Requirements

- Linux (X11 recommended for global hotkeys; Wayland hotkeys are limited)
- [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/) **or** Flatpak `com.dec05eba.gpu_screen_recorder`
- `ffmpeg` / `ffprobe` (thumbnails + duration)
- Rust toolchain to build from source

### Optional / environment notes

- Running inside Toolbox/Distrobox is supported via `flatpak-spawn --host`
- Folder picker uses the XDG desktop portal (`rfd`)

## Build

```bash
cargo build --release
```

Binary: `target/release/replayforge`

## Run

```bash
cargo run --release
```

On first launch, pick your display and clips folder, then start replay from **Home**.

Default save hotkey: **F8**

## Install (user-local)

```bash
./scripts/install.sh
```

This installs:

- `~/.local/bin/replayforge`
- `~/.local/share/applications/replayforge.desktop`
- `~/.local/share/icons/hicolor/scalable/apps/replayforge.svg`

Then log out/in (or refresh your app menu) and launch **ReplayForge**.

## Flatpak (optional packaging)

A starter manifest lives at [`packaging/flatpak/com.replayforge.ReplayForge.yml`](packaging/flatpak/com.replayforge.ReplayForge.yml). Building a fully sandboxed Flatpak that can drive host GSR needs extra permissions and is best treated as a follow-up release step.

## Config

Example `config.toml`:

```toml
output_dir = "/home/you/Videos/ReplayForge"
display = "DP-1"
fps = 60
buffer_seconds = 60
codec = "h264"
hotkey = "F8"
backend = "auto"          # auto | host | flatpak
autostart = false
minimize_to_tray = true
first_run_complete = true
```

## Hotkeys & Wayland

- The save hotkey always works while the ReplayForge window is focused.
- On **X11**, global hotkeys use `global-hotkey`.
- On **Wayland**, global hotkeys use `evdev` when your user can read `/dev/input` (typically: `sudo usermod -aG input $USER`, then re-login).
- Without input-group access on Wayland, use the in-app Save button or focus ReplayForge and press the hotkey.

## License

MIT
