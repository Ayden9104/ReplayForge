#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH="linux-x86_64"
NAME="replayforge-${VERSION}-${ARCH}"
STAGE="$ROOT/dist/$NAME"
OUT="$ROOT/dist/${NAME}.tar.gz"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found" >&2
  exit 1
fi

echo "Building ReplayForge ${VERSION} (release, locked)..."
# Honor CARGO_TARGET_DIR when set (e.g. CI/sandbox caches); otherwise use ./target.
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
# If CARGO_TARGET_DIR is a relative path, resolve from cwd after cargo runs.
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  TARGET_DIR="$CARGO_TARGET_DIR"
  [[ "$TARGET_DIR" = /* ]] || TARGET_DIR="$(pwd)/$TARGET_DIR"
else
  TARGET_DIR="$ROOT/target"
fi
BIN="$TARGET_DIR/release/replayforge"
if [[ ! -x "$BIN" ]]; then
  echo "error: expected release binary at $BIN" >&2
  exit 1
fi

rm -rf "$STAGE"
mkdir -p "$STAGE"

install -Dm755 "$BIN" "$STAGE/replayforge"
install -Dm644 "$ROOT/assets/replayforge.desktop" "$STAGE/replayforge.desktop"
install -Dm644 "$ROOT/assets/replayforge.svg" "$STAGE/replayforge.svg"
# Standalone installer for people who only download the tarball.
install -Dm755 "$ROOT/scripts/install-from-dir.sh" "$STAGE/install.sh"

cat >"$STAGE/README-runtime.txt" <<EOF
ReplayForge ${VERSION} (${ARCH})

Runtime needs:
  - gpu-screen-recorder (host binary or Flatpak com.dec05eba.gpu_screen_recorder)
  - ffmpeg and ffprobe
  - curl (for Share links)

Install:
  tar -xzf ${NAME}.tar.gz
  cd ${NAME}
  ./install.sh

Or from a clone of the repo:
  ./scripts/install-bin.sh /path/to/${NAME}.tar.gz

https://github.com/Ayden9104/ReplayForge
EOF

mkdir -p "$ROOT/dist"
tar -C "$ROOT/dist" -czf "$OUT" "$NAME"

echo "Wrote $OUT"
ls -lh "$OUT"
