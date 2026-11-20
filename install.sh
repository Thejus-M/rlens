#!/usr/bin/env bash
# Build rlens and install it for the current user (~/.local).
set -euo pipefail
cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
cargo build --release --locked
install -Dm755 target/release/rlens "$PREFIX/bin/rlens"
install -Dm644 assets/rlens.desktop "$PREFIX/share/applications/rlens.desktop"
echo "Installed $PREFIX/bin/rlens"

missing=()
command -v tesseract >/dev/null || missing+=("tesseract (+ language data)")
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
  command -v wl-copy >/dev/null || missing+=("wl-clipboard (recommended)")
  case "${XDG_CURRENT_DESKTOP,,}" in
    *gnome*|*kde*|*plasma*) ;;
    *) command -v slurp >/dev/null && command -v grim >/dev/null || missing+=("grim + slurp (wlroots compositors)") ;;
  esac
elif [ -n "${DISPLAY:-}" ]; then
  command -v xclip >/dev/null || missing+=("xclip (recommended)")
fi
if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing runtime tools:"
  printf '  - %s\n' "${missing[@]}"
fi
