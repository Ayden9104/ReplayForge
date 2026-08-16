#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

BIN="$PREFIX/bin/replayforge"
DESKTOP="$PREFIX/share/applications/replayforge.desktop"
ICON="$PREFIX/share/icons/hicolor/scalable/apps/replayforge.svg"

uninstall() {
  echo "Uninstalling ReplayForge from $PREFIX..."
  rm -f "$BIN" "$DESKTOP" "$ICON"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
  fi
  echo "Removed binary, desktop entry, and icon (config at ~/.config/ReplayForge is kept)."
}

if [[ "${1:-}" == "--uninstall" ]]; then
  uninstall
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found. Install a Rust toolchain (https://rustup.rs/) first." >&2
  exit 1
fi

echo "Building ReplayForge (release, locked)..."
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"

install -Dm755 "$ROOT/target/release/replayforge" "$BIN"
install -Dm644 "$ROOT/assets/replayforge.desktop" "$DESKTOP"
install -Dm644 "$ROOT/assets/replayforge.svg" "$ICON"

# Point desktop entry at the installed binary when not on PATH yet.
sed -i "s|^Exec=replayforge$|Exec=$BIN|" "$DESKTOP"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

warn_missing() {
  echo "warning: $1" >&2
}

if ! command -v ffmpeg >/dev/null 2>&1; then
  warn_missing "ffmpeg not found (needed for thumbnails / trim). Install ffmpeg."
fi
if ! command -v ffprobe >/dev/null 2>&1; then
  warn_missing "ffprobe not found (needed for clip metadata). Install ffmpeg."
fi
if ! command -v curl >/dev/null 2>&1; then
  warn_missing "curl not found (needed for cloud Share link uploads). Install curl."
fi

if command -v gpu-screen-recorder >/dev/null 2>&1; then
  :
elif command -v flatpak >/dev/null 2>&1 && flatpak info com.dec05eba.gpu_screen_recorder >/dev/null 2>&1; then
  :
else
  warn_missing "gpu-screen-recorder not found (host binary or Flatpak com.dec05eba.gpu_screen_recorder)."
fi

echo
echo "Installed ReplayForge to $PREFIX"
echo "  Binary:  $BIN"
echo "  Desktop: $DESKTOP"
echo
if [[ ":$PATH:" != *":$PREFIX/bin:"* ]]; then
  echo "Note: $PREFIX/bin is not on your PATH."
  echo "  Add this to ~/.bashrc (or equivalent), then open a new terminal:"
  echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
  echo "  Or launch from your app menu after logging out/in."
else
  echo "Launch with: replayforge"
  echo "Or find ReplayForge in your app menu (log out/in if it is missing)."
fi
echo
echo "Post-install:"
echo "  1. Open ReplayForge and finish first-run (display + clips folder)."
echo "  2. Settings → Enable global hotkey (portal) for in-game F8 on Wayland."
echo "  3. Run on the host desktop session (not only inside Toolbox) for portal + audio."
echo
echo "Custom prefix: PREFIX=/opt/replayforge ./scripts/install.sh"
echo "Uninstall:     ./scripts/install.sh --uninstall"
