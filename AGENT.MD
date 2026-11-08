# Devbox — Native Development Environment Manager

## Complete Implementation Prompt
### Rust + iced + macOS Monterey UI + Iconify SVG + Custom Binary Detection

## Role

Act as a Principal Rust Engineer, Linux Systems Engineer, Native Desktop Application Architect, and Senior UI/UX Designer.

Design and implement **Devbox**, an open-source, lightweight native development environment manager.

Devbox must use Rust for its application core and **iced for its entire desktop interface**, with visual styling inspired by macOS Monterey.

All graphical icons must be real SVG assets sourced from **https://iconify.design/**.

The primary platform is Linux, including Arch Linux, Fedora, Ubuntu, and Debian. Structure platform-specific functionality so Windows and macOS support can be added later.

Devbox must discover and manage existing host runtimes and development services without requiring containers.

### Strict Restrictions

Do not use:

- Docker or Podman.
- Containers or Kubernetes.
- Virtual machines or WSL.
- Tauri or Electron.
- WebViews.
- HTML, CSS, or JavaScript for the GUI.
- Icon fonts as a replacement for SVG.
- Root privileges during normal operation.

Implement functional software, not merely mockups, architecture documents, or placeholder controls.

---

# 1. Technology Stack

| Component | Technology |
|---|---|
| Main language | Rust |
| Desktop GUI | iced |
| Icons | Iconify SVG |
| Asynchronous runtime | Tokio |
| Process execution | tokio::process |
| Binary discovery | Rust std + OS adapters |
| Data persistence | SQLite |
| Configuration | Serde + TOML |
| Filesystem monitoring | notify |
| Logging | tracing |
| Error handling | thiserror / anyhow |
| Process monitoring | Linux-native process inspection |
| Local proxy | Installed Caddy / Nginx |
| Project architecture | Cargo workspace |

Use stable compatible crate versions and verify their APIs.

---

# 2. Main Application Architecture

```text
Devbox
│
├── Native GUI (iced)
│   ├── Monterey Design System
│   ├── Iconify SVG Renderer
│   ├── Dashboard
│   ├── Project Manager
│   ├── Workspace Manager
│   ├── Binary Manager
│   ├── Runtime Manager
│   ├── Service Manager
│   └── Settings
│
└── Rust Core
    ├── System Scanner
    ├── Binary Discovery Engine
    ├── Custom Binary Registry
    ├── Runtime Resolver
    ├── Native Process Manager
    ├── Native Service Manager
    ├── Project Detector
    ├── Microservices Orchestrator
    ├── Extension Manager
    ├── Local Network Manager
    └── SQLite Persistence
```

Keep UI and business logic separate. All filesystem, process, and service operations must be asynchronous or performed without blocking GUI rendering.

---

# 3. macOS Monterey-Inspired UI

Build a modern, polished desktop interface inspired by macOS Monterey's Finder and System Preferences.

### Visual Requirements

- Minimalist desktop layout.
- Rounded surfaces and buttons.
- Light and dark modes.
- Semi-transparent sidebar where technically supported.
- Frosted-glass-inspired surfaces with opaque fallbacks.
- Subtle shadows and separators.
- Compact native-feeling toolbar.
- Segmented controls.
- Clean typography.
- Consistent spacing.
- Keyboard navigation.
- Accessible focus indicators.
- Smooth loading and interaction feedback.

Do not duplicate Apple branding or proprietary assets.

### Light Mode

```text
Background:    #F5F5F7
Sidebar:       #E9E9EF
Surface:       #FFFFFF
Text:          #1D1D1F
Secondary:     #6E6E73
Accent:        #007AFF
Success:       #34C759
Warning:       #FF9500
Error:         #FF3B30
Separator:     #D2D2D7
```

### Dark Mode

```text
Background:    #1C1C1E
Sidebar:       #252528
Surface:       #2C2C2E
Text:          #F5F5F7
Secondary:     #98989D
Accent:        #0A84FF
Success:       #30D158
Warning:       #FF9F0A
Error:         #FF453A
Separator:     #38383A
```

Use centralized design tokens for all components.

Use system fonts with appropriate fallbacks. Do not bundle proprietary Apple fonts.

---

# 4. Icon System — Iconify SVG

