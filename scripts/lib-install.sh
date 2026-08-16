# Shared install helpers for ReplayForge scripts.
# Sourced by install.sh / install-from-dir.sh / install-bin.sh

replayforge_parse_force() {
  FORCE=0
  local filtered=()
  for arg in "$@"; do
    case "$arg" in
      --force) FORCE=1 ;;
      *) filtered+=("$arg") ;;
    esac
  done
  REPLAYFORGE_ARGS=("${filtered[@]}")
}

replayforge_require_safe_prefix() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    echo "error: refusing to install as root. Run without sudo (installs to ~/.local)." >&2
    exit 1
  fi
  if [[ "$PREFIX" == *..* ]]; then
    echo "error: PREFIX must not contain '..': $PREFIX" >&2
    exit 1
  fi
}

replayforge_require_overwrite_ok() {
  local bin="$1"
  if [[ -e "$bin" && "${FORCE:-0}" -ne 1 ]]; then
    echo "error: $bin already exists. Re-run with --force to overwrite." >&2
    exit 1
  fi
}

replayforge_print_path_hint() {
  local prefix_bin="$1"
  if [[ ":$PATH:" != *":$prefix_bin:"* ]]; then
    echo "Note: $prefix_bin is not on your PATH."
    echo "  Add this to ~/.bashrc (or equivalent), then open a new terminal:"
    echo "    export PATH=\"$prefix_bin:\$PATH\""
    echo "  Or launch from your app menu after logging out/in."
  else
    echo "Launch with: replayforge"
    echo "Or find ReplayForge in your app menu (log out/in if it is missing)."
  fi
}

# Install an executable via temp + mv so replacing a running binary avoids ETXTBSY.
replayforge_install_bin() {
  local src="$1"
  local dest="$2"
  local dir tmp
  dir="$(dirname "$dest")"
  mkdir -p "$dir"
  tmp="$dir/.$(basename "$dest").new"
  rm -f "$tmp"
  install -m755 "$src" "$tmp"
  mv -f "$tmp" "$dest"
}
