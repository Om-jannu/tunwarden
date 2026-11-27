# PRD: Tunwarden – Tunnel Manager

Author: Om Jannu · Status: Draft v3 · Date: 27 Nov 2026 · Audience: the coding agent / contributors building this

## 0. Changes

### Draft v3

Tunwarden is now generic. Nothing in the code is specific to gcloud, kubectl or any other tool:

- `[[projects]]` (a gcloud project, account and config directory) is replaced by two general tables. A **profile** is a set of environment variables, a `production` flag and an optional login. A **login** is a credential the user renews now and then, described by a `check` command, a `login` command, `expired` patterns and its own environment. Several profiles can share one login.
- The `gcloud-iap` and `kubectl-port-forward` types are removed. Those tunnels are `command` tunnels (the renamed `custom` type, now the default) and the example config shows them. The SSH types stay, because SSH needs safe defaults (FR-42) and supports `trust`.
- The auth check, expiry detection and re-login (FR-22 to FR-25) work for any tool through the login's commands and patterns. `auth` and `login` take a login or profile name; `--project` becomes `--profile`.
- `[tools]` keeps only `ssh` and gains `path`, a list of extra directories to find commands in. The gcloud SDK install folders are no longer searched.
- A profile's environment is added on top of the inherited environment. Nothing is removed from it.
- `auth_check_interval_minutes` is renamed `login_check_interval_minutes`.

### Draft v2 (from Draft v1)

Draft v1 used the name Burrow. The main changes in v2:

- Renamed to Tunwarden. The name Burrow clashes with an existing open-source tunneling tool and a crates.io crate.
- The background daemon is required and moves into M1.
- The config file is TOML instead of YAML, so tool edits keep comments.
- Readiness and liveness are now separate checks. The v1 liveness limit is stated, and latency is dropped from v1.
- gcloud login is tracked per auth context (account plus config directory), not per project. (Generalised to logins in v3.)
- Each tunnel has a list of forwards, and the bind address defaults to `127.0.0.1`.
- Rules for hand edits while the daemon runs, a failure reason on every `failed` tunnel, Windows process rules, `tunwarden trust`, autostart, a version handshake and a config schema version.
- `gcloud-ssh` moves past v1. `custom` tunnels may now use a project. (v3 removes all gcloud types.)
- Open questions 1–5 are closed.

v2.1 adds decisions from the technical design (`docs/technical-design.md`): the crash sweep applies to Linux as well as macOS (FR-52), the client runs the gcloud login (FR-25), `trust` hands off to ssh (FR-42), an inconclusive auth check never stops tunnels (FR-22), Windows may stop tunnels by force (FR-12), `daemon stop` no longer restores tunnels unless `--keep` is given (FR-15), and the tool lookup order (FR-36).

Removed requirements keep their number and are marked as removed, so references stay valid.

## 1. Overview

