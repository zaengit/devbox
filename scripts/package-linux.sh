#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project_root/Cargo.toml" | head -n 1)"
architecture="$(uname -m)"
output_dir="$project_root/target/packages"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

cargo build --manifest-path "$project_root/Cargo.toml" --locked --release -p devbox-app
install -Dm755 "$project_root/target/release/devbox-app" "$stage_dir/devbox/bin/devbox"
install -Dm644 "$project_root/packaging/devbox.desktop" "$stage_dir/devbox/share/applications/devbox.desktop"
install -Dm644 "$project_root/assets/icons/boxes.svg" "$stage_dir/devbox/share/icons/hicolor/scalable/apps/devbox.svg"
install -Dm644 "$project_root/README.md" "$stage_dir/devbox/share/doc/devbox/README.md"
install -Dm644 "$project_root/LICENSE" "$stage_dir/devbox/share/doc/devbox/LICENSE"
install -Dm644 "$project_root/assets/icons/LICENSE-lucide.txt" "$stage_dir/devbox/share/doc/devbox/LICENSE-lucide.txt"
mkdir -p "$output_dir"
tar -C "$stage_dir" -czf "$output_dir/devbox-${version}-linux-${architecture}.tar.gz" devbox
printf 'Created %s\n' "$output_dir/devbox-${version}-linux-${architecture}.tar.gz"
printf 'Runtime dependencies: see README.md, Linux packaging section.\n'
