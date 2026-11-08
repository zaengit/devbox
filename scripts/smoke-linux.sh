#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$project_root/target/release/devbox-app}"
if [[ ! -x "$binary" ]]; then
  printf 'Build the app first; executable not found: %s\n' "$binary" >&2
  exit 2
fi
if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
  printf 'An interactive DISPLAY or WAYLAND_DISPLAY session is required for this smoke check.\n' >&2
  exit 2
fi
smoke_data="$(mktemp -d "${TMPDIR:-/tmp}/devbox-smoke.XXXXXX")"
set +e
XDG_DATA_HOME="$smoke_data" timeout -s TERM 8s "$binary"
status=$?
set -e
if [[ $status -ne 124 ]]; then
  printf 'Devbox exited before the 8-second smoke window (status %s). Data: %s\n' "$status" "$smoke_data" >&2
  exit 1
fi
printf 'Devbox stayed alive for 8 seconds. Smoke data: %s\n' "$smoke_data"