Tunwarden is a free, open-source, cross-platform tool (macOS, Windows, Linux) that lets an engineer manage many tunnels of any kind (plain SSH, or any command that opens a local port, such as `gcloud compute start-iap-tunnel`, `kubectl port-forward` or `aws ssm start-session`) from one place. It replaces a pile of personal zsh/.zshrc scripts with a reliable tool that can start, stop and add tunnels, show live health, reconnect automatically, keep logs, and handle logins that expire (for example gcloud's 12-hour session) across several accounts and environments.

It ships as two front ends over one shared core and one background daemon:

- A **CLI** (`tunwarden`) for daily terminal use and scripting.
- A **desktop app** (system tray + dashboard window) for at-a-glance status and one-click control.

The tool is generic. It has no code specific to a cloud or tool, and nothing company-specific (project IDs, hostnames, VM names) is bundled; everything comes from the user's own config file.

## 2. Problem

The primary user works with many servers reachable only through cloud-provided tunnels (for example `gcloud compute start-iap-tunnel` or `gcloud compute ssh --tunnel-through-iap`). Today this is managed by shell aliases and scripts, which causes recurring pain:

1. Tunnels are started and stopped by hand, with no overview of what is running.
2. gcloud logs the user out about every 12 hours, tunnels silently die, and it is not obvious why.
3. There are several gcloud projects, each with different tunnel commands; switching the global `gcloud config` breaks other tunnels or runs commands against the wrong project.
4. Dropped tunnels (sleep, Wi-Fi change, VPN, timeouts) are not retried.
5. There is no logging, no health status, and adding a new tunnel means editing shell files.

## 3. Goals and non-goals

### Goals

- G1. Start, stop, restart and add tunnels at any time, individually or in groups.
- G2. Show accurate live status and health for every tunnel, within the limits stated in 7.4.
- G3. Detect expired logins of any tool, make re-login one step, and recover affected tunnels automatically afterwards.
- G4. Support several environments and accounts at once (through profiles) without them interfering.
- G5. Reconnect dropped tunnels automatically with sensible backoff.
- G6. Keep per-tunnel logs that are easy to view.
- G7. Run on macOS, Windows and Linux, installed from a single download with no language runtime to install. The tools the user's tunnels run (ssh, gcloud, kubectl and so on) and the OS webview (WebKitGTK on Linux) are the user's own prerequisites.
- G8. Be safe to open source: no secrets stored, no company data shipped.
- G9. Support every kind of tunnel through one model: built-in types for plain SSH, plus a `command` type for any command that opens a local port (see 7.10).

### Non-goals (v1)

- Not a VPN or a replacement for ssh, gcloud or kubectl. Tunwarden only supervises tunnels built on tools the user already has.
- No team sync, cloud accounts or remote control. Everything is local to one machine.
- No mobile app.
- No storing or proxying of credentials.
- No App Store distribution. Releases are via GitHub, with signing as an optional later step.
- No protocol-level health checks (database or HTTP pings) and no latency measurement. See 7.4 and section 14.
- No tool-specific built-in types beyond SSH. gcloud, kubectl, AWS and other tunnels are `command` tunnels. See section 14.
- No update check.

## 4. Users

**Primary:** a backend/DevOps-minded engineer who connects to databases, VMs and internal services through cloud tunnels daily and is comfortable with a terminal.

**Secondary:** teammates and open-source users with similar setups who want the same convenience. They bring their own config.

## 5. Technology decision

**Decision: Rust for the core, daemon and CLI, with Tauri 2 and React (TypeScript) for the desktop app.**

Rationale:

- Tauri produces small, fast desktop apps (roughly 5–10 MB) that use the operating system's built-in webview, with a web frontend.
- React with TypeScript is the frontend, because it is the framework most contributors already know.
- Rust suits the core work: supervising child processes, handling process groups and signals, and doing lightweight network probes, with no runtime for users to install.
- One Rust crate holds all the logic and is used by the daemon, the CLI and the Tauri app, so behavior is identical everywhere.
- Tauri has built-in support for the system tray, notifications and launch at login, and an official GitHub Action builds installers for all three operating systems.

Tradeoffs to accept: Rust has a steeper learning curve and slower compile times than Go, and Tauri depends on each OS's webview (WebView2 on Windows, WebKitGTK on Linux), which the installers and docs must account for. Keep the core free of UI code so it stays easy to test and contribute to.

Considered and not chosen for v1: Go with Wails (simpler language, but a larger app and weaker tray support), Electron (large bundles) and a Python UI (hard to package for all three platforms).

**Architecture:** a Cargo workspace with these parts.

1. `tunwarden-core` – a library crate with config loading and editing, tunnel types, the supervisor, readiness and liveness checks, auth watcher, state store, logger, process control and the control-interface protocol. No UI code.
2. `tunwarden-cli` – the `tunwarden` binary. It is a thin client over the core crate, and `tunwarden daemon` runs the background daemon from the same binary.
3. `src-tauri` plus the React frontend – the Tauri app, a thin client of the daemon.
4. A test helper (`fake-tunnel`) written in Rust, so the same fake tunnels run on all three OSes (see section 9, Testability).

The daemon is the single owner of all running tunnels (see 7.9). The CLI and the desktop app must not duplicate logic.

## 6. Key concepts

- **Profile:** settings shared by tunnels: environment variables (for example `CLOUDSDK_CORE_PROJECT` or `AWS_PROFILE`), a `production` flag, and optionally the login its tunnels depend on. Optional; any tunnel may use one.
- **Login:** a credential the user renews now and then, for any tool. It has an optional `check` command that succeeds while the login is valid, an optional interactive `login` command, `expired` patterns (output that means "log in again") and its own environment. Several profiles can share one login, so one re-login recovers all of them.
- **Tunnel:** a named connection of a given type with one or more forwards, optionally using one profile. It may belong to zero or more groups.
- **Forward:** one port mapping of a tunnel: a local bind address and port to a remote host and port, a SOCKS proxy, or a remote forward.
- **Group:** a user-defined label (for example "daily") used to start or stop several tunnels together.
- **Daemon:** the single background process that owns all supervisors and serves the CLI and the desktop app.
- **Supervisor:** the component inside the daemon that runs one tunnel's command, watches it, and restarts it when needed.
- **Tunnel type:** how a tunnel is built, checked and diagnosed. Built-in types cover SSH local, remote and dynamic forwards and any command (see 7.10).
- **Prerequisite tool:** a program the config runs: ssh for SSH types, a `command` tunnel's program, and each login's `check` and `login` programs.
- **Readiness:** the tunnel has opened what it should open, for example its local port is listening.
- **Liveness:** the tunnel is still usable after it became ready. See 7.4 for what v1 can and cannot detect.

## 7. Functional requirements

### 7.1 Configuration

- FR-1. A single human-editable TOML file defines settings, tools, logins, profiles and tunnels. Default location: `~/.config/tunwarden/config.toml` on macOS and Linux (honoring `XDG_CONFIG_HOME`), and `%APPDATA%\tunwarden\config.toml` on Windows. Overridable by the `--config` flag and the `TUNWARDEN_CONFIG` environment variable.
- FR-2. Profile and login fields:
  - Profile: name (required), `env` (a map of environment variables), `production` flag (default false, see FR-33), `login` (the name of a login, optional).
  - Login: name (required), `check` (a command line that exits 0 while the login is valid, optional), `login` (an interactive command line that renews it, optional), `expired` (regular expressions, matched ignoring case, optional), `env` (a map of environment variables for both commands).
- FR-3. Tunnel fields: name, type (default: `command`), profile (optional, any type), the settings of its type (see 7.10), a list of forwards, groups (optional), auto-restart (default true), max retries (default 10), health interval in seconds (default 15).
- FR-4. Validate on load:
  - unique tunnel, profile and login names;
  - names contain only letters, digits, `-`, `_`, `.`, start with a letter or digit, are at most 64 characters, and are not Windows device names (`CON`, `NUL`, `COM1` and so on). Names are unique ignoring case, because they become file names and macOS and Windows file systems ignore case;
  - no two forwards anywhere in the config share the same local bind address and port;
  - a tunnel that names a profile, and a profile that names a login, reference existing ones;
  - each type's required fields are present and valid (for `command`, and for a login's `check` and `login`, the command line parses under the rules in FR-8 and is non-empty), and `expired` patterns are valid regular expressions;
  - the schema version is supported (FR-50).

  Errors must say exactly what is wrong and where (file, line, field).
