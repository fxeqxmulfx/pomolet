#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${WAYLAND_DISPLAY:-}" && -z "${WAYLAND_SOCKET:-}" ]]; then
    printf 'No Wayland session found (WAYLAND_DISPLAY or WAYLAND_SOCKET is missing).\n' >&2
    exit 1
fi

# Without DISPLAY, winit cannot silently fall back to X11/XWayland.
unset DISPLAY

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec cargo run --release --manifest-path "$project_dir/Cargo.toml" -- "$@"
