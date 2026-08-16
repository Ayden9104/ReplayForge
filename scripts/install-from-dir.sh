#!/usr/bin/env bash
# Install ReplayForge from this extracted package directory (ships inside the release tarball).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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

if [[ ! -f "$HERE/replayforge" ]]; then
  echo "error: run this from the extracted ReplayForge package directory" >&2
  exit 1
fi

install -Dm755 "$HERE/replayforge" "$BIN"
install -Dm644 "$HERE/replayforge.desktop" "$DESKTOP"
install -Dm644 "$HERE/replayforge.svg" "$ICON"
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
else
  echo "Launch with: replayforge"
fi
echo
echo "Uninstall: $0 --uninstall"