**MANDATORY: All application UI icons must use SVG assets sourced from Iconify.**

Official source:

https://iconify.design/

Icon browser:

https://icon-sets.iconify.design/

Iconify SVG API:

https://api.iconify.design/

## 4.1 Primary Icon Collection

Use **Lucide**, available through Iconify, as the default icon collection.

Icon prefix:

`lucide`

Use Tabler Icons (`tabler`) as an optional secondary collection when an equivalent Lucide icon does not exist.

Prefer a single consistent icon style within the same interface.

Do not mix unrelated icon collections arbitrarily.

## 4.2 SVG Integration with iced

Use iced's native SVG widget.

Enable the required iced SVG feature.

Example:

```toml
[dependencies]
iced = { version = "0.14", features = ["svg"] }
```

Verify the selected stable version and its exact API before implementing.

Example Rust:

```rust
use iced::widget::svg;

let icon = svg(
    svg::Handle::from_path(
        "assets/icons/house.svg"
    )
)
.width(20)
.height(20);
```

Support embedded SVG assets through `include_bytes!` and `svg::Handle::from_memory` where suitable.

Do not use browser-based Iconify components.

Do not require JavaScript to display icons.

## 4.3 Offline SVG Assets

Icons required by the application must be available offline.

Use Iconify to obtain SVG assets during development or a controlled asset-generation step.

Example Iconify SVG endpoints:

```text
https://api.iconify.design/lucide/house.svg
https://api.iconify.design/lucide/folder.svg
https://api.iconify.design/lucide/settings.svg
https://api.iconify.design/lucide/database.svg
https://api.iconify.design/lucide/terminal.svg
```

These are asset source endpoints, not runtime dependencies.

Store approved SVG assets locally:

```text
assets/
└── icons/
    ├── navigation/
    ├── actions/
    ├── status/
    ├── runtimes/
    └── services/
```

Do not download icons every time the application starts.

The desktop application must render all bundled icons without internet access.

## 4.4 Reusable Icon Component

Create a centralized Rust icon module.

Suggested API:

```rust
pub enum AppIcon {
    Dashboard,
    Folder,
    Terminal,
    Database,
    Server,
    Settings,
    Search,
    Refresh,
    Plus,
    Trash,
    Play,
    Stop,
    Restart,
    Check,
    Warning,
    Error,
}
```

Implement a reusable icon-rendering component.

Example intended usage:

```rust
icon(AppIcon::Folder, 20)
```

Support:

- Icon identifier.
- Size.
- Theme-aware color.
- Hover color.
- Disabled appearance.
- Active navigation appearance.
- Tooltip or accessible label where needed.

Cache SVG handles to avoid repeatedly loading and parsing the same asset.

## 4.5 SVG Color Handling

Support monochrome icons that adapt to the active application theme.

Do not assume CSS `currentColor` automatically inherits iced text colors.

Use supported iced SVG styling or preprocess monochrome SVG colors explicitly.

Preserve the SVG `viewBox` and aspect ratio.

For multicolor icons, preserve their original colors unless a specific monochrome variant is intended.

Use the following standard sizes:

| Context | Size |
|---|---|
| Compact inline | 14–16 px |
| Sidebar | 18–20 px |
| Toolbar | 18–20 px |
| Buttons | 16–18 px |
| Runtime and service rows | 20–24 px |
| Empty states | 40–48 px |

Maintain consistent optical alignment.

## 4.6 Icon Guidelines

Use Lucide-style icons for:

| Feature | Suggested Iconify ID |
|---|---|
| Dashboard | `lucide:layout-dashboard` |
| Projects | `lucide:folder-code` |
| Workspaces | `lucide:boxes` |
| Runtimes | `lucide:terminal` |
| Binary Manager | `lucide:binary` |
| Databases | `lucide:database` |
| Services | `lucide:server` |
| Extensions | `lucide:puzzle` |
| Settings | `lucide:settings` |
| Search | `lucide:search` |
| Add | `lucide:plus` |
| Refresh | `lucide:refresh-cw` |
| Start | `lucide:play` |
| Stop | `lucide:square` |
| Logs | `lucide:scroll-text` |

Verify every icon ID exists before asset generation.

For language and database identities, use clearly distinguishable icons or labels. Do not use protected brand artwork without checking its license.

## 4.7 SVG Safety and Licensing