- FR-5. Adding, editing and removing tunnels is possible from the CLI and from the UI. The daemon is the only process that writes the config file; the CLI and app send edits to it. Tool edits preserve comments and formatting. Editing the file by hand is always supported:
  - The daemon watches the file and reloads it atomically when it changes.
  - Before every write, the daemon re-reads the file and compares a hash with the content it last loaded. If the file was changed by hand since then, the daemon reloads it first and applies its edit on top. If the edit can no longer be applied (for example the tunnel was removed by hand), it refuses with a clear error. It never overwrites the file blindly.
  - If a hand edit makes the file invalid, the daemon keeps running on the last valid config and shows the exact error in `status`, `doctor` and the app until the file is fixed.
  - When a running tunnel's settings change, it keeps running and is marked "restart to apply". When a running tunnel is removed from the config, it is stopped. New tunnels and changes to stopped tunnels apply immediately.
- FR-6. `init` creates an example config. `import` turns tunnel commands pasted by the user (for example from their shell aliases) into config entries, matched to a built-in type when possible (FR-43). It warns about any part of a pasted command that FR-8 does not support.

Illustrative example (field names may be refined during implementation):

```toml
version = 1

[settings]
restore_on_start = true
login_check_interval_minutes = 5

[tools]
path = ["~/google-cloud-sdk/bin"]   # extra directories to find commands in

[[logins]]
name = "google"
check = "gcloud config config-helper --force-auth-refresh --format=none"
login = "gcloud auth login me@example.com"
expired = ["Reauthentication failed", "There was a problem refreshing your current auth tokens"]
env = { CLOUDSDK_CORE_ACCOUNT = "me@example.com" }

[[profiles]]
name = "staging"
login = "google"
env = { CLOUDSDK_CORE_PROJECT = "example-staging-123" }

[[profiles]]
name = "prod"
login = "google"
production = true
env = { CLOUDSDK_CORE_PROJECT = "example-prod-456", CLOUDSDK_CONFIG = "~/.config/gcloud-prod" }

[[tunnels]]
name = "staging-db"
profile = "staging"
command = "gcloud compute start-iap-tunnel db-1 5432 --zone=europe-west1-b --local-host-port=127.0.0.1:5433"
groups = ["daily"]
forwards = [{ local_port = 5433 }]

[[tunnels]]
name = "bastion-redis"
type = "ssh-local"
host = "bastion"          # a host or alias from ~/.ssh/config
jump = ["jump.example.com"]
forwards = [{ local_port = 6380, remote_host = "redis.internal", remote_port = 6379 }]
```

### 7.2 Profile isolation

- FR-7. Every tunnel process that uses a profile is started with the inherited environment plus the login's environment (if the profile has a login), so the tunnel runs as the account the check verifies, plus the profile's environment. The profile's value wins over the login's for the same variable. This is how tools are pointed at an account, project or config directory (for example `CLOUDSDK_CORE_PROJECT`, `CLOUDSDK_CONFIG`, `AWS_PROFILE`, `KUBECONFIG`). The tool never modifies any tool's global configuration.
- FR-8. Commands are never run through a shell (see section 9, Security). A `command` tunnel's command line, and a login's `check` and `login` commands, are split into arguments with shell-style word splitting and quoting, and a leading `~` is expanded. Pipes, `&&`, `;`, redirection, `$VAR` and other shell features are not supported. Validation (FR-4) and `import` (FR-6) report them clearly. A user who needs shell features can point the command at their own script.

### 7.3 Tunnel lifecycle

- FR-9. Start, stop and restart for one tunnel, several named tunnels, a group, a profile, or all tunnels.
- FR-10. Tunnels are run by the daemon, so they survive closing the terminal or the desktop window. Closing the tray app must offer a choice: keep tunnels running or stop them.
- FR-11. Each tunnel has exactly one of these states: `stopped`, `starting`, `running`, `unhealthy`, `reconnecting`, `failed`. State transitions are logged with timestamps. A `failed` tunnel always carries a reason (FR-48).
- FR-12. Stopping must terminate the tunnel command and all of its child processes cleanly on every OS: graceful first, forced after a timeout (default 5 seconds).
  - On macOS and Linux, each tunnel runs in its own process group. Stop sends SIGTERM to the group, then SIGKILL after the timeout.
  - On Windows, each tunnel runs in its own job object and process group. Stop sends CTRL_BREAK_EVENT where possible, then terminates the job object after the timeout. If a background daemon cannot deliver CTRL_BREAK_EVENT reliably, Windows tunnels are stopped by terminating the job object directly. ssh, gcloud and kubectl hold no state that a forced stop loses; other commands are expected to behave the same.
- FR-13. Before starting, check each forward's local port; if one is already in use, refuse to start and report which process holds it where the OS allows.
- FR-14. Starting an already-running tunnel is a no-op with a clear message.
- FR-15. Auto-start and restore:
  - Tunnels marked auto-start are started when the daemon starts.
  - When `restore_on_start` is on (the default), tunnels that were running when the daemon crashed or the machine rebooted are started again.
  - `restore_on_start` governs restoring after a crash or reboot. After a clean `daemon stop --keep`, tunnels are restored regardless, because the user asked for it.
  - `tunwarden daemon stop` stops every tunnel and they stay stopped on the next daemon start. `tunwarden daemon stop --keep` stops them now but restores them on the next start. A daemon restart, an upgrade, and the OS stopping the daemon at logout or shutdown restore them like `--keep`.
  - `tunwarden autostart enable|disable|status` registers the daemon to start at login (launchd agent on macOS, systemd user unit on Linux, the `Run` registry key on Windows). This works without the desktop app installed. The entry records the shell's `PATH` (macOS, Linux), never restarts the daemon after it exits, and takes effect at the next login without touching the running daemon.

### 7.4 Health and reconnect

Readiness and liveness are separate. Periodic checks must not open connections through the tunnel. Doing so would create a new remote session (for example IAP or kubectl) every interval and leave aborted-connection noise in the remote service's logs.

