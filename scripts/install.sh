#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

echo "Building ReplayForge (release)..."
cargo build --release --manifest-path "$ROOT/Cargo.toml"

install -Dm755 "$ROOT/target/release/replayforge" "$PREFIX/bin/replayforge"
install -Dm644 "$ROOT/assets/replayforge.desktop" "$PREFIX/share/applications/replayforge.desktop"
install -Dm644 "$ROOT/assets/replayforge.svg" "$PREFIX/share/icons/hicolor/scalable/apps/replayforge.svg"

# Point desktop entry at the installed binary when not on PATH yet.
sed -i "s|^Exec=replayforge$|Exec=$PREFIX/bin/replayforge|" "$PREFIX/share/applications/replayforge.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo
echo "Installed ReplayForge to $PREFIX"
echo "  Binary:  $PREFIX/bin/replayforge"
echo "  Desktop: $PREFIX/share/applications/replayforge.desktop"
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
echo "On Wayland: Settings → Enable global hotkey (portal) for in-game F8."
echo "Run on the host desktop session (not only inside Toolbox) for portal + audio."