- Bundle only necessary icons.
- Verify the license of each Iconify icon collection.
- Preserve required licenses and attribution notices.
- Prefer permissive icon sets.
- Reject malformed SVG assets.
- Exclude scripts, external resource references, and unsupported active content.
- Validate SVG compatibility with iced's rendering backend.
- Do not silently substitute raster graphics when SVG loading fails.

Implement a build-time icon validation step that detects missing, invalid, or duplicated assets.

---

# 5. Navigation Layout

Use a Monterey-inspired sidebar:

```text
Devbox
│
├── Overview
│   ├── Dashboard
│   └── Activity
│
├── Development
│   ├── Projects
│   ├── Workspaces
│   └── Runtimes
│
├── System
│   ├── Binary Manager
│   ├── Databases
│   ├── Cache
│   ├── Storage
│   ├── Email
│   └── Web Servers
│
├── Tools
│   ├── Extensions
│   ├── Processes
│   └── Logs
│
└── Settings
```

All navigation items must use the centralized Iconify SVG component.

Implement:

- Selection indicators.
- Hover feedback.
- Sidebar collapse/expand.
- Keyboard navigation.
- Responsive window resizing.
- Active page titles.

Do not display non-functional navigation items as completed features.

---

# 6. Linux Runtime Detection

Automatically discover installed development binaries.

Supported runtimes:

- PHP.
- Go.
- Rust.
- Cargo.
- Node.js.
- npm.
- pnpm.
- Python.
- Ruby.
- Composer.

Supported infrastructure:

- MySQL.
- MariaDB.
- PostgreSQL.
- MongoDB.
- Redis.
- Valkey.
- Mailpit.
- Garage or another supported S3 server.
- Caddy.
- Nginx.
- Apache.

Search the active environment's PATH and known installation directories.

Support runtime managers such as rustup, mise, and nvm.

Detect versions using appropriate commands.

Differentiate:

- Executable available.
- Executable missing.
- Binary incompatible.
- Service installed but stopped.
- Service running.
- Service unhealthy.
- Access denied.

Never treat the existence of a client executable as proof that a database server is installed.

Do not execute arbitrary discovered programs during filesystem scanning.

---

# 7. Custom Binary Manager

**Custom executable discovery and registration are mandatory.**

Allow users to register binaries installed outside PATH.

Example locations:

```text
/usr/bin/php
/opt/php83/bin/php
/home/user/.local/bin/php
/home/user/sdk/go/bin/go
/home/user/.cargo/bin/rustc
/mnt/tools/custom-runtime/bin/server
```

### Features

- Add custom binary.
- Browse executable path.
- Edit registration.
- Remove registration without deleting the file.
- Add custom search directory.
- Disable/enable directory scanning.
- Manually rescan.
- Detect multiple versions.
- Configure custom version arguments.
- Configure custom version parsers.
- Configure regex extraction.
- Set default runtime.
- Select binary per project.
- Validate executable permissions.
- Detect missing files and broken symlinks.
- Store registrations in SQLite.

### Binary Registration Model

Store:

```text
id
display_name
category
executable_path
resolved_path
version_arguments
version_parser
version_regex
detected_version
manual_version
environment_profile
discovery_source
trust_status
enabled
last_verified_at
```

### Version Detection Security

- Use structured process arguments.
- Do not interpolate executable commands into shell strings.
- Apply process timeouts.
- Limit captured output.
- Do not execute unknown binaries without user approval.
- Do not use sudo.
- Validate paths.
- Avoid unrestricted recursive scanning.

### Binary Selection Priority

1. Explicit project selection.
2. Explicit workspace selection.
3. User-selected default.
4. Automatically detected compatible binary.
5. Missing binary error.

Do not silently replace a missing runtime with an incompatible version.

---

# 8. Binary Manager GUI

Create a macOS System Preferences-inspired binary management screen.

Display:

- Binary icon.
- Display name.
- Version.
- Executable path.
- Discovery source.
- Availability status.
- Associated projects.

Add the following actions:

- Add Binary.
- Scan Directory.
- Rescan All.
- Test Binary.
- Set Default.
- Edit.
- Remove Registration.

Use native iced dialogs and controls.

Include an Add Custom Binary dialog with:

```text
Add Custom Binary

Name:
[ PHP 8.3 Custom ]

Category:
[ Runtime ]

Executable:
[ /opt/php83/bin/php ] [Browse]

Version Arguments:
[ --version ]

Version Parser:
[ PHP ]

[ Test Binary ]

Status: Available
Version: PHP 8.3.x

[ Cancel ] [ Save ]
```

Use Iconify SVG icons in the dialog, buttons, validation messages, and list rows.

The Test Binary action must perform real validation, not simulate success.

---

# 9. Project Manager

Projects can exist in any accessible host directory.

Features:

- Add existing project.
- Create project.
- Detect language/framework.
- Select native runtime binary.
- Configure startup commands.
- Configure environment variables.
- Start.
- Stop.
- Restart.
- View logs.
- Open project folder.
- Open in configured editor.

Detect frameworks through project files.

Examples:

- Laravel.
- Symfony.
- WordPress.
- Nuxt.
- Next.js.
- SvelteKit.
- Django.
- FastAPI.
- Gin.
- Fiber.
- Axum.
- Actix Web.

Do not execute untrusted project scripts during framework detection.

Do not relocate source code.

---

# 10. Native Runtime Execution

Use `tokio::process::Command`.

Run all project workloads under the current Linux user.

Every process must have:

- Executable path.
- Argument array.
- Working directory.
- Environment variables.
- Process identity.
- Start time.
- Exit status.
- stdout/stderr capture.
- Shutdown policy.

Support:

- Start.
- Stop.
- Restart.
- Graceful shutdown.
- Crash detection.
- Resource monitoring.
- Port conflict detection.
- Live log streaming.

Do not terminate unrelated processes.

Track child process groups safely.

Do not rely exclusively on persisted PIDs.

---

# 11. Native Service Management

Support existing system services and separate user-managed instances.

### Existing System Service

Detect through appropriate methods:

- systemd system units.
- systemd user units.
- Process inspection.
- Endpoint connectivity.

Display service status without assuming Devbox owns the process.

Do not automatically stop, restart, or modify system services that require elevated privileges.

### User-Managed Instance

When supported:

- Execute an existing server binary.
- Use user-owned configuration.
- Use user-owned data directories.
- Use unprivileged ports.
- Maintain logs.
- Persist instance configuration.
- Support safe lifecycle operations.

Never overwrite system database directories.

---

# 12. Databases, Cache, S3, and Email

### Databases

Support:

- MySQL.
- MariaDB.
- PostgreSQL.
- MongoDB.

Provide connection profiles, testing, server information, and accessible database management.

### Cache

Detect and connect to Redis and Valkey.

Support optional user-managed instances.

### S3

Support already-installed S3-compatible servers, including Garage when available.

Provide endpoint and bucket configuration.

### Email

Detect Mailpit.

Support:

- SMTP connection settings.
- Native server lifecycle when Devbox-owned.
- Opening the inbox in the default browser.
- Service logs and status.

Use SVG icons for each service category.

---

# 13. Microservices Workspaces

Support multi-directory and multi-repository development.

Example:

```text
Ecommerce
├── Auth API       → Go
├── User API       → PHP
├── Product API    → Rust
├── Frontend       → Node.js
├── MySQL
├── MongoDB
├── Redis
├── Mailpit
└── S3 Storage
```

Required features:

- Create workspace.
- Add projects.
- Configure dependencies.
- Select binaries per service.
- Start all.
- Stop all.
- Restart individual processes.
- Start selected services with dependencies.
- Detect dependency cycles.
- Monitor readiness.
- Aggregate logs.
- Display service relationships.

Support sharing native infrastructure services where explicitly configured.

Devbox must not stop externally managed services when workspace projects exit.

---

# 14. PHP Extension Manager

Use the PHP executable selected by each project.

Support:

- List loaded extensions.
- Detect extension configuration files.
- Inspect required extensions.
- Detect incompatible extensions.
- Configure project-specific PHP settings where supported.
- Identify missing extensions.
- Report installation options.

Examples:

```bash
/opt/php83/bin/php -m
/opt/php83/bin/php --ini
/opt/php83/bin/php --ri redis
```

Never edit system-wide PHP configuration silently.

Never install system packages without authorization.

---

# 15. Local Domains

Support local development URLs:

```text
api.localhost
auth.localhost
shop.localhost
frontend.localhost
```

Detect installed Caddy or Nginx.

Support custom proxy executable paths.

Provide:

- Local domain assignment.
- Reverse proxy.
- HTTP.
- Optional HTTPS.
- WebSockets.
- Port conflict detection.
- Proxy diagnostics.

Avoid privileged ports where possible.

Do not automatically modify system certificate stores or hosts files.

---

# 16. Process Monitoring and Logs

Create functional screens for:

- Running project processes.
- Service status.
- CPU usage.
- Memory usage.
- Process uptime.
- Exit codes.
- stdout/stderr.
- Searchable logs.
- Restart controls.
- Error details.

Use Iconify SVG status and action icons.

Use clear visual states for running, stopped, failed, and unknown processes.

Avoid excessive polling.

Do not block the GUI during monitoring.

---

# 17. Settings

Create a Monterey-inspired Settings screen.

Sections:

- General.
- Appearance.
- Binary Discovery.
- Runtime Defaults.
- Project Directories.
- Native Services.
- Networking.
- Process Monitoring.
- Logs.
- Advanced.

Include user-configurable settings for:

- Light/dark theme.
- Accent color.
- Runtime search directories.
- Custom binary paths.
- Preferred editor.
- Automatic rescanning.
- Default ports.
- Service ownership behavior.
- Log retention.

Persist all applicable settings.

---

# 18. SQLite Persistence

Use database migrations.

Suggested tables:

```text
projects
project_runtimes
workspaces
workspace_members
detected_binaries
custom_binaries
binary_search_directories
binary_project_assignments
runtime_profiles
service_profiles
service_consumers
process_sessions
application_settings
```

Use stable IDs.

Distinguish cached discovery data from authoritative OS state.

Reconcile stored process records on application startup.

Do not remove custom binary registrations simply because a file is temporarily unavailable.

---

# 19. Cargo Workspace Structure

```text
devbox/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── crates/
│   ├── devbox-app/
│   ├── devbox-ui/
│   ├── devbox-icons/
│   ├── devbox-core/
│   ├── devbox-platform/
│   ├── devbox-detector/
│   ├── devbox-binary/
│   ├── devbox-process/
│   ├── devbox-runtimes/
│   ├── devbox-services/
│   ├── devbox-projects/
│   ├── devbox-workspaces/
│   └── devbox-storage/
├── assets/
│   └── icons/
│       ├── navigation/
│       ├── actions/
│       ├── status/
│       ├── runtimes/
│       └── services/
├── scripts/
├── tests/
└── .github/
    └── workflows/
```

The `devbox-icons` crate or module must own SVG asset loading, caching, color treatment, and reusable icon rendering.

Do not duplicate icon-loading code across screens.

---

# 20. Security

Requirements:

- Run as the active user.
- No sudo for normal operation.
- No containers.
- No automatic global package modifications.
- No execution of untrusted binaries without approval.
- No uncontrolled recursive scanning.
- No arbitrary shell interpolation.
- No unrelated process termination.
- No unnecessary host configuration changes.
- Protect local credentials.
- Redact secrets from logs.
- Validate custom SVG assets.
- Preserve source file ownership.

Use secure defaults and clearly explain permission limitations.

---

# 21. Performance

Devbox must remain lightweight.

Requirements:

- Async runtime discovery.
- Async process monitoring.
- Bounded directory scanning.
- Incremental rescans.
- Cached binary information.
- Cached SVG handles.
- Prebundled icon assets.
- Limited concurrent probes.
- Cancellable long-running operations.
- Minimal memory overhead.
- Responsive iced rendering.

Do not require internet access for normal UI rendering.

---

# 22. Testing

Implement automated tests for:

### Binary Discovery

- PATH detection.
- Custom executable paths.
- Multiple binary versions.
- Version parsing.
- Missing executable handling.
- Permission errors.
- Directory scanning limits.
- Symlink resolution.
- Per-project runtime selection.
- SQLite persistence.

### Icons and UI

- Icon manifest integrity.
- Missing SVG detection.
- Invalid SVG detection.
- Icon ID mapping.
- SVG rendering compatibility.
- Light/dark color treatment.
- Icon dimensions and alignment.
- Navigation states.
- Keyboard accessibility.