- FR-16. A tunnel is `running` only when its readiness check passes, not merely when the process exists. For types with a local port, readiness means the tunnel's process (or one of its children) holds a listening socket on every configured local port, read from the OS socket table. The check runs every second while starting and at the configured interval afterwards.
- FR-17. Startup and health limits:
  - A tunnel that is not ready within the startup grace period (25 seconds) goes to `reconnecting`.
  - A `running` tunnel whose readiness check fails goes to `unhealthy`.
  - After 3 consecutive failed checks, an `unhealthy` tunnel is restarted (`reconnecting`). If a check passes first, it returns to `running`.
- FR-18. If the process exits, retry with exponential backoff (1s, 2s, 4s, doubling, capped at 60s) up to the max retries. A tunnel that stays `running` for 60 seconds resets its retry counter. After the limit, mark `failed` with reason "retries exhausted" and the last error.
- FR-19. Detect system sleep/wake and network changes and trigger a prompt reconnect check instead of waiting for the next interval. After a wake or network change, tunnels that are `failed` for a reason other than credentials get a fresh set of retries.
- FR-20. (Removed in v2.) Latency was measured as connect time to the local port, which is close to 0 ms for every tunnel and says nothing about the remote side. Latency moves to section 14 with the deeper checks.
- FR-21. (Moved to section 14 in v2.) Deeper, protocol-level checks.
- FR-49. Liveness in v1 comes from these signals:
  - the tunnel process exiting;
  - SSH keepalives making `ssh` exit when the remote side stops answering (FR-42);
  - credential and error patterns in the tunnel's output (FR-24, FR-45);
  - the scheduled login check (FR-22).

  **Known v1 limit:** a tunnel whose local port is still listening while the remote side is dead (for example a stopped VM behind an IAP tunnel, or a kubectl port-forward to a pod that no longer answers) can show as `running`. The docs must state this limit. The deeper checks in section 14 are the planned fix.

### 7.5 Logins

Tunwarden knows nothing about any particular tool's login. A `[[logins]]` entry describes it with commands and patterns, and the example config has ready-made entries for gcloud and AWS SSO.

- FR-22. Each login with a `check` command is checked on a schedule (default every 5 minutes, `login_check_interval_minutes`) and on demand. Only logins used by an active tunnel are checked on the schedule. The check runs with the login's environment and a null standard input. Its standard output is discarded unread, so a tool that prints a token there never leaks it; only the exit status and error output are used. The result is one of:
  - `ok` (exit 0): nothing changes.
  - `expired`: the check failed and its error output matches one of the login's `expired` patterns, or the login has no `expired` patterns and the check failed in any way. Every active tunnel whose profile uses that login is stopped and marked `failed` with reason `auth expired`. A tunnel with expired credentials may keep listening locally while it cannot open new connections, so leaving it `running` would be wrong.
  - `unknown`: the check failed, its error output matches no `expired` pattern, or it timed out (30 seconds) or could not be run. Nothing is stopped, and the result is shown in `auth` and `doctor`. A flaky network must not look like an expired login. Users should list `expired` patterns for any tool whose check can fail for other reasons.

  The check must force a real refresh where the tool caches tokens. For gcloud, the example uses `gcloud config config-helper --force-auth-refresh`; `gcloud auth print-access-token` reported success against a real environment while the session had expired, because it can return a cached token.
