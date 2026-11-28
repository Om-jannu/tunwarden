# Tunwarden – Desktop App Design (M3)

Status: Draft 1, for review · Date: 27 Nov 2026 · Scope: PRD 7.8 (FR-31 to FR-36), FR-10, FR-25, FR-55

The app is a thin client of the daemon, like the CLI. It holds no tunnel logic: every action is a daemon protocol call (design section 5), and every update comes from `subscribe`. If the app is closed, tunnels keep running.

## 1. Decisions

| # | Decision | Why | Rejected |
| --- | --- | --- | --- |
| D1 | **Tauri 2.12, React, TypeScript, Vite** in `apps/desktop`; `apps/desktop/src-tauri` joins the Cargo workspace | Already decided in the PRD; one workspace keeps `tunwarden-core` shared | Electron, Wails |
| D2 | **The app ships the `tunwarden` CLI as a Tauri sidecar** and starts the daemon with `tunwarden daemon start` | One daemon binary however it was started, so app, CLI and daemon versions match; the CLI is also on PATH after install (FR-55) | The app binary running the daemon itself: two different daemon binaries could fight over one state directory |
| D3 | **The Rust side of the app is a small bridge**: Tauri commands that call `tunwarden_core::client`, and one task that relays `subscribe` events to the window as Tauri events | Same client code as the CLI; the web view never talks to the socket | Calling the CLI and parsing its output |
| D4 | **TypeScript types are generated from `tunwarden_core::proto`** with `ts-rs`, behind a `ts` feature; CI fails if the generated file is stale | The protocol and the UI cannot drift apart | Hand-written types |
| D5 | **UI: Tailwind CSS and shadcn/ui components**, one small store (Zustand) fed by daemon events, no router library (five views) | Small, widely known, no runtime theme system | A full component framework, Redux |
| D6 | **Closing the window hides it to the tray**; Quit asks "Keep tunnels running" (default) or "Stop all tunnels" (FR-10) | Tunnels belong to the daemon, not the window | Quitting on window close |
| D7 | **Re-login from the app runs the login's `login` command as a child process** and shows its output live; tools that open a browser (gcloud, `aws sso`) work like this. A **"Run in Terminal"** button opens the OS terminal with `tunwarden login NAME` for tools that ask questions | FR-25 says the client runs the login in the user's session; the app has no terminal of its own | Embedding a terminal emulator |
| D8 | **Host keys:** the app never accepts a key. A host-key failure shows `tunwarden trust HOST` with Copy and Run in Terminal (FR-42) | Accepting a fingerprint needs the user to compare it, which ssh does in a terminal | A fingerprint dialog in the app |
| D9 | **Notifications are decided in the app**, not the daemon: one per tunnel and event at most every 5 minutes, and events within 2 seconds of each other with the same reason are combined ("3 tunnels: the google login has expired") (FR-35) | The daemon already sends every state change; rate limits are a display choice | Notification logic in the daemon |
| D10 | **Config edits use the existing `config.*` methods.** Two new methods, `config.edit_entry` and `config.remove_entry`, let the app edit profiles and logins too. "Open config file" opens it in the default editor | Forms for everything users can add; hand edits stay supported (FR-5) | Writing the file from the app |
| D11 | **`doctor` moves from the CLI into `tunwarden-core`** (`doctor::run`), so the CLI and the app show the same checks (FR-36) | One implementation | Duplicating the checks in the app |
| D12 | **Autostart from the app uses `tunwarden autostart`** (the same entry as the CLI). On Windows the entry starts the app hidden (`--background`) instead of the console CLI, so no window flashes at login | One entry per user, no Tauri autostart plugin with its own entry | Two separate login entries |
| D13 | **Tests:** Vitest and Testing Library for views against a fake client; Rust tests for the bridge against a real daemon with `fake-tunnel`; `tauri-driver` end-to-end tests on Linux and Windows CI later (WebDriver does not support macOS); a manual checklist on macOS | Same layers as the core | — |
| D14 | **Releases:** the app is built by `tauri-action` in its own workflow and attached to the GitHub Release, unsigned at first. It ships after the CLI's v0.1.0, as its own stage | Staged channels (PRD section 10) | Bundling it into the first release |

## 2. Screens

1. **Tray** (menu bar on macOS). Icon color: green when every started tunnel runs; amber when one is starting, reconnecting or unhealthy; red when one failed or a login expired; grey when nothing runs or the daemon is down (FR-31). Menu (FR-32):
   - one line per tunnel with its state, click to start or stop;
   - groups: start or stop all;
   - "Log in to google…" for each expired login;
   - Open Tunwarden, Quit.
