# Devbox Roadmap

Implementation roadmap based on the current repository and the goals in `AGENT.MD`. Unchecked items are not considered complete.

## Current status

The following foundation is already available:

- [x] Native Rust and iced desktop application with light and dark themes.
- [x] Bundled Lucide SVG icons validated during the build.
- [x] Linux discovery for supported runtimes and tools, including version probing.
- [x] Custom executable registration and SQLite persistence for binaries.
- [x] Registration of existing project directories and per-project runtime selection.
- [x] Project process start/stop and bounded stdout/stderr capture.
- [x] Framework hints detected from fixed project manifests.
- [x] Read-only discovery of host services.

Devbox retains binary probe diagnostics and supports project restart, searchable stream-filtered logs, exit details, uptime, Linux CPU/RAM samples, isolated environment profiles, PHP extension inspection, native managed-service profiles, workspaces, Caddy local-domain configuration, and a manifest-only extension registry. Host service discovery remains read-only. See phase notes below for current boundaries.

## Phase 1 — Stabilize the foundation

**Goal:** Make binary and project discovery diagnosable and reliable across host environments.

- [x] Show the reason for failed version probes, including when an executable is found but its version cannot be read.
- [x] Investigate failed runtime probes on supported test hosts, including `pnpm`, and document environment-specific results.
- [x] Test failed binary registration, binary removal, symlinks, file permissions, and persistence across application restarts.
- [x] Ensure operations on deleted projects or binaries return clear errors.
- [x] Add UI flow tests for navigation, forms, busy states, and error messages.
- [x] Verify incremental database upgrades from every supported legacy schema.

Host probe note (2026-11-08): on Arch Linux rolling, `/usr/bin/pnpm` resolves to `/usr/lib/node_modules/pnpm/pnpm`; `pnpm --version` returned `12.16.1`. The failed probe was not reproduced on this host. This does not establish behavior on other distributions.

**Exit criteria:** Discovery failures are understandable to users, existing data remains readable, and the relevant core and UI flow tests pass.

## Phase 2 — Process management and observability

**Goal:** Add the process controls and information needed for everyday use.

- [x] Add restart support that safely stops the existing process before starting it again.
- [x] Add log search and filtering, stdout/stderr separation, and display of exit codes and error details.
- [x] Display process uptime and start time.
- [x] Design CPU and memory monitoring using platform-specific adapters; polling must not block the UI.
- [x] Persist process status transitions and reconcile status after application startup.
- [x] Test restarts, crashes, missing processes, large logs, and multiple active projects.

**Exit criteria:** Process status remains accurate through start, stop, restart, crashes, and application restarts; log viewing and metrics stay responsive.

## Phase 3 — Runtime settings and project environments

**Goal:** Manage runtime and environment configuration without changing global host settings.

- [x] Add a Settings page and persist settings through compatible database migrations.
- [x] Add user-defined binary discovery directories and an option for automatic rescanning.
- [x] Add per-project runtime and environment profiles, with validation and secret redaction in logs.
- [x] Inspect PHP extensions and display results for each PHP executable.
- [x] Expand framework detection incrementally using known, size-limited manifests.
- [x] Test configuration isolation between projects with different runtimes and environments.

**Exit criteria:** Settings persist across restarts, and one project's configuration does not affect another project or the system configuration.

## Phase 4 — Native service lifecycle

**Goal:** Safely manage user-selected local services without containers or root privileges.

- [x] Define generic profiles for database, cache, Mailpit/email, object storage, web server, and other user-selected service executables.
- [x] Distinguish services managed by Devbox from external services that are only detected.
- [x] Implement start/stop/restart only for Devbox-owned processes; never stop external services.
- [x] Validate executables, existing current-user-owned data directories, unprivileged ports, and argument bounds before starting a service.
- [x] Add TCP readiness checks and port conflict diagnostics; show unknown ownership for detected host services.
- [x] Add UI for service status, configuration, bounded logs, and lifecycle actions.
- [x] Test with executable fixtures rather than relying on host daemons being available.

**Exit criteria:** Managed services can be started and stopped by the current user, while external services remain untouched.

Service profiles are generic and do not initialize database data directories or install server software. Health checks use configured TCP ports; without a configured port, only process liveness is known.

