#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project_root/Cargo.toml" | head -n 1)"
architecture="$(dpkg --print-architecture)"
output_dir="$project_root/target/packages"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

cargo build --manifest-path "$project_root/Cargo.toml" --locked --release -p devbox-app
install -Dm755 "$project_root/target/release/devbox-app" "$stage_dir/usr/bin/devbox"
install -Dm644 "$project_root/packaging/devbox.desktop" "$stage_dir/usr/share/applications/devbox.desktop"
install -Dm644 "$project_root/assets/icons/boxes.svg" "$stage_dir/usr/share/icons/hicolor/scalable/apps/devbox.svg"
install -Dm644 "$project_root/README.md" "$stage_dir/usr/share/doc/devbox/README.md"
install -Dm644 "$project_root/LICENSE" "$stage_dir/usr/share/doc/devbox/copyright"
install -Dm644 "$project_root/assets/icons/LICENSE-lucide.txt" "$stage_dir/usr/share/doc/devbox/LICENSE-lucide.txt"
mkdir -p "$stage_dir/DEBIAN" "$output_dir"
cat > "$stage_dir/DEBIAN/control" <<CONTROL
Package: devbox
Version: $version
Section: devel
Priority: optional
Architecture: $architecture
Maintainer: Devbox contributors
Depends: libasound2, libxkbcommon0, libwayland-client0, libx11-6, libxcursor1, libxi6, libxrandr2, libvulkan1, xdg-desktop-portal
Description: Native development environment manager
 Discovers host runtimes and runs projects and explicitly configured services
 as the current user, without containers or elevated privileges.
CONTROL
dpkg-deb --build "$stage_dir" "$output_dir/devbox_${version}_${architecture}.deb"
printf 'Created %s\n' "$output_dir/devbox_${version}_${architecture}.deb"