2. **Dashboard** (FR-33, FR-34). Tunnels grouped by profile, "No profile" last; production profiles in a distinct color and a confirmation before starting. Each row: name, state with failure reason, local address with a Copy button (`127.0.0.1:5433`), uptime, restarts, and Start / Stop / Restart. A banner for expired logins with Log in, and one for config errors from hand edits.
3. **Tunnel detail.** Settings, the generated command (copy), "restart to apply" when pending, notes, and the log viewer: last 2,000 lines, follow, text filter, copy last N lines (FR-29). Edit and Remove.
4. **Add / edit forms.** Tunnel by type (command, ssh-local, ssh-remote, ssh-dynamic), profile and login forms. Errors come from daemon validation, shown on the field they name.
5. **Logins and Doctor.** Each login with its profiles, last result and Check now / Log in. The doctor report with the same sections as the CLI.
6. **Activity.** Recent events across all tunnels (FR-30).

## 3. Edge cases handled from the start

- **Daemon not running:** the app starts it through the sidecar. If that fails, the dashboard shows the error and the last lines of `daemon.log`.
- **Protocol mismatch** (a different `tunwarden` version's daemon is running): a banner with "Restart daemon", which runs `daemon restart` and keeps tunnels (FR-53).
- **Connection lost** (daemon stopped from the CLI): show "daemon stopped", retry every 2 seconds, resubscribe on reconnect.
- **Linux tray:** needs `libayatana-appindicator`; on GNOME without the AppIndicator extension there is no tray, so the app keeps its window and says why.
- **macOS:** the app is a menu-bar app with a Dock icon only while the window is open.

## 4. Build order (in stages)

- **M3a:** project setup, bridge, generated types, tray, dashboard, start/stop/restart, production confirmation, log viewer, notifications, quit choice.
- **M3b:** forms with `config.edit_entry`/`remove_entry`, the login flow, Logins and Doctor views (with `doctor` moved into core), Activity, autostart toggle.
- **M3c:** app release workflow, Linux and Windows end-to-end tests, manual macOS checklist.

## 5. Open questions for review

0. **Brand and logo: decided.** Concept B (tunnel with a signal lamp) is the app and tray icon; the warden is the website mascot (`PRODUCT.md`, `docs/brand/concepts/`). Original note: A pixelated tunnel with a warden guarding it, as the logo and the base of the theme. Proposed scope: app icon, tray glyph (monochrome on macOS, with a small state-colour dot), first-run and empty states, installer art and the website hero; the dashboard and logs stay plain for scanning. Do this as a design pass with the `impeccable` skill (direction, DESIGN.md tokens, audit of the existing screens), starting with two or three logo sketches.

1. Visual direction: a dense, terminal-like dashboard (monospace, compact rows) or a softer, card-based one?
2. Should the app show a first-run screen that runs `init` or `import` when there is no config, or just point to the CLI?

## 6. Setup (done)

- pnpm 12 workspace at the repo root (`pnpm-workspace.yaml`, `apps/*`); `pnpm desktop` runs the app in development.
- `apps/desktop`: Vite 8, React 19.3, TypeScript 7, Tailwind 4.3, Vitest; `@tauri-apps/api` 2.12 and the notification plugin.
- `apps/desktop/src-tauri`: crate `tunwarden-desktop` in the Cargo workspace, Tauri 2.12 with `tray-icon`, identifier `io.github.om-jannu.tunwarden.desktop`, a strict CSP, and only `core:default` and `notification:default` permissions. One bridge command so far (`daemon_status`).
- CI: the core jobs exclude `tunwarden-desktop`; a separate `desktop` job installs the Linux webview and tray libraries and builds the app on all three OSes. The CLI release (`dist`) does not include the app.
- The Windows-target lint cannot cover the app from macOS (its build script needs Windows resource tools); the `desktop` CI job on Windows covers it.

## 7. M3a (done)

- **Design tokens** (`src/index.css`): cool slate surfaces, colour only for tunnel state (running green, starting/reconnecting/unhealthy amber, failed red, stopped grey) plus one production violet; the system UI font at 13 px, monospace only for addresses, commands and logs; light and dark follow the OS. The one distinctive element is the state rail, a 3 px left edge on each row in its state colour, and violet on production profiles.
- **Types:** `src/protocol.ts` is generated from `tunwarden_core::proto` with `ts-rs` (`cargo test -p tunwarden-core --features ts export_typescript`); CI fails if it is stale.
- **Bridge** (`src-tauri/src/bridge.rs`): commands `config_info`, `init_example`, `open_config`, `import_preview`, `import_apply`, `status`, `list`, `act`, `logs_tail`, `auth_status`, `restart_daemon`, `quit`; a watcher subscribes to `tunnels` and `activity`, emits `daemon` events, keeps the last status for the tray, and reconnects every 2 seconds. The daemon is started through the `tunwarden` binary next to the app, else the one on PATH.
- **Tray** (`src-tauri/src/tray.rs`): a dot drawn at runtime in the health colour, the summary as tooltip and first menu line, one check item per tunnel (click starts or stops), group start/stop, Open, Quit. Starting a production tunnel from the tray opens the window to confirm.
- **Screens:** first run (paste or choose a file, preview, pick, add; example config; config path), dashboard (grouped by profile, filter, start/stop/restart, copy address, banners for daemon problems, config errors and expired logins), tunnel panel (facts, actions, log viewer with follow, filter and copy last 100), production confirmation, quit choice. Closing the window hides it.
- **Notifications** (`src/logic.ts`, `Notifier`): failures, drops and recoveries; changes within 2 seconds with the same reason are one notification; the same tunnel and event at most once per 5 minutes.
- **Tests:** Rust unit tests for tray health and the icon; Vitest for grouping, health, detail, uptime, log parsing and the notifier; a rendering test against the mock daemon (production confirmation, first run). A browser preview (`pnpm --filter tunwarden-desktop dev`, `?fresh` for first run) uses the mock in `src/mock.ts`.
- **Checked by hand:** screenshots of the dashboard (dark), tunnel panel and first run (light); the real app started its own daemon from a temporary state directory, and tunnels kept running after it quit.
- **Not yet:** the login flow from the app (M3b; the banner gives the command to copy), forms, Logins, Doctor and Activity views, bundling the CLI as a sidecar and installers (M3c), and the app on Linux and Windows.

## 9. M3c (done, except what needs Windows)

- **The CLI ships inside the app** (D2). `pnpm --filter tunwarden-desktop bundle` (or `pnpm desktop:build` at the root) runs `tauri build --config src-tauri/tauri.bundle.conf.json`. Its `beforeBuildCommand` runs `scripts/sidecar.mjs`, which builds `tunwarden-cli` for the target (`TAURI_ENV_TARGET_TRIPLE`) and copies it to `src-tauri/binaries/tunwarden-<triple>`; `externalBin` puts it next to the app's binary (on macOS `Tunwarden.app/Contents/MacOS/tunwarden`), where the app looks first. The sidecar is only in the bundle config, so plain `cargo build` and `pnpm desktop` need no extra step; debug builds find `target/debug/tunwarden` next to the app.
- **Installers:** macOS `.app` and `.dmg` (minimum macOS 11), Windows NSIS and MSI (WebView2 bootstrapper), Linux `.deb`, `.rpm` and AppImage, all unsigned. Built locally on macOS: `Tunwarden.app` 24 MB, `Tunwarden_0.1.0_aarch64.dmg` 7.8 MB, with the CLI inside.
- **Starting hidden and one copy.** The window starts hidden and is shown at launch unless the app is started with `--background`. `tauri-plugin-single-instance` makes a second launch show the running app's window and exit.
- **Windows autostart (D12).** From the app, the `Run` entry starts `Tunwarden.exe --background`, so no console window opens at login; the app then starts the daemon. On macOS and Linux the app writes the same launchd or systemd entry as the CLI, which runs the bundled CLI's daemon without the app. `autostart::Entry::app` selects this.
- **Release:** `.github/workflows/app-release.yml`, started by hand with a tag (for example `v0.1.0`), builds macOS Apple Silicon and Intel, Linux x86-64 and Windows x86-64 with `tauri-action` v1 and adds the installers to that release. It is not started by tags, because every version tag already starts the CLI release.
- **Smoke test** (`scripts/smoke.sh APP FAKE_TUNNEL`): starts the app with a temporary config and state directory, checks that it runs its daemon with the CLI next to it, that a second launch hands over, that a tunnel runs, and that it keeps running after the app quits. It refuses to run while another copy of the app is open. CI runs it on Linux (under Xvfb) and macOS. Passed on Linux in Docker.
- **Verified by hand on 0.1.0-rc.3 (28 Nov 2026):** macOS (Apple Silicon): install from the `.dmg`, Open Anyway, window, tray popover over full-screen apps, palette, daemon switch-over. Windows: CLI installer, `.exe` installer, adding, starting, reconnecting and stopping a tunnel (no processes left), production prompt, tray popover, light and dark tray, tunnels outliving the app, autostart after sign-in. Not checked by hand: the `.msi` installer, the Linux packages, and spike S3 (the pipe accepts only its user).