## Phase 5 — Workspaces and multi-project orchestration

**Goal:** Run related projects together as a workspace.

- [x] Add workspace and project membership models through database migrations.
- [x] Add project dependencies and cycle-checked start/stop order.
- [x] Display aggregated workspace status, processes, and recent logs.
- [x] Connect workspaces to shared services with in-process consumer tracking; only stop a managed service started by the workspace runner after its last active workspace stops.
- [x] Handle partial startup failures and stop newly started processes for recovery.
- [x] Test two runtimes, multiple processes, failed dependencies, and shared services.

**Exit criteria:** Workspaces start and stop consistently, and ownership of shared services remains correct.

## Phase 6 — Local domains and proxy

**Goal:** Provide local project domains through a proxy installed by the user.

- [x] Detect Caddy/Nginx without assuming Devbox owns or manages them.
- [x] Add per-project `.localhost` domain and loopback upstream configuration, including duplicate-domain and port-range validation.
- [ ] Detect HTTP listener conflicts with externally managed proxy owners before reload; Devbox leaves reload to the user's service manager.
- [ ] Validate proxy configuration before reloading it. Devbox validates before writing and deliberately leaves Caddy reload to the user's service manager.
- [x] Generate HTTP and WebSocket-ready Caddy routes; document manual HTTPS and certificate guidance.
- [x] Never modify the system hosts file or trust store automatically.
- [x] Test configuration generation, conflicts, validation failures, and backup restoration.

**Exit criteria:** Proxy configuration can be applied and restored without changing host configuration outside locations approved by the user.

Proxy configuration generation currently targets Caddy. Nginx is discovered read-only. HTTPS is left to a user-managed Caddy setup.

## Phase 7 — Extension system

**Goal:** Define and manage Devbox extensions without silently running untrusted code.

- [x] Define a size-bounded JSON extension manifest, capabilities, API version, and SQLite storage.
- [x] Define capability approvals and require explicit approval before an extension can be enabled.
- [x] Implement extension listing, validation, enable/disable, and removal.
- [x] Test invalid manifests, incompatible API versions, and denied permissions.

**Exit criteria:** Extensions can be discovered and managed with documented capability limits and explicit approval.

## Phase 8 — UX, accessibility, and distribution

**Goal:** Prepare the application for daily use and Linux distribution.

- [x] Complete Workspace, Settings, and extension management screens with working actions.
- [ ] Add keyboard navigation, focus indicators, accessible labels, and resizing tests at the minimum window size. Page shortcuts and text-field tab traversal are present; keyboard activation of buttons, screen-reader labels, and manual resize checks remain.
- [ ] Review light/dark theme consistency, loading states, error messages, and SVG icon usage in a live GUI session.
- [x] Prepare Linux tarball, Arch PKGBUILD, Debian package script, and RPM spec; document native dependencies.
- [x] Isolate process-group and metrics OS adapters so additional platforms have a defined extension point.
- [x] Add a launch smoke test on the current Linux desktop and record graphics backend limitations.

**Exit criteria:** Main workflows work with keyboard and mouse, the window remains responsive at supported sizes, and the Linux package installs and launches.

## Verification gate for every phase

- [x] `cargo fmt --all --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `cargo build --workspace`
- [ ] Manually test the UI and relevant Linux scenarios; record anything that cannot be tested automatically.
- [x] Update the README only for behavior that has been implemented and verified.

Manual UI interaction, screen-reader labels/focus review, minimum-size layout review, and live graphics-backend visual checks remain outstanding. A release binary launched for eight seconds on the current Arch Linux Wayland session (AMD Radeon integrated graphics, RADV/Mesa 26.2.6); it stayed alive, emitted `VK_EXT_physical_device_drm` unavailable and Wayland GLES context reinitialization warnings, and the compositor denied the screenshot protocol. Core tests cover migration, managed service fixtures and port conflicts, workspace startup/recovery/shared services, extension permissions, proxy validation and restoration, process behavior, and construction of each UI page; the current host's `pnpm` probe result is recorded in Phase 1.

## Scope boundaries

This roadmap does not include Windows/macOS support, system-wide service management, automatic changes to the hosts file or trust store, or running extensions without approval in the current milestone.