- FR-23. (Removed in v2.) Warning before expiry. Tools do not reliably expose when re-login will be needed (gcloud's 12-hour limit is usually an organization session policy). Tunwarden instead shows "auth expired" per login, and the profiles using it, as soon as the scheduled check or tunnel output detects it.
- FR-24. A tunnel's output is matched against its login's `expired` patterns. On a match, stop retrying immediately, mark it `failed` with reason `auth expired` and an actionable message ("the google login has expired; run `tunwarden login google`"), and do not hammer the auth system.
- FR-25. A one-step re-login action, for a login or a profile (meaning its login), runs the login's `login` command (for example `gcloud auth login`, which opens the browser) with the login's environment. On success, every tunnel that uses that login, through any profile, and failed because of it is restarted automatically.
  - When a tunnel's own output shows an expired login, the daemon checks that login right away, so the other tunnels that share it fail at once instead of at the next scheduled check.
  - The client runs the login, not the daemon: `tunwarden login` runs it in the user's terminal, and the desktop app runs it from the user's session. The daemon has no terminal and may not be able to open a browser (for example as a systemd user service), so it could not show the tool's prompts, its fallback URL-and-code flow, or its errors.
  - When the command finishes, the client tells the daemon. The daemon runs the check again (FR-22) and restarts the affected tunnels only if the result is `ok`. A login without a `check` counts as `ok`.
  - A login without a `login` command tells the user to log in with their tool and then run `tunwarden auth NAME`.
- FR-26. The tool never reads, stores or transmits tokens or passwords. The user's tools own all credentials.

### 7.6 Logging

- FR-27. Each tunnel has its own log file containing the tunnel command's output plus Tunwarden's own events (start, ready, unhealthy, retry, stopped, failed with reason), each with a timestamp. Log files are stored in the per-OS state directory (FR-51) and are readable only by the current user.
- FR-28. Logs rotate by size (default 5 MB, keep at least one previous file).
- FR-29. CLI: show the last N lines and follow live. UI: a log viewer per tunnel with follow mode, text filter, and a copy-last-lines button.
- FR-30. A global activity view shows recent events across all tunnels.

### 7.7 CLI

Binary name `tunwarden`. Commands (names are normative, flags may be refined):

| Command | Purpose |
| --- | --- |
| `init` | Create an example config |
| `import` | Turn pasted commands into config entries (FR-6) |
| `list` | List configured tunnels |
| `status [names]` | Table of state, failure reason, PID, restarts, uptime, last error, "restart to apply" |
| `start`, `stop`, `restart` | Accept names, `--all`, `--profile`, `--group`; wait up to `--wait` seconds (default 30) and print the result per tunnel |
| `add`, `edit`, `remove` | Manage tunnel entries |
| `show-command NAME` | Print the generated command for a tunnel (FR-40) |
| `logs NAME` | Show/follow logs, with `-n` and `-f` |
| `auth [NAME]` | Check logins (all, or one login or profile); exits non-zero if one has expired |
| `login NAME` | Run a login's `login` command (NAME is a login or profile) and recover auth-failed tunnels |
| `trust HOST` | Accept an SSH host key once (FR-42) |
| `autostart enable\|disable` | Start the daemon at login (FR-15) |
| `daemon start\|stop\|restart\|status` | Manage the background daemon; `stop --keep` restores tunnels on the next start (FR-15) |
| `config-path` | Print config location |
| `completions SHELL` | Print a completion script for bash, zsh, fish or PowerShell |
| `doctor` | Diagnose the environment (see FR-36) |

Requirements:

- Non-zero exit codes on failure.
- A `--json` output option for `status` and `list`.
- Shell completion for bash, zsh, fish and PowerShell.
- Starting a tunnel in a `production` profile asks for confirmation unless `--yes` is given (FR-33).
- The CLI starts the daemon on demand if it is not running.

### 7.8 Desktop app

- FR-31. System tray (menu bar on macOS) icon whose color reflects overall health: all running, some degraded, or something failed or auth expired.
- FR-32. Tray menu: per-tunnel start/stop, start/stop group, re-login per login, open dashboard, quit.
- FR-33. Dashboard window listing tunnels grouped by profile, with tunnels that have no profile under "No profile". Each row shows state, failure reason, local ports, uptime and restarts. Profiles with `production = true` are shown in a distinct color and require a confirmation before any of their tunnels start.
- FR-34. Dashboard actions: add/edit/remove tunnel, start/stop/restart, copy connection string (for example `localhost:5433`), show and copy the generated command, open logs.
- FR-35. Desktop notifications for: tunnel down, reconnecting, retries exhausted, auth expired, auth recovered. The same event for the same tunnel is notified at most once every 5 minutes. When several tunnels fail together for one reason (for example auth expiry), one combined notification is shown.
- FR-36. `doctor` and an equivalent UI panel check:
  - every prerequisite tool used by the config is installed and on PATH or at its configured path, with its version (FR-46);
  - the PATH problem GUI apps and login items have on macOS (they do not inherit the shell PATH). Tools are found in this order: the path set in the config (`tools.ssh`, or a command given as a path), the daemon's PATH, the directories in `tools.path`, then known install locations per OS (for example Homebrew, `/usr/local/bin` and `~/.local/bin`). `doctor` shows which path was used and where it came from. Reading the login shell's PATH is not done in v1, because it runs the user's shell profile;
  - each login's check;
  - config validity;
  - port availability;
  - daemon status and version.

### 7.9 Shared state between CLI and app

- FR-37. A tunnel started from the CLI must appear correctly in the app and vice versa, because both are clients of the same daemon.
- FR-38. A single long-running daemon owns all supervisors and exposes a local control interface: a Unix domain socket on macOS and Linux, a named pipe on Windows. Both are restricted to the current user. The CLI and the app are clients of it, and the CLI starts it on demand.
- FR-52. The daemon and its tunnels live and die together:
  - Tunnel processes must not outlive the daemon. On Windows, use a job object that kills its processes when the daemon exits.
  - On macOS and Linux, the daemon records each tunnel's PID, process group and process start time. On its next start, it terminates the process group of any recorded process that is still running (same PID and same start time) before restoring tunnels. This sweep is required on Linux too, because Linux's parent-death signal only reaches the direct child, and many tools (gcloud, for example) start their own child processes. Spike S4 confirmed this: the parent-death signal killed only the direct child and left the grandchildren running, so it is not used.
  - A tunnel whose supervisor has vanished is reported as `stopped` with an explanatory note, never as running, and is restored according to FR-15.
- FR-53. Clients and the daemon exchange a protocol version when they connect. On a mismatch (for example after an upgrade while the old daemon is still running), the client reports it clearly and offers `tunwarden daemon restart`. Installers and upgrades restart the daemon.

### 7.10 Tunnel types

Every tunnel has a type. A type knows how to build the command, how to check that the tunnel is ready, how to recognize credential problems, and which tools it needs. v1 ships these built-in types:

| Type | What it runs | Main settings |
| --- | --- | --- |
| `ssh-local` | SSH local forward (`ssh -L`) | SSH host or alias, forwards (local bind address and port, remote host, remote port) |
| `ssh-remote` | SSH remote forward (`ssh -R`) | SSH host or alias, forwards (remote port, local host, local port) |
| `ssh-dynamic` | SOCKS proxy over SSH (`ssh -D`) | SSH host or alias, local bind address and port |
| `command` (default) | Any command the user provides | full command line, forwards (local ports to check), optional credential-error patterns |

Every type can use a profile. Tool-specific tunnels such as `gcloud compute start-iap-tunnel`, `gcloud compute ssh --tunnel-through-iap`, `kubectl port-forward` and `aws ssm start-session` are `command` tunnels; the example config shows each.

Any SSH type can go through one or more jump hosts (`ssh -J` / ProxyJump). The local bind address of every forward defaults to `127.0.0.1`, so tunnels are not exposed to the local network unless the user asks for it.

- FR-39. Every tunnel has a type. The supervisor, health checks, retry, logging, state and UI are shared by all types: adding a new type must only require a new type definition, not changes to those parts.
- FR-40. Built-in types build the command from structured fields, so users do not write full commands. The generated command can be shown and copied from the UI and the CLI. The CLI `add` command accepts a type and its settings as flags. Only the `command` type takes a full command line.
- FR-41. Each type defines four things: how to build the command and environment, how to check readiness, which output means a credential problem, and which tools it needs.
- FR-42. SSH types use the system OpenSSH binary (not bundled) and respect `~/.ssh/config`, keys and ssh-agent.
  - By default they add `ServerAliveInterval=15`, `ServerAliveCountMax=3` and `ExitOnForwardFailure=yes`, so a dead tunnel exits and is restarted.
  - They run with `BatchMode=yes`, so a background tunnel never waits for a prompt.
  - They turn off SSH connection sharing (`ControlMaster=no`, `ControlPath=none`), so each tunnel owns its own connection. Otherwise a `ControlMaster`/`ControlPersist` setting in `~/.ssh/config` could make a tunnel ride on a connection it doesn't own, or leave a background master running after a stop. The cost is a fresh SSH handshake on each start.
  - An unknown or changed host key makes the tunnel fail with a "host key" reason and the hint `tunwarden trust HOST`. That command runs `ssh` interactively in the user's terminal, through the same jump hosts as the tunnel, with `StrictHostKeyChecking=ask`. ssh itself shows the fingerprint, asks the user, and writes `known_hosts`. Tunwarden never edits `known_hosts`, so ssh's own handling of hashed entries and host aliases stays correct. In the desktop app, the host-key failure shows the command to run in a terminal.
  - SSH passwords and key passphrases are never stored; a passphrase-protected key must be loaded in the agent or keychain.
- FR-43. Creating SSH tunnels can start from hosts found in `~/.ssh/config`. Pasted `ssh` commands (FR-6) are matched to an SSH type when possible; everything else is saved as `command`, with local ports guessed from the command. Leading `NAME=value` assignments become a profile with that environment (an existing profile with the same environment is reused).
- FR-44. Readiness defaults to every local port listening (FR-16) for `ssh-local`, `ssh-dynamic` and `command`. For `ssh-remote`, which has no local listening port, the tunnel counts as ready once the process is still up after a short delay (default 5 seconds); `ExitOnForwardFailure` makes it exit if the remote forward cannot be set up. A `command` tunnel without forwards counts as ready while its process runs.
- FR-45. Credential problems stop retries with an actionable message, as FR-24 does for expired logins. The SSH types detect SSH permission denied, a key not in the agent, and an unknown or changed host key. A `command` tunnel lists its own `credential_patterns` (for example kubectl's "You must be logged in to the server", or IAP's `4033: 'not authorized'`, which means a missing permission rather than an expired login). Built-in patterns live in the type definition and are covered by tests.
- FR-46. Before starting, the tool checks that the tunnel's program is installed (on PATH or at a configured path) and says which one is missing. The same checks appear in `doctor`.
- FR-47. Groups, profiles, logins, status, logs, retry and notifications work identically for every type. Any tunnel with a profile gets the profile's environment (FR-7) and its login's handling (FR-22 to FR-25).

### 7.11 Additional requirements (new in v2)

- FR-48. Every `failed` state has one reason from a fixed list: `auth expired`, `credentials` (a credential error other than an expired login), `host key`, `port conflict`, `missing tool`, `spawn error`, `retries exhausted`, `invalid config`. The CLI, app and notifications show the reason and the next step.
- FR-50. The config file has a top-level `version` field (starting at 1). A newer Tunwarden migrates older versions; an older Tunwarden refuses a newer version with a clear message.
- FR-51. State, logs and the control socket live in a per-OS state directory, readable only by the current user: `~/Library/Application Support/tunwarden` on macOS, `$XDG_STATE_HOME/tunwarden` (default `~/.local/state/tunwarden`) on Linux, `%LOCALAPPDATA%\tunwarden` on Windows.
- FR-54. On Windows, some tools are installed as batch scripts (gcloud as `gcloud.cmd`, for example), which can only run through `cmd.exe`. This is the only allowed exception to "never through a shell". Tunwarden relies on Rust's standard batch-file argument escaping, which refuses any argument it cannot escape safely, and reports such a value as a config error.
- FR-55. The desktop installers include the `tunwarden` CLI and daemon and offer to add the CLI to PATH. The CLI is also released on its own for users who do not want the app.

## 8. State machine

States and transitions:

- `stopped` → `starting` on start.
- `starting` → `running` when readiness first passes; → `failed` on spawn error, port conflict, missing tool or credential error; → `reconnecting` if the process exits or readiness does not pass within the 25-second startup grace.
- `running` → `unhealthy` on a failed readiness check; → `reconnecting` if the process exits; → `failed` on a credential error in the output or an `expired` login check (FR-22).
- `unhealthy` → `running` if a check passes; → `reconnecting` after 3 consecutive failed checks.
- `reconnecting` → `starting` after the backoff delay; → `failed` (`retries exhausted`) after max retries.
- Any active state → `stopped` on user stop.
- `failed` → `starting` on user start, after login recovery (FR-25), or, for non-credential reasons, after a wake or network change (FR-19).

A tunnel whose supervisor has vanished (daemon crash, reboot) must be reported as `stopped` with an explanatory note, never as running.

## 9. Non-functional requirements

- **Platforms:** macOS (Apple Silicon and Intel), Windows 10/11, Linux (x86-64 and arm64).
- **Footprint:** idle CPU near zero and low memory; health checks must be lightweight. A login check runs the user's tool, which can take a second or more, so it only runs for logins with active tunnels (FR-22).
- **Reliability:** no orphaned tunnel processes after stop, quit, daemon crash or reboot (FR-12, FR-52); no data loss in state or config files (atomic writes).
- **Security:**
  - No credentials stored.
  - Commands are executed directly as an argument list, never through a shell, so config values cannot inject commands. The only exception is FR-54.
  - The control socket or pipe, state files and logs are readable only by the current user.
  - Forwards bind to `127.0.0.1` by default.
  - Names are restricted to safe characters because they become file names.
- **Privacy:** no telemetry and no update check. No network calls except those made by the user's own tunnel and login commands, and `tunwarden trust` fetching a host key the user asked for.
- **Usability:** every error says what happened and what to do next.
- **Observability:** all state changes and retries are in the logs.
- **Testability:** the core must be testable without ssh or any cloud tool, using the `fake-tunnel` helper and a fake login tool. It can listen on a port, exit on a schedule, print auth-error text, and spawn child processes for the orphan tests.

## 10. Open-source requirements

- License: dual MIT OR Apache-2.0.
- Repository contains an example config only, with placeholder project IDs and hostnames; the user's real config path is git-ignored.
- README with install steps, quick start, config reference, platform notes, the v1 liveness limit (FR-49), and a security section.
- CONTRIBUTING and issue templates; semantic versioning and a changelog.
- GitHub Actions on every push:
  - On macOS, Windows and Linux: build, unit tests, and the tests that use `fake-tunnel` and a fake login tool. These cover process control and no-orphan tests (AC-4), the supervisor, retry, auth detection and the daemon.
  - On Linux only, because GitHub's macOS and Windows runners cannot run Linux containers: integration tests with a Docker SSH server and jump host, and `kind` for a kubectl `command` tunnel.
- On a version tag, build release artifacts and publish them to GitHub Releases.
- Release artifacts: CLI binaries for each OS/arch; desktop installers (`.dmg`, `.exe`/`.msi`, `.AppImage` or `.deb`) with checksums.
- The README must explain the unsigned-app warnings until signing is set up, with the current steps for each supported macOS version (Gatekeeper's "Open Anyway" in System Settings → Privacy & Security) and Windows SmartScreen ("More info" → "Run anyway").
- Hosting uses free services only, under the author's personal GitHub account (`Om-jannu`), with no GitHub org or paid domain:
  - Repository: `github.com/Om-jannu/tunwarden`. One repository holds the Rust crates, the desktop app (`apps/desktop`) and the website (`apps/web`).
  - Binaries and installers: GitHub Releases. Download links use `releases/latest/download/<file>`, so the website needs no change per release.
  - Website: a Next.js static export on Vercel's free tier at `tunwarden.vercel.app`, with a landing page, a download page that detects the OS, and the docs.
- Release tooling: `dist` (cargo-dist) builds every OS from one version tag and generates the installers.
- Channels are added in stages, not all at once. The first release (v0.1.0) is published only on GitHub Releases, with the shell and PowerShell install scripts. The other channels below are prepared in the repository and switched on one at a time after that. Planned CLI channels:
  - GitHub Releases: archives for macOS (Apple Silicon, Intel), Linux (x86-64, arm64; statically linked with musl, so they run on any distribution) and Windows (x86-64), with checksums;
  - shell and PowerShell install scripts;
  - a Homebrew tap in the author's shared tap repository (`Om-jannu/homebrew-tap`), used for all of the author's projects;
  - a Scoop bucket in the author's shared bucket repository (`Om-jannu/scoop-bucket`);
  - npm as the unscoped package `tunwarden` (installed with `npm i -g tunwarden`; `npx` is not recommended, because `autostart` would record a path in npx's cache);
  - crates.io, with `cargo binstall` metadata so Rust users can install the release binary without compiling;
  - `.deb` and `.rpm` packages attached to the GitHub Release.
- CLI channels after one or two stable releases: winget (`microsoft/winget-pkgs`), AUR (`tunwarden-bin`), a Nix flake in the repository, and homebrew-core once the project is popular enough. mise can install from GitHub Releases without extra work.
- Desktop app channels (M3): installers in GitHub Releases, a cask in the author's tap, and winget. The official Homebrew cask repository has required signed and notarized apps since 1 Sep 2026, and Apple signing is paid, so the app stays out of it while unsigned. Free Windows signing through SignPath Foundation (for open-source projects) is to be applied for.
- Not used: Snap and Flathub (their sandboxes prevent running the user's ssh and cloud tools and reading `~/.ssh`), Chocolatey (covered by winget and Scoop), Docker Hub (tunnels must run on the host), and the Mac App Store.
- On macOS, a binary downloaded in a browser is quarantined and Gatekeeper blocks it while unsigned. Installs through Homebrew, the install script or npm are not, so the download page and README point macOS users to those.
- Discovery at launch: GitHub topics, lib.rs (automatic from crates.io), Show HN, r/rust, r/commandline, r/devops, This Week in Rust, AlternativeTo, and the awesome-rust and awesome-ssh lists.

## 11. Milestones

1. **M1 – Core, daemon and CLI.**
   - Config load, validation and comment-preserving edits, including the hand-edit rules.
   - Tunnel types: SSH (local, remote, dynamic, jump hosts) and command. Profiles and logins.
   - Process control on all three OSes, supervisor, readiness checks, backoff retry, credential-error detection, failure reasons, logs.
   - The daemon with its control interface, version handshake and crash recovery.
   - All CLI commands except `autostart`, plus `doctor` and the `fake-tunnel` test suite.

   Usable daily, replaces the zsh scripts.
2. **M2 – Sleep, network and autostart.** Sleep/wake and network-change handling, retrying non-credential failures after wake, `autostart`, restore after reboot. Built: wake and network changes are found by polling on every OS (design 11), a wake also checks the logins in use, and `autostart` covers launchd, systemd and the Windows `Run` key. Still to confirm on real hardware: a real sleep, a real systemd login session, and Windows.
3. **M3 – Desktop app.** Tray, dashboard, log viewer, add/edit forms, production confirmation, notifications, `doctor` panel. Built (docs/desktop.md sections 7 to 9): the app ships the CLI, starts hidden with `--background`, runs one copy, and is packaged for all three OSes; the Windows parts are still to be checked on Windows.
4. **M4 – Release pipeline and website.** Cross-platform CI, release artifacts and installers (section 10), README and docs, the website at `tunwarden.vercel.app`, public repository. The release setup may start before M2 so that test builds can be installed without building from source; the website is built later.
5. **M5 – Polish.** Package-manager listings, optional code signing, deeper health checks and latency, a library of ready-made login and tunnel templates.

## 12. Acceptance criteria (v1 = M1 through M4)

- AC-1. With two profiles configured and four tunnels, one command starts all of them, and `status` shows each as `running` within 30 seconds.
- AC-2. Killing a tunnel's process externally results in it returning to `running` automatically with the retry counter incremented and the event in the log.   
- AC-3. Simulating a login expiry (a fake tool whose check fails with a re-authentication error, or a tunnel printing one) leads to `failed` with reason `auth expired` and the login hint within 10 seconds, with no further retries. After `login`, the affected tunnels restart without extra commands, including tunnels in other profiles that share the same login. A check failing for another reason (no network) stops nothing.
- AC-4. Stopping a tunnel leaves no child processes behind on macOS, Linux and Windows.
- AC-5. Starting a tunnel whose local port is occupied fails immediately with reason `port conflict` and a clear message.
- AC-6. Tunnels started from the CLI appear correctly in the desktop app and vice versa. After killing the daemon or rebooting, no tunnel processes from the old daemon remain once the daemon starts again, and every tunnel's state is accurate.
- AC-7. Putting a laptop to sleep and waking it restores all previously running tunnels within a minute.
- AC-8. Two tunnels in different profiles run at once, each with its own environment (for example its own gcloud project), without touching any tool's global config.
- AC-9. A fresh install on each OS can follow the README from download to a running tunnel without building from source.
- AC-10. The repository contains no real project IDs, hostnames, tokens or personal paths.
- AC-11. One config containing an SSH local forward, an SSH forward through a jump host, a gcloud IAP `command` tunnel and a kubectl port-forward `command` tunnel runs side by side. Each reaches `running`, each reconnects after its process is killed, and each shows a clear message when its credentials are missing or expired.
  - The SSH and kubectl parts are automated on Linux CI (Docker SSH server and jump host, `kind`).
  - The login part is automated with a fake login tool.
  - The full criterion is checked by one manual run against real GCP before each release.
- AC-12. Editing the config by hand while the daemon runs is picked up without restarting running tunnels. An invalid edit is reported with its location while the daemon keeps the last valid config. A tool edit after a hand edit keeps both changes and all comments.

## 13. Risks and mitigations

- **Orphaned child processes**, especially on Windows and after a daemon crash on macOS: use process groups, job objects and the PID records in FR-52, and test AC-4 and AC-6 on every OS.
- **A tool's login error text varies by version:** the patterns live in the user's config, so they can be fixed without a release; the example config's patterns are checked against real output (`docs/spikes.md`). The scheduled check is the fallback.
- **A login check that fails for other reasons looks expired:** without `expired` patterns, any failure counts as expired. The example config lists patterns for every login, and the docs recommend them.
- **A tunnel shows `running` while the remote side is dead:** stated as a v1 limit (FR-49) and documented; deeper checks are the planned fix.
- **GUI apps on macOS do not inherit the shell PATH:** look in known install locations, allow a configurable path per tool, and show the path used in `doctor` (FR-36). Reading the login shell's PATH can be added later if users report missing tools.
- **Windows batch-file quoting for tools such as `gcloud.cmd`:** rely on Rust's standard escaping, which refuses unsafe arguments, and cover it with tests (FR-54).
- **Webview and tray differences across OSes:** test the tray, notifications and webview on all three OSes early in M3 (WebView2 on Windows, WebKitGTK on Linux).
- **Lost edits between hand edits and tool edits:** the daemon is the only writer and checks the file hash before every write (FR-5).
- **Scope creep:** anything outside sections 3 and 7 goes to a backlog, not into v1.
- **Interactive prompts hanging a background tunnel:** run SSH with `BatchMode=yes`, detect password, passphrase and host-key failures, and report an actionable message instead of waiting.

## 14. Future ideas (backlog)

- Deeper health checks (database or HTTP pings, an end-to-end check through IAP) and end-to-end latency with history graphs. These fix the v1 liveness limit (FR-49).
- A library of ready-made login and tunnel templates (gcloud IAP, kubectl, AWS SSM, Azure Bastion, Cloudflare Tunnel, Docker or Podman port forwards) that `add` and `init` can offer, as data rather than code.
- Per-tunnel environment variables (today they come from the profile) and pre/post-start hooks.
- Export and import of config bundles with secrets removed.
- Scheduled tunnel windows (for example start at working hours).

## 15. Decisions and open questions

Decided in v2:

1. The daemon is required and is part of M1 (FR-38).
2. v1 ships only the types in 7.10: the SSH types and `command`. Tool-specific behaviour lives in the user's config (profiles, logins, patterns), not in code (v3).
3. Production confirmation is in v1 (FR-33), in M3 for the app and M1 for the CLI.
4. License: dual MIT OR Apache-2.0.
5. Name: Tunwarden (binary `tunwarden`, crates `tunwarden-core` and `tunwarden-cli`).
   - Checked on 26 Nov 2026 and free on all of these: crates.io, Homebrew formulae and casks, GitHub user/org and repo names, npm, PyPI, Docker Hub, winget, Scoop, Chocolatey, AUR, Snap, Flathub, Debian, Repology, and the domains .com, .dev, .io, .sh, .app, .net and .org.
   - The name Burrow was dropped because `hackclub/burrow` is an existing open-source tunneling tool with a `burrow` CLI, and the `burrow` crate is taken.

Open:

1. Trademark search for "Tunwarden" in the USPTO and WIPO Global Brand Database has not been done yet. Do it before publishing.
2. Claim the names before publishing, since availability can change: the crates.io crates, the npm package `tunwarden`, and the Vercel project `tunwarden` (for `tunwarden.vercel.app`). No domain or GitHub org is needed (section 10).

## 16. Appendix: reference prototype

A rough, uncompiled Rust prototype of the CLI core exists from earlier discussion. It is not yet in this repository; link it here when it is added. It is reference material only. It demonstrates the config shape, the per-environment approach (now profiles), the supervisor loop (health checks, backoff, auth detection) and the CLI command set. It is not binding on structure or naming, and the implementation should follow this document where they differ.