### Native Processes

- Start/stop/restart.
- Correct executable selection.
- Correct working directory.
- Non-root execution.
- Log capture.
- Crash handling.
- Safe process termination.

### Project Integration

- Two projects using different PHP binaries.
- Multiple simultaneous processes.
- Microservices dependency handling.
- Shared service ownership.
- Data persistence.

Do not report tests as passing unless executed successfully.

---

# 23. Implementation Roadmap

### Phase 1 — Desktop Foundation

- Create Rust Cargo workspace.
- Initialize iced application.
- Implement Monterey design tokens.
- Implement SVG Iconify asset pipeline.
- Build reusable SVG icon components.
- Implement sidebar and toolbar.
- Implement light and dark themes.

### Phase 2 — Binary Detection

- Scan PATH.
- Detect installed runtimes.
- Detect versions.
- Implement Binary Manager GUI.
- Persist detection results.

### Phase 3 — Custom Binaries

- Register arbitrary executable paths.
- Add custom search directories.
- Implement version probes.
- Implement executable validation.
- Add per-project binary overrides.

### Phase 4 — Project Manager

- Register existing projects.
- Detect frameworks.
- Select runtime binaries.
- Start native processes.
- Display logs.

### Phase 5 — Infrastructure

- Database detection.
- Redis/Valkey detection.
- Mailpit detection.
- S3 server detection.
- Native user-managed service instances.

### Phase 6 — Multi-Project and Microservices

- Multiple runtime processes.
- Cross-directory workspaces.
- Dependencies.
- Service sharing.
- Aggregated monitoring.

### Phase 7 — Advanced Features

- PHP extensions.
- Local domains.
- HTTPS.
- Resource management.
- Settings.

### Phase 8 — Stabilization

- Unit tests.
- Integration tests.
- UI testing.
- Accessibility review.
- SVG license audit.
- Performance optimization.
- Linux packaging.
- Documentation.

---

# 24. Acceptance Criteria

Devbox is considered functionally complete when:

1. A native iced GUI launches successfully.
2. The UI has a consistent Monterey-inspired appearance.
3. All application icons use bundled Iconify SVG assets.
4. SVG icons work without internet access.
5. Light and dark themes are supported.
6. Runtime binaries are detected automatically.
7. Custom executable registration works.
8. Multiple runtime versions can coexist.
9. Projects can choose specific binaries.
10. Projects run in their existing host directories.
11. Processes run as the active user.
12. Multiple projects can run simultaneously.
13. Native infrastructure services can be detected.
14. Supported user-managed services can be operated safely.
15. Microservices workspaces function correctly.
16. Runtime logs and process states are visible.
17. User configurations persist across restarts.
18. The GUI remains responsive.
19. Automated tests cover critical functionality.
20. No Docker, Podman, Tauri, Electron, or WebView is required.

Mark incomplete or unverified features explicitly.

---

# 25. Final Implementation Instructions

**Start implementing Devbox immediately.**

Follow these engineering rules:

1. Inspect the existing repository.
2. Preserve user modifications.
3. Verify current iced and Rust APIs.
4. Implement actual functionality instead of mockups.
5. Use Iconify SVG assets for all application icons.
6. Keep GUI and core logic independent.
7. Avoid unnecessary dependencies.
8. Write tests throughout implementation.
9. Run `cargo fmt`.
10. Run `cargo clippy`.
11. Run `cargo test`.
12. Build the desktop application.
13. Fix compilation and test failures.
14. Document actual functionality.
15. Do not claim unverified behavior works.
16. Do not commit or push unless explicitly requested.

### First Functional Milestone

Implement and verify:

- Native iced application window.
- Monterey-inspired sidebar and toolbar.
- Offline Iconify SVG rendering.
- Light and dark themes.
- Linux runtime detection.
- Custom binary registration.
- Custom version detection.
- SQLite persistence.
- Per-project runtime selection.
- Native process execution.
- Live logs and safe shutdown.

The first milestone must demonstrate detecting an installed runtime, adding a custom executable, registering an existing project, running the selected binary as the current user, and stopping it safely.

Then continue with native infrastructure, microservices, extensions, and local domain support.

**Devbox must remain a fully native Rust + iced application with Iconify SVG icons, without containerization, and with project source code kept in the user's original directories.**