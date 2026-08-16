# ReplayForge

Medal-style **instant replay** for Linux.

It keeps a short rolling buffer with [GPU Screen Recorder](https://git.dec05eba.com/gpu-screen-recorder/). Hit a hotkey to save the last N seconds, then open, trim, or share clips from a simple desktop app.

Latest release: [v0.1.0](https://github.com/Ayden9104/ReplayForge/releases/tag/v0.1.0)

## What you get

- Rolling replay buffer with system + mic audio
- Global save hotkey (works best with the desktop portal on Wayland)
- Clips library: thumbnails, trim, rename, delete, share link
- Optional tray, autostart, and clip-save sound

## Requirements

- Linux (Wayland or X11)
- [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/) **or** Flatpak `com.dec05eba.gpu_screen_recorder`
- `ffmpeg` and `ffprobe`
- `curl` (for Share links)
- Rust toolchain (to build)

## Install

```bash
git clone https://github.com/Ayden9104/ReplayForge.git
cd ReplayForge
./scripts/install.sh
```

That builds a release binary and installs:

- `~/.local/bin/replayforge`
- desktop entry + icon under `~/.local/share/`

Make sure `~/.local/bin` is on your `PATH`, then launch **ReplayForge** from the app menu or a terminal.

```bash
# custom install location
PREFIX=/opt/replayforge ./scripts/install.sh

# remove the app (keeps ~/.config/ReplayForge)
./scripts/install.sh --uninstall
```

If the build fails looking for `alsa` / `alsa.pc`, install ALSA development headers for your distro (e.g. `alsa-lib-devel` on Fedora) and run the install script again. On immutable systems (Bazzite, etc.), building inside a Fedora Distrobox with those headers works well.

## Quick start

1. Pick your display and clips folder on first launch.
2. Start replay from **Home**.
3. On Wayland, open **Settings → Hotkey** and enable **global hotkey (portal)** so save works in games.
4. Press **F8** (default) to save a clip, then check **Clips**.

Share uploads to ReplayForge cloud and copies a link. Clips are about **500 MB** max and expire after roughly **7 days**. You can turn Share off or change the endpoint under **Settings → Sharing**.

## Common gotchas

- **Wayland hotkeys / app audio:** run ReplayForge on the **host** session, not only inside a container, so it can talk to the desktop portal.
- **Tray icon missing:** some desktops need a StatusNotifier / AppIndicator host. Try **Settings → Desktop → Retry tray**, or use **Quit** in the sidebar. Closing the window may only hide the app if minimize-to-tray is on.
- **Flatpak:** there is an experimental manifest under `packaging/flatpak/`, but the supported install path is still `./scripts/install.sh`.

## Config

Settings live in the app. The file is `~/.config/ReplayForge/config.toml` if you want to peek or back it up.

Default save hotkey is **F8**. While the ReplayForge window is focused, the hotkey always works; for in-game saves on Wayland, use the portal option above. X11 can use a normal global hotkey binding; if nothing else works, an advanced `/dev/input` (evdev) fallback exists after adding your user to the `input` group and re-logging in.

## Development

```bash
cargo build --release
cargo run --release
```

Self-hosting the share backend is optional; see [`share-worker/README.md`](share-worker/README.md).

## License

MIT
