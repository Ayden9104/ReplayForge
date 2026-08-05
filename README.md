# ReplayForge

Bare-bones Medal-like **instant replay** for Linux.

Start a rolling replay buffer with [GPU Screen Recorder](https://git.dec05eba.com/gpu-screen-recorder/), press a hotkey to save the last N seconds, and browse clips in a simple desktop app.

## Features

- Instant replay start/stop
- System + microphone audio (all desktop or selected apps)
- Quality presets (Balanced / High / Ultra CBR)
- Global save hotkey (portal on Wayland; X11 / evdev fallbacks)
- Clips library: thumbnails, open, copy path, rename, full-screen trim with filmstrip timeline, draggable playhead, in-app preview playback, delete, sort/filter
- Desktop notification when a clip saves
- Settings: display, FPS, buffer, codec, quality, audio, output, backend
- System tray (show/hide, save, quit)
- Autostart + minimize-to-tray + optional auto-start replay on launch
- First-run setup (display, folder, audio, portal hotkey)
- Config at `~/.config/replayforge/config.toml`
- Dark UI theme (consistent trim and panel styling)

## Requirements

- Linux (Wayland supported; global hotkeys via desktop portal — see below)
- [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/) **or** Flatpak `com.dec05eba.gpu_screen_recorder`
- `ffmpeg` / `ffprobe` (thumbnails, trim, trim preview video decode)
- Rust toolchain to build from source
- `alsa-lib-devel` when building from source (trim preview audio via rodio)

### Optional / environment notes

- Running inside Toolbox/Distrobox is supported for capture via `flatpak-spawn --host`
- Global Wayland hotkeys work best when ReplayForge runs on the **host** (portal + session bus)
- Folder picker uses the XDG desktop portal (`rfd`)

## Build

```bash
cargo build --release
```

Fedora/Bazzite builders need `alsa-lib-devel` for trim preview audio (rodio/cpal; works with PipeWire via pipewire-alsa).

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

Ensure `~/.local/bin` is on your `PATH` if the menu entry is missing and `replayforge` is not found in a terminal. On Bazzite/Wayland, run from the **host** session for portal hotkeys and app audio.

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
quality = "high"            # balanced | high | ultra (CBR kbps)
hotkey = "F8"
portal_hotkey_enabled = false
capture_system_audio = true
capture_microphone = true
system_audio_mode = "all"   # all | apps
audio_apps = []             # app names when mode is "apps" (GSR app:Name)
backend = "auto"          # auto | host | flatpak
autostart = false
auto_start_replay = false
minimize_to_tray = true
first_run_complete = true
```

Audio is one GSR `-a` track. Default is `default_output|default_input`. In **Settings → Capture**, choose **Selected apps** to record only those apps (PipeWire). App list names are PipeWire clients (Discord often appears as `webrtc voiceengine`).

## Hotkeys & Wayland

- The save hotkey **always** works while the ReplayForge window is focused.
- On **Wayland**, prefer **Settings → Enable global hotkey (portal)** — your desktop shows a normal permission dialog (no sudo).
- On **X11**, in-game/global hotkeys use `global-hotkey` when portal is not active.
- Fallback on Wayland: `evdev` (reads `/dev/input`) if portal is unavailable or not enabled.

### Enable global hotkeys on Wayland (recommended)

1. Open **Settings → Hotkey**
2. Click **Enable global hotkey (portal)** and accept the desktop prompt
3. Status should show `Global hotkey F8 active (portal: …)`
4. Optional: **Configure global hotkey…** to change the binding in the portal UI

`portal_hotkey_enabled` is saved so later launches restore the portal session without prompting on first run (enable is opt-in).

**Toolbox / Distrobox:** run ReplayForge on the **host** so it can talk to the session portal (and `/dev/input` if you use the advanced fallback).

### Advanced: `input` group / evdev

If your desktop portal does not support GlobalShortcuts:

```bash
sudo usermod -aG input $USER
```

Then **log out and back in** (or reboot). Restart ReplayForge — the status bar should say `Global hotkey F8 active (evdev, …)`.

Without portal or `/dev/input` access, use the Save button or focus ReplayForge and press the hotkey. Settings shows the exact reason.

## License

MIT
