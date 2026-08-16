#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/scripts/lib-install.sh"

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

replayforge_parse_force "$@"
set -- "${REPLAYFORGE_ARGS[@]+"${REPLAYFORGE_ARGS[@]}"}"

if [[ "${1:-}" == "--uninstall" ]]; then
  uninstall
  exit 0
fi

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 [--force] <replayforge-*-linux-x86_64.tar.gz|extracted-dir>" >&2
  echo "       $0 --uninstall" >&2
  echo "PREFIX=$PREFIX (override with PREFIX=...)" >&2
  exit 1
fi

replayforge_require_safe_prefix

SRC="$1"
TMP=""
cleanup() {
  if [[ -n "$TMP" && -d "$TMP" ]]; then
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

if [[ -f "$SRC" && "$SRC" == *.tar.gz ]]; then
  # Reject unsafe archive members before extract.
  while IFS= read -r member; do
    [[ -z "$member" ]] && continue
    if [[ "$member" == /* || "$member" == *..* ]]; then
      echo "error: refusing archive member with absolute path or '..': $member" >&2
      exit 1
    fi
  done < <(tar -tzf "$SRC")

  TMP="$(mktemp -d)"
  tar --no-same-owner -xzf "$SRC" -C "$TMP"

  mapfile -t tops < <(find "$TMP" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort)
  if [[ "${#tops[@]}" -ne 1 ]]; then
    echo "error: archive must contain exactly one top-level directory" >&2
    exit 1
  fi
  if [[ ! "${tops[0]}" =~ ^replayforge-.*-linux- ]]; then
    echo "error: unexpected package directory name: ${tops[0]}" >&2
    exit 1
  fi
  SRC="$TMP/${tops[0]}"
elif [[ -d "$SRC" ]]; then
  :
else
  echo "error: expected a .tar.gz or an extracted package directory: $1" >&2
  exit 1
fi

if [[ ! -f "$SRC/replayforge" || -L "$SRC/replayforge" ]]; then
  echo "error: missing regular-file binary at $SRC/replayforge" >&2
  exit 1
fi
if [[ ! -f "$SRC/replayforge.desktop" || ! -f "$SRC/replayforge.svg" ]]; then
  echo "error: package is missing desktop entry or icon" >&2
  exit 1
fi

replayforge_require_overwrite_ok "$BIN"

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
replayforge_print_path_hint "$PREFIX/bin"
echo
echo "Uninstall: $0 --uninstall"
