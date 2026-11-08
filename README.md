# Devbox

Devbox is a native Rust desktop app for discovering host development tools and running projects with an explicitly selected executable. Its UI uses iced and bundled Lucide SVGs. Devbox does not use containers, a web view, or elevated privileges.

## Current implementation

- Light/dark themes, keyboard page shortcuts (`Ctrl/⌘ + 1–9`), `Tab`/`Shift+Tab` text-field traversal, and a resizable window with an 860 × 600 minimum.
- Linux discovery for PHP, Go, Rust, Cargo, Node.js, npm, pnpm, Python, Ruby, Composer, and known infrastructure executables.
- Custom executable registration with explicit version tests, optional regex parsing, and SQLite persistence.
- Existing project registration, per-project runtime selection and environment variables, native process start/stop/restart, bounded logs, exit details, uptime, and Linux CPU/RAM samples.
- Framework detection from a fixed set of size-limited manifests. Detection does not run project scripts.
- Devbox-managed service profiles for user-selected executables, arguments, existing user-owned data directories, and unprivileged TCP health ports. Start, stop, restart, logs, readiness checks, and port conflict diagnostics apply only to process groups Devbox started.
- Read-only discovery for host databases, caches, Mailpit, S3 services, and web servers. Discovered host services show unknown ownership and have no lifecycle controls.
- Workspaces with project membership, prerequisite ordering, dependency-cycle rejection, readiness checks for linked managed services, aggregated process/log status, partial-start recovery, and reference-counted shared managed services.
- Local `.localhost` domains with loopback upstream validation, Caddyfile preview, Caddy validation before writing, a preserved `.devbox-backup`, and a restore action. WebSocket upgrades use Caddy `reverse_proxy` support. Devbox does not reload Caddy, edit the hosts file, or change certificate stores.
- Extension manifest validation (`devbox-extension.json`), capability listing, explicit capability approval, enable/disable, and removal. This version does not execute extension code.
- PHP extension/configuration inspection through a selected PHP executable.
- SQLite schema version 5, with migrations for older databases and user-only database file permissions on Unix.
- A narrow OS adapter for owned process-group shutdown and process metrics. Linux `/proc` metrics are implemented; other platforms currently report metrics and safe process-group shutdown as unsupported.

Managed services use the executable and argument array that the user configured. Devbox does not create database data directories or silently initialize databases. Data directories must already exist and be owned by the current user. Service ports below 1024 are rejected. A missing health port means Devbox can only report process liveness, not protocol readiness.

### Extension manifest

Save a JSON manifest named `devbox-extension.json` and register it from the Extensions page:

```json
{
  "id": "example.readonly",
  "name": "Example extension",
  "version": "1.0.0",
  "api_version": 1,
  "capabilities": ["projects.read", "processes.read"]
}
```

Allowed capability names are `projects.read`, `processes.read`, `processes.start`, `services.read`, `services.manage`, and `storage.write`. Registration starts disabled. Approving capabilities records consent in SQLite; no code is run in this release.

## Run from source

Install Rust/Cargo and the Linux native development libraries for your distribution, then run:

```sh
cargo run -p devbox-app
```

For Arch Linux, the provided PKGBUILD lists the build and runtime packages. Debian/Ubuntu can build a `.deb` with `scripts/package-deb.sh` (requires `dpkg-deb`). Fedora/RHEL packaging metadata is in `packaging/rpm/devbox.spec`. A portable Linux tarball can be created with:

```sh
scripts/package-linux.sh
```

On an interactive desktop session, run the eight-second launch check with:

```sh
scripts/smoke-linux.sh
```

The tarball is written to `target/packages/`. It includes the executable, desktop entry, README, and license. It does not bundle system graphics/audio libraries. Linux runtime dependencies include ALSA, Vulkan loader/driver, X11 or Wayland, XKB common, and an XDG desktop portal implementation for native file dialogs. Package names vary by distribution; examples are `alsa-lib`, `vulkan-icd-loader`, `libxkbcommon`, `wayland`, and the relevant X11 libraries on Arch, or `libasound2`, `libvulkan1`, `libxkbcommon0`, `libwayland-client0`, and X11 client libraries on Debian/Ubuntu.

Devbox stores its database under `$XDG_DATA_HOME/devbox/devbox.sqlite3`, or `~/.local/share/devbox/devbox.sqlite3` when `XDG_DATA_HOME` is unset.

## Verification

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Tests cover migration, service fixture lifecycle and port conflicts, workspace dependency failures and shared service references, extension permissions, proxy generation/validation/backup restore, process behavior, and page view construction. A real GUI interaction, button keyboard activation, screen-reader behavior, and minimum-size layout check still needs a manual pass on the host display session.

## Known limits

- The managed service runner is generic: users configure the server executable, arguments, data directory, and ports themselves. Devbox does not install services or provide vendor-specific database initialization.
- Proxy configuration currently targets Caddy. Nginx is discovered read-only. HTTPS remains a manual Caddy setup step so Devbox does not install or trust certificates.
- Extension records and permissions are managed, but extensions cannot call Devbox APIs or run code yet.
- Only Linux service/process behavior is implemented and tested. Windows and macOS remain outside this milestone.
- Accessibility and window sizing have code-level support, but screen reader behavior, minimum-size layout, and live graphics backend behavior still need manual checks on supported Linux hosts.

## Icon license

Bundled monochrome icons are sourced from the Lucide collection through the Iconify SVG API. Lucide is distributed under the ISC license; see [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt).
