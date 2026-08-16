#!/usr/bin/env bash
set -euo pipefail

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

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <replayforge-*-linux-x86_64.tar.gz|extracted-dir>" >&2
  echo "       $0 --uninstall" >&2
  echo "PREFIX=$PREFIX (override with PREFIX=...)" >&2
  exit 1
fi

SRC="$1"
TMP=""
cleanup() {
  if [[ -n "$TMP" && -d "$TMP" ]]; then
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

if [[ -f "$SRC" && "$SRC" == *.tar.gz ]]; then
  TMP="$(mktemp -d)"
  tar --no-same-owner -xzf "$SRC" -C "$TMP"
  SRC="$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)"
  if [[ -z "$SRC" ]]; then
    echo "error: archive did not contain a package directory" >&2
    exit 1
  fi
elif [[ -d "$SRC" ]]; then
  :
else
  echo "error: expected a .tar.gz or an extracted package directory: $1" >&2
  exit 1
fi

if [[ ! -f "$SRC/replayforge" ]]; then
  echo "error: missing binary at $SRC/replayforge" >&2
  exit 1
fi
if [[ ! -f "$SRC/replayforge.desktop" || ! -f "$SRC/replayforge.svg" ]]; then
  echo "error: package is missing desktop entry or icon" >&2
  exit 1
fi

install -Dm755 "$SRC/replayforge" "$BIN"
install -Dm644 "$SRC/replayforge.desktop" "$DESKTOP"
install -Dm644 "$SRC/replayforge.svg" "$ICON"
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
echo "Uninstall: $0 --uninstall"
