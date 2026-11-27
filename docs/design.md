# Tunwarden – Technical Design (M1)

Status: Draft 2 · Date: 27 Nov 2026 · Scope: milestone M1 (core, daemon, CLI)

Draft 2 makes Tunwarden generic (PRD Draft v3): profiles and logins replace gcloud projects, and the `command` type replaces `custom`, `gcloud-iap` and `kubectl-port-forward`. Section 16 summarises the change.

This document covers how to build what the [PRD](../PRD%20Tunwarden%20–%20Tunnel%20Manager.md) asks for, and only the decisions that are expensive to change later. The PRD stays the source of truth for behavior. Where this document refines the PRD, section 13 lists the change so the PRD can be updated.

The desktop app (M3) is a thin client of the daemon protocol in section 5 and gets its own short design at the start of M3. Sleep/wake and network detection (M2) are sketched in section 11.

## 1. Workspace layout

```
tunwarden/
  Cargo.toml              workspace, shared lints and dependency versions
  rust-toolchain.toml
  crates/
    tunwarden-core/       library: everything except argument parsing and UI
    tunwarden-cli/        `tunwarden` binary: CLI commands and `tunwarden daemon run`
    fake-tunnel/          test helper binary (section 10)
  apps/desktop/           Tauri 2 + React app (M3)
  docs/
```

Modules in `tunwarden-core`:

| Module | Responsibility |
| --- | --- |
| `paths` | Per-OS config and state locations, test overrides |
| `config` | TOML model, validation with locations, comment-preserving edits, reload diff |
| `types` | The `TunnelType` trait and the built-in types |
| `process` | Spawning, process groups and job objects, stop, crash sweep |
| `probe` | Readiness from the OS socket table, port-conflict lookup |
| `supervisor` | Per-tunnel state machine and its runtime task |
| `auth` | Running a login's `check` and classifying the result |
| `logs` | Per-tunnel log files with rotation, activity log |
| `state` | `state.json` store with atomic writes |
| `proto` | Daemon protocol messages, shared by the daemon, CLI and app |
| `daemon` | Control server, registry of supervisors, config watcher |
| `client` | Connect, handshake, start the daemon on demand |
| `tools` | Locating ssh and the config's commands; `doctor` checks |

`tunwarden-core` has no UI code and no `println!`. The CLI and the app format output themselves.

## 2. Main dependencies

Versions were current on 26 Nov 2026. They are pinned in the workspace `Cargo.toml`.

| Need | Crate | Notes |
| --- | --- | --- |
| Async runtime | `tokio` 1.54 | Multi-threaded runtime in the daemon, current-thread in the CLI |
| CLI | `clap` 4.6, `clap_complete` 4.6 | Derive API; completions for bash, zsh, fish, PowerShell |
| Config read | `toml` 1.2, `serde` | Errors carry byte spans, converted to line and column |
| Config edit | `toml_edit` 0.25 | `DocumentMut` keeps comments and formatting |
| Process groups / job objects | `process-wrap` 11 (tokio) | `ProcessGroup::leader()` on Unix, `JobObject` on Windows |
| Socket table | `listeners` 0.6 | Listening sockets with PIDs on macOS, Linux, Windows |
| Process table | `sysinfo` 0.40 | Parent PIDs and process start times |
| Signals | `nix` 0.32 (Unix), `windows-sys` 0.61 (Windows) | Group signals, console control events |
| Local IPC | `interprocess` 2.4 | Unix domain sockets and Windows named pipes, tokio support |
| File watching | `notify` 9, `notify-debouncer-full` 0.8 | Watches the config directory |
| Command splitting | `shlex` 2 | `command` tunnels and login commands (FR-8) |
| Tool lookup | `which` | PATH lookup, including `.cmd` on Windows |
| Locks | `fs4` | Single-daemon lock file |
| Atomic writes | `tempfile` | Write to a temp file in the same directory, then rename |
| Patterns | `regex` | Credential and error patterns |
| Hash | `sha2` | Config file hash for the hand-edit check (FR-5) |
| Diagnostics | `tracing`, `tracing-subscriber` | Daemon's own log, not tunnel logs |
| Errors | `thiserror` in core, `anyhow` in the CLI | |
| Output | `tabled`, `serde_json` | Tables and `--json` |
| Tests | `assert_cmd`, `insta`, `rstest`, `tempfile` | |

Per-tunnel logs use a small rotator written in core (section 8), because `tracing-appender` rotates by time, not size.

## 3. Config

### 3.1 Model

The file is parsed twice from the same bytes:

1. `toml::from_str` into the raw model below. A type error stops here, with the exact position from the error's span.
2. Semantic checks then collect every problem. Each error names a field path, and a `toml_edit::Document` parsed from the same text turns the path into a line and column.

For edits, a `toml_edit::DocumentMut` is kept in memory (section 3.3).

```rust
struct ConfigFile {
    version: u32,                       // FR-50
    #[serde(default)] settings: Settings,
    #[serde(default)] tools: ToolPaths, // `ssh` path, `path` = extra search directories
    #[serde(default)] logins: Vec<Login>,
    #[serde(default)] profiles: Vec<Profile>,
    #[serde(default)] tunnels: Vec<RawTunnel>,
}

struct Login {
    name: String,
    check: Option<String>,     // command line; exit 0 while the login is valid
    login: Option<String>,     // interactive command line
    expired: Vec<String>,      // regexes, ignoring case
    env: BTreeMap<String, String>,
}

struct Profile {
    name: String,
    env: BTreeMap<String, String>,
    production: bool,
    login: Option<String>,
}

struct RawTunnel {
    name: Spanned<String>,
    #[serde(default = "command")] r#type: Spanned<String>,
    profile: Option<Spanned<String>>,
    #[serde(default)] groups: Vec<String>,
    #[serde(default)] forwards: Vec<Spanned<Forward>>,
    auto_start: bool, auto_restart: bool, max_retries: u32, health_interval_secs: u32,
    // No `#[serde(flatten)]`: it would lose error positions. The type's own fields are
    // the tunnel table's remaining keys, taken from a second parse into `toml::Table`.
}

struct Forward {
    #[serde(default = "loopback")] bind: IpAddr, // 127.0.0.1 (FR-3, 7.10)
    local_port: Option<u16>,
    remote_host: Option<String>,
    remote_port: Option<u16>,
    local_host: Option<String>,               // ssh-remote target
}
```

Common fields are parsed by core. Everything else in a tunnel table goes to the type, which parses it into its own settings struct with `deny_unknown_fields`. This keeps each type self-contained (FR-39): a misspelled field is reported by the type that owns it.

### 3.2 Validation

`config::load(path) -> Result<Config, Vec<ConfigError>>` collects all errors instead of stopping at the first. Each `ConfigError` has a file, line, column, field path (`tunnels[2].forwards[0].local_port`) and a message with the fix.

Validation order:

1. TOML syntax and common fields.
2. Names: unique, `[A-Za-z0-9._-]+`.
3. Profile references from tunnels, login references from profiles, and login commands and patterns.
4. Each type validates its settings and returns its **listen addresses**, the `(bind, port)` pairs it will open locally. `ssh-remote` returns none.
5. Listen addresses are unique across the whole config (FR-4).
6. Schema version: newer than supported is refused; older is migrated in memory and rewritten only on the next tool edit.

The result is a `Config` of resolved tunnels. Each resolved tunnel holds its type, a `LaunchPlan` (section 4) and its readiness rule. Two resolved tunnels compare equal when their launch plan, readiness rule and retry settings are equal. The reload diff uses that equality (3.4).

### 3.3 Edits and the hand-edit rule (FR-5)

Only the daemon writes the file. It keeps `loaded_hash`, the SHA-256 of the bytes it last loaded.

Edit request (`config.add`, `config.edit`, `config.remove`):

1. Read the file. If its hash differs from `loaded_hash`, run a full reload first (3.4).
2. Apply the edit to a clone of the `DocumentMut`. Edits address tunnels by name, not by index, so a hand edit that reordered tunnels does not break them.
3. Render the document and validate the result as if loading it. If it is invalid, refuse and return the errors. The file is not touched.
4. Write atomically: temp file in the same directory, `fsync`, rename over the original, keeping the original file's permissions.
5. Set `loaded_hash` to the new content's hash.

If step 2 finds that the tunnel no longer exists, or a new name is already taken, the edit is refused with a message saying the file changed by hand.

### 3.4 Watching and reload

The daemon watches the config file's **directory**, because many editors save by writing a new file and renaming it. Events are debounced (300 ms). On an event, the daemon reads the file and compares the hash with `loaded_hash`. Equal means the event came from its own write and is ignored.

On a real change:

- If the file is invalid, keep the current config, store the errors, and report them in `status`, `doctor` and the app (FR-5).
- If it is valid, diff the old and new resolved tunnels:
  - added: create a supervisor in `stopped`. `auto_start` only applies when the daemon starts, so a new tunnel is not started by a reload;
  - removed and active: stop it, then drop it;
  - changed and active: keep running, set `pending_restart = true` ("restart to apply"); the next restart uses the new plan;
  - changed and stopped or failed: replace the plan immediately.
- Profile and login changes flow through the launch plans and patterns of their tunnels, so they need no special case.

### 3.5 As implemented (M1 step 10)

- **Detecting hand edits.** The daemon checks the config file's size and modification time every 500 ms instead of using a file-watching library. That is fast enough for one small file, handles editors that save by renaming, and needs no dependency. When the stamp changes, it compares the full text with the text it last loaded. It keeps the text itself, not a hash, because the file is small. Equal text, for example after its own write, is ignored.
- **Edits.** `config.add`, `config.edit` and `config.remove` take JSON fields, which `config::edit` turns into TOML on a `toml_edit::DocumentMut`. New tunnels are appended with `name`, `type`, `profile` and `groups` first and `forwards` last; forwards are written as inline tables. Editing a value keeps its trailing comment. Every edit goes through one lock:
  1. Re-read the file. If it changed by hand, reload it first.
  2. Refuse if the file is invalid (fix it first).
  3. Apply the edit to the document and validate the whole result as a config.
  4. Write atomically and switch to the new config.

  A failed validation writes nothing and returns every error with its location.
- **Reload rules** (`apply_config`):
  - an added tunnel gets a stopped supervisor;
  - a removed tunnel is shut down and forgotten in `state.json`;
  - a changed tunnel gets `Supervisor::update`. "Changed" means its own fields, its profile's or login's fields, or the `[tools]` settings.

  Known login results are kept for logins that still exist.
- **CLI.** `tunwarden add NAME`, `edit NAME` and `remove NAME` take `--type`, `--profile`, `--group` (repeat), `--forward` (repeat), `--command`, and `--set KEY=VALUE` for any other field. `edit` also takes `--unset KEY`.
  - A `--set` value is read as TOML if it parses (`22`, `true`, `["a", "b"]`) and as a string otherwise.
  - A `--forward` is `LOCAL`, `LOCAL:REMOTE_PORT`, `LOCAL:HOST:REMOTE_PORT`, or `key=value,...`.
  - When editing, `--group` and `--forward` replace the whole list.
- **Profiles and logins** are added with `config.add_entry` (`table`, `name`, `fields`), which `import` uses. Editing them is done in the file for now.

## 4. Tunnel types

```rust
pub trait TunnelType: Send + Sync + 'static {
    type Settings: DeserializeOwned + Debug + PartialEq + Send + Sync;

    fn id(&self) -> &'static str;                       // "ssh-local"
    fn prerequisites(&self) -> &'static [Tool];         // FR-46

    fn validate(&self, s: &Self::Settings, forwards: &[Forward])
        -> Result<Vec<SocketAddr>, Vec<FieldError>>;    // listen addresses

    fn launch(&self, s: &Self::Settings, ctx: &LaunchContext)
        -> Result<LaunchPlan, BuildError>;              // FR-40, FR-41

    fn readiness(&self, s: &Self::Settings, listen: &[SocketAddr]) -> Readiness;

    fn output_patterns(&self) -> &'static [OutputPattern]; // FR-45
}

pub struct LaunchPlan {
    pub tool: Tool,              // resolved to a path at spawn time (section 6.4)
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,   // login env, then profile env (FR-7)
}

pub enum Readiness {
    Listening(Vec<SocketAddr>),           // every address must be listening
    AliveFor(Duration),                   // ssh-remote, `command` without forwards
}

pub struct OutputPattern {
    pub regex: &'static str,
    pub reason: FailureReason,            // AuthExpired, Credentials, HostKey, ...
    pub hint: String,                     // "run `tunwarden login google`"
}
```

A `command` tunnel adds its `credential_patterns` to the type's list at load time, and every tunnel with a profile adds its login's `expired` patterns (reason `auth expired`).

Each type is one module implementing this trait. An adapter erases `Settings` so the registry can hold `Box<dyn ErasedType>`. Adding a type means one new module and one line in the registry; the supervisor, probe, logs, state and protocol do not change (FR-39).

`launch_plan(profile, login)` adds the environment in one shared place, so every type gets the same variables (FR-7, FR-47): the login's `env`, then the profile's, the profile winning on a clash. The login's variables are included so the tunnel runs as the account the check verifies.

**As implemented (M1 step 2).** The built-in types are a closed set, so they are an enum, `TypeSettings`, with one variant per type. Each variant's struct implements a `TypeDef` trait with the four parts above. The supervisor, logs and state only use `LaunchPlan`, `Readiness` and `OutputPattern`, so FR-39 still holds. Adding a type means one module, one enum variant and one line in `parse_settings`. Type fields are read with a small `Fields` helper instead of serde, so every error points at the exact field. Further rules:

- **Values passed to ssh** (`host`, `user`, `jump`) must not be empty, start with `-`, or contain whitespace or control characters. This stops a config value from adding options such as `-oProxyCommand=...`.
- **Forward fields per type.** Each type rejects forward fields it does not use. A `command` tunnel's forwards only need `local_port` (and `bind`); they are what readiness checks.
- **No tool-specific environment.** Tunwarden adds no variables of its own (Draft 1 added `CLOUDSDK_CORE_DISABLE_PROMPTS`). stdin is null, so a tool that prompts fails instead of hanging; users can add such variables to a profile.
- **A `command` tunnel that runs `ssh`** is located like the SSH types (6.4) and gets ssh's output patterns. Any other program is looked up as a command.
- **Windows caveat for `command`.** Command lines are split with POSIX rules, so a backslash outside quotes is an escape. Windows paths must be quoted, for example `'C:\tools\x.exe'`. To be revisited during the Windows spikes.

**SSH arguments.** All SSH types share one builder that always adds:

```
-N -T
-o BatchMode=yes
-o ControlMaster=no -o ControlPath=none
-o ExitOnForwardFailure=yes
-o ServerAliveInterval=15 -o ServerAliveCountMax=3
[-J jump1,jump2]
```

User options from `~/.ssh/config` still apply because ssh reads it as usual. Options given on the command line take precedence over the file, which is why these defaults are passed explicitly.

**Command display.** `show-command` renders the plan with shell quoting for the current OS. The rendering is for display and copying only; the daemon always spawns from the argument list.

**Tests.** Each built-in type has `insta` snapshot tests of its launch plan for typical settings, and a fixture corpus of real error output (`tests/fixtures/output/<type>/*.txt`) that its patterns must classify correctly.

## 5. Daemon and control protocol

### 5.1 Process model

`tunwarden daemon run` is the daemon. It is the same binary as the CLI.

- **Single instance.** The daemon takes an exclusive lock on `<state>/daemon.lock` (`fs4`) and holds it for its lifetime. Only the lock holder may remove a stale socket file. A second daemon exits immediately with "already running".
- **Start on demand.** When a client cannot connect, it starts `current_exe() daemon run` detached and retries for up to 5 seconds. Detached means `setsid` on Unix, and `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW` on Windows. Standard streams go to the daemon log.
- **Runtime.** Multi-threaded tokio. One `Registry` task owns the config, the supervisors map and the auth watcher, and receives requests through a channel. Each tunnel has one supervisor task (section 7). There are no shared mutable maps; tasks talk through channels.
- **Shutdown.** The daemon stops every tunnel (FR-12) before exiting. What happens on the next start depends on how it was stopped (FR-15):
  - `daemon stop`: every tunnel's `desired` is set to `stopped`, so nothing is restored.
  - `daemon stop --keep`, `daemon restart`, an upgrade, or SIGTERM from the OS at logout or shutdown: `desired` stays as it was, so running tunnels are restored.
  - A crash or power loss leaves `desired` as it was, so running tunnels are restored.

### 5.2 Transport

- macOS and Linux: Unix domain socket at `<state>/tunwarden.sock`. The state directory is mode `0700` and the socket `0600`. Socket paths are limited to about 104 bytes on macOS; if the path is longer, the socket moves to a short per-user directory under the system temp directory, created with mode `0700`, and its location is recorded in `<state>/socket-path`.
- Windows: named pipe `\\.\pipe\tunwarden-<user SID>` with a security descriptor that grants access only to the current user. Spike S3 confirms `interprocess` supports this; otherwise the pipe is created with `windows-sys` directly.

### 5.3 Messages

A small subset of JSON-RPC 2.0, one JSON object per line (JSON Lines), UTF-8.

Handshake, the first exchange on every connection (FR-53):

```json
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol":1,"client":"cli","version":"0.1.0"}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"daemon_version":"0.1.0"}}
```

If the protocol numbers differ, the daemon answers with error code `protocol_mismatch` and both versions. The client then suggests `tunwarden daemon restart`. The protocol number only changes on incompatible changes; adding optional fields does not change it.

Requests and responses use JSON-RPC ids. Errors have a stable string `code`, a `message` and a `hint`:

```json
← {"jsonrpc":"2.0","id":7,"error":{"code":-32000,"message":"Port 5433 is in use by postgres (PID 812)","data":{"code":"port_conflict","hint":"Stop that process or change local_port for staging-db"}}}
```

Events are JSON-RPC notifications (no id), sent only after `subscribe`:

```json
→ {"jsonrpc":"2.0","id":9,"method":"subscribe","params":{"topics":["tunnels","activity","log:staging-db"]}}
← {"jsonrpc":"2.0","method":"tunnel.state","params":{"name":"staging-db","state":"running","since":"2026-11-26T09:12:03Z","restarts":1}}
```

Methods for M1:

| Method | Purpose |
| --- | --- |
| `hello` | Handshake |
| `status`, `list` | Snapshot of tunnels; also the `--json` output schema |
| `start`, `stop`, `restart` | Selector: names, group, profile or all |
| `config.get`, `config.add`, `config.edit`, `config.remove` | Config access and tunnel edits (section 3.3) |
| `config.add_entry` | Add a profile or login (`table`, `name`, `fields`) |
| `config.errors` | Current invalid-config errors, if any |
| `logs.tail` | Last N lines; following uses `subscribe` with `log:<name>` |
| `auth.status`, `auth.check` | Login results, on-demand check (`name`: a login or profile, or all) |
| `auth.recovered` | Client reports a finished login; daemon re-checks (section 9) |
| `doctor` | Daemon-side checks (tools as the daemon sees them, ports, config, auth) |
| `subscribe` | Event topics |
| `daemon.shutdown` | Stop all tunnels and exit; `keep: true` keeps them marked for restore (5.1) |

The `proto` module defines these messages as Rust types with serde. The CLI's `--json` output prints the `result` objects unchanged, so the JSON output and the protocol share one schema.

### 5.4 As implemented (M1 step 7)

- **Modules.** `proto` holds the messages and `ipc` the transport. `daemon::run` is the daemon, and `client::Client` with `client::Launcher` is the client. The CLI so far has `tunwarden daemon run|start|stop [--keep]|restart|status`; the other commands follow in step 8.
- **Transport.** tokio's own `UnixListener` and Windows named pipes are used instead of the `interprocess` crate, which saves a dependency.
  - The socket is `<state>/tunwarden.sock`, mode 0600. If that path is longer than 100 bytes, it falls back to `$TMPDIR/tunwarden-<uid>/<hash>.sock`. Client and daemon compute the same path, so no extra file records it.
  - The Windows pipe is `\\.\pipe\tunwarden-<hash of the state dir>` and refuses remote clients. A security descriptor limited to the current user still waits for spike S3.
- **Single instance.** The daemon uses `std::fs::File::try_lock` on `daemon.lock` (stable Rust), not `fs4`. Only the lock holder removes a stale socket.
- **Crash detection.** The daemon sets `state.json`'s `daemon` field at start and clears it on a clean shutdown. A non-empty field at the next start means the previous daemon crashed. The new daemon then:
  - sweeps leftover processes;
  - adds a daemon note and a per-tunnel note to `status`;
  - restores tunnels that were running if `restore_on_start` is on.

  After a clean `stop --keep`, tunnels are restored regardless of `restore_on_start`, because the user asked for it.
- **Signals.** SIGTERM and SIGINT stop the daemon like `stop --keep`. SIGHUP is ignored. The launcher starts the daemon in its own process group (no `setsid`, which would need `unsafe`), with stdin null and stdout and stderr appended to `<state>/daemon.log`.
- **Config at start.** A missing or invalid config does not stop the daemon. It runs with no tunnels and reports the errors in `status`. Reloading the file comes in step 10.
- **Startup timeout.** The client waits up to 10 seconds for a newly started daemon. If the daemon exits early, the error includes the last lines of `daemon.log`.

### 5.5 CLI (M1 step 8)

- **Layout.** The CLI lives in the `tunwarden_cli` library, and `main.rs` only calls `tunwarden_cli::run`. The test package builds the same code as `tw-test-cli`, so the system tests run the real command line next to `fake-tunnel`.
- **No daemon needed.** `init`, `config-path`, `list`, `show-command` and `completions` read the config file directly. `status` without a daemon shows every tunnel as stopped. `logs -n` without a daemon reads the log file, and `stop` without a daemon is a no-op.
- **Waiting.** `start`, `restart` and `stop` start the daemon on demand if needed (not `stop`). They then wait up to `--wait` seconds (default 30) for the selected tunnels to settle, and print one row per tunnel. The exit code is non-zero if any tunnel did not reach the goal. `--wait 0` returns at once.
- **Production confirmation (FR-33).** Before `start` or `restart`, the CLI sends the selector with `dry_run: true` to learn which tunnels it picks. If any is in a production profile, it asks for confirmation on a terminal. Without a terminal it refuses with the hint `--yes`. Nothing starts until the answer is yes.
- **`logs -f`** subscribes before reading the tail, then skips lines already shown or older than the newest one shown, so nothing is missed or printed twice.
- **Exit codes.** 0 on success, 1 on any failure, 2 for usage errors (clap), and 3 from `daemon status` when the daemon is not running.

### 5.6 doctor, import and trust (M1 step 11)

- **`doctor`** reports five sections: Config, Daemon, Tools, Logins and Ports. Each line is `ok`, `warning` or `problem`, and the exit code is non-zero if any line is a problem.
  - **Tools.** When the daemon runs, the tool check uses the daemon's view (method `doctor`), because the daemon starts the tunnels. If the shell finds a tool that the daemon cannot, the hint gives the exact `[tools]` line to add. This is the macOS PATH case (FR-36). If both find it in different places, a warning says so.
  - **Logins.** Checked through the daemon (`auth.check`) or directly.
  - **Ports.** Ports of tunnels that are not running must be free; a taken one names the holder.
- **`import [FILE] [--write]`** (`import` module) reads commands, `alias name='...'` lines, `\` continuations and comments.
  - **SSH types.** `ssh` with only `-L`, `-R` or `-D`, plus `-J`, `-l`, `-p` and harmless flags, becomes the matching ssh type. Anything ssh-specific the types cannot express (remote commands, other `-o` options, mixed forward kinds) keeps it a `command` tunnel, with a note.
  - **Everything else becomes `command`** with the original command. Local ports are guessed from `-L` and `-D`, any `--*port*=N` option, `localhost:N` or `127.0.0.1:N`, and `N:M` pairs, with a warning to check them.
  - **Removed parts.** `ssh -f` and a trailing `&` are removed with a warning.
  - **Environment prefixes.** Leading `NAME=value` assignments become a profile named `imported` (then `imported-2`, ...) with that `env`. An existing profile with exactly the same `env` is reused.
  - **Names** come from the alias, or from the host or program plus the local port. They are sanitized and made unique against the config.
  - **Writing.** Without `--write` it prints TOML for review. `--write` adds profiles (`config.add_entry`), then tunnels, through the daemon.
- **`trust HOST|TUNNEL`** runs the user's ssh twice with `PreferredAuthentications=none`, so no password or passphrase is ever asked for. The first run, with `StrictHostKeyChecking=ask`, lets ssh show the fingerprint and ask on the terminal. The second run, with `BatchMode=yes`, confirms the key is trusted, and reports a changed key with the `ssh-keygen -R` hint, or an unreachable host. A tunnel name uses that tunnel's user, port and jump hosts. Tunwarden never edits `known_hosts`.

## 6. Process control

### 6.1 Spawn

Every tunnel is spawned through `process-wrap` (tokio) with:

- stdin set to null, so nothing can wait for input;
- stdout and stderr piped to the supervisor;
- Unix: `ProcessGroup::leader()`, so the tunnel and its children share a new process group;
- Windows: `JobObject` plus `CreationFlags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)`, and the job object set to kill its processes when the job handle closes (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). If `process-wrap` does not set that limit, core sets it on the job handle;
- No parent-death signal on Linux: spike S4 showed it kills only the direct child and leaves the grandchildren (6.3).

After spawning, the supervisor writes the PID, process group ID and process start time to `state.json` (section 8.2).

**As implemented (M1 step 3).** `process::TunnelProcess` wraps `process-wrap`. On Windows it adds `KillOnDrop`, which is what sets `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Unix does not use `KillOnDrop`, so a dropped handle behaves like a crashed daemon and the sweep must clean up. The tests rely on that. The PID and start time are recorded right after spawn.

### 6.2 Stop (FR-12)

- Unix: `SIGTERM` to the process group, wait up to 5 seconds, then `SIGKILL` to the group.
- Windows: `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)`, wait up to 5 seconds, then terminate the job object. `CTRL_BREAK_EVENT` only reaches processes that share the caller's console, and a detached daemon has none. Spike S1 decides between giving the daemon a hidden console that children inherit, or forced termination only. ssh, gcloud and kubectl hold no state that needs a graceful exit, so forced termination is acceptable if the spike fails.

A tunnel is `stopped` only after the whole group or job is gone. A stop also works after the command has exited, and removes anything it left behind. On Unix, the leader is reaped while waiting, because an unreaped zombie still counts as a group member.

Until spike S1 runs, the Windows stop terminates the job object directly.

### 6.3 Crash sweep (FR-52)

Nothing cleans up tunnel processes when the daemon dies on macOS or Linux (spike S4). The daemon sweeps on its next start:

1. On start, after taking the lock, read `state.json`.
2. For each recorded tunnel, decide from the leader PID, its recorded start time and its process group:

   | Leader | Group | Action |
   | --- | --- | --- |
   | alive, start time matches | – | kill the group |
   | alive, start time differs | – | PID was reused: leave it alone |
   | gone | still has members | kill the group |
   | gone | empty | nothing to do |

   The third row is safe because POSIX never reuses a process ID while a process group with that ID still exists. It covers tunnels whose leader exited while grandchildren kept running, which a PID check alone misses.
3. Kill as in 6.2: SIGTERM to the group, then SIGKILL.
4. Then restore tunnels according to FR-15.

Process start times have one-second resolution, so a PID reused within the same second could slip past the guard. That is accepted. Tagging processes with an environment variable is not an option, because macOS does not expose other processes' environments.

On Windows, the job object kills everything when the daemon dies. The sweep handles anything left over by checking the recorded PID and start time, as in the first two rows.

**Known gap:** a process that leaves its group escapes both the sweep and a normal stop. The main case was an SSH ControlMaster forked by `ControlPersist`, which SSH types now prevent by turning connection sharing off (section 15). A `command` tunnel can still do it.

### 6.4 Locating tools

The daemon is often started by the desktop app or at login, and on macOS those processes do not get the shell's PATH. Programs are `ssh` (for the SSH types and `command` tunnels that run ssh) or a command name (the first word of a `command` tunnel or a login's `check` or `login`). Resolution order:

1. `ssh`: `tools.ssh` if set; a missing file there is an error, not a fallback. A command given as a path (it contains `/` or `\`, `~` expanded) is used as is.
2. `PATH` of the daemon process (which also finds `.cmd` and `.exe` on Windows).
3. The directories in `tools.path`, in order.
4. Known install locations per OS with no vendor-specific folders: `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `~/.local/bin` on macOS; `/usr/local/bin`, `/usr/bin`, `/snap/bin`, `~/.local/bin` on Linux. Vendor folders such as `~/google-cloud-sdk/bin` go in `tools.path`.

The resolved path and its source appear in `doctor`, which checks every program the config uses, including login commands. A missing program fails the tunnel with reason `missing tool` and says to install it, use its full path, or add its directory to `tools.path` (FR-46).

### 6.5 Batch-file tools on Windows (FR-54)

Rust's standard library runs `.cmd` and `.bat` programs through `cmd.exe` and escapes arguments for it. Since Rust 1.77 it returns an error for arguments it cannot escape safely instead of passing them through. Core relies on that, maps the error to a config error naming the field, and covers it with tests that run a fake `.cmd` tool (section 10). gcloud is the common case (`gcloud.cmd`).

## 7. Supervisor

### 7.1 Structure

The state machine is a pure function, separate from the task that runs it:

```rust
fn step(m: &Machine, event: Event, now: Instant) -> (Machine, Vec<Action>);

enum Event {
    Start, Stop, Restart,
    Spawned(ProcessInfo), SpawnFailed(SpawnError),
    Exited(ExitStatus),
    ProbeResult(ProbeOutcome),
    OutputMatched(FailureReason, String),
    Timer(TimerKind),          // ProbeDue, StartupGrace, Backoff, StableReset, StopTimeout
    AuthExpired, AuthRecovered,
    Wake,                      // M2
}

enum Action {
    Spawn, Terminate, ForceKill,
    Probe, SetTimer(TimerKind, Duration), CancelTimer(TimerKind),
    Persist, Emit(TunnelEvent), Log(String),
}
```

`Machine` holds the state from section 8 of the PRD, the retry counter, the failure reason and the last error. The runtime task owns the child process, the output readers and the timers, executes actions, and feeds results back as events.

This split makes every rule in PRD sections 7.3, 7.4 and 8 testable as a plain unit test (`step` with a fake clock), with no processes and no sleeping. The runtime is tested separately with `fake-tunnel` and `tokio::time::pause`.

### 7.2 Rules mapped from the PRD

| Rule | Implementation |
| --- | --- |
| Port conflict (FR-13) | Before `Spawn`, the probe looks up each listen address; if any is taken, `failed(port conflict)` with the holder's PID and name |
| Missing tool (FR-46) | Resolved before `Spawn`; failure gives `failed(missing tool)` |
| Probe cadence (FR-16) | `ProbeDue` every 1 s in `starting`, every `health_interval_secs` in `running` and `unhealthy` |
| Startup grace (FR-17) | `StartupGrace` 25 s; firing in `starting` gives `reconnecting` |
| Unhealthy (FR-17) | 1 failed probe in `running` gives `unhealthy`; 3 consecutive gives `reconnecting`; a pass gives `running` |
| Backoff (FR-18) | `min(2^n, 60)` seconds; `StableReset` 60 s after `running` resets n |
| Retries exhausted | n reaches `max_retries`: `failed(retries exhausted)` |
| Output patterns (FR-24, FR-45) | `OutputMatched` in any active state: terminate, `failed(reason)`, no retry |
| Login expiry from the scheduled check | `AuthExpired`: terminate, `failed(auth expired)` (section 9) |
| Login recovery (FR-25) | `AuthRecovered` in `failed(auth expired)`: `starting` with a fresh retry counter |
| Restart to apply (FR-5) | `pending_restart` is cleared on the next spawn, which uses the new plan |
| Auto-restart off | Exit or unhealthy goes to `failed(retries exhausted)` without retrying |

Every transition produces `Persist`, `Emit` and `Log` actions, so state, events and logs stay consistent (PRD section 9, Observability).

### As implemented (M1 step 5)

`supervisor::Machine::step(event) -> Vec<Action>` is the pure state machine. 28 unit tests cover it. `supervisor::Supervisor` is the runtime task. Details that differ from, or add to, 7.1 and 7.2:

- **Exits are found by polling** `try_wait` every 100 ms, not by awaiting `wait()`. `process-wrap`'s Unix `wait()` reaps inside `spawn_blocking`, so a cancelled `select!` branch would lose the exit status.
- **Output first, then the exit.** When the command exits, the runtime waits up to 300 ms for its output readers. It classifies output printed just before the exit (for example gcloud's auth error) before it handles the exit, so an auth failure never turns into a retry.
- **Stale output is ignored.** Output events carry a process generation, and events from an old process are dropped.
- **Every exit ends with a full terminate** of the process group, so anything the command left behind is removed before a retry.
- **Pre-spawn checks.** The runtime locates the tool (`tools::resolve`, design 6.4; failure gives `missing tool`). It then looks up each listen address in a fresh socket snapshot. A taken address gives `port conflict` with the holder's name and PID, and nothing is started (FR-13).
- **The environment is added, never cleared.** A tunnel inherits the daemon's environment plus its login's and profile's `env` (FR-7). Draft 1 removed gcloud's variables first; that was tool-specific and is gone. A variable that must not leak from the daemon's environment has to be set in the profile.
- **Dropping a `TunnelProcess` kills its group** (Unix; Windows uses the job object's kill-on-close). A panic or an aborted task therefore cannot leave a tunnel running. A daemon killed with SIGKILL runs no destructors, so the crash sweep (6.3) still applies.
- **Retry rules.** A `spawn error` is retried with backoff. All other failure reasons need the user. After a wake, a tunnel that failed with `retries exhausted` or `spawn error` gets a fresh set of retries; credential failures do not (FR-19).
- **Restart to apply.** `Supervisor::update(spec)` takes effect at once if no process is running. Otherwise it is held as `pending_restart` until the next spawn (FR-5).

### 7.3 Output handling

Two reader tasks read stdout and stderr line by line (invalid UTF-8 is replaced, very long lines are cut at 8 KB). Each line is written to the tunnel log and checked against the tunnel's output patterns. Matching only runs on new lines, so it costs nothing while a tunnel is idle.

## 8. Probe, state and logs

### 8.1 Readiness probe (FR-16)

The probe never connects to the port. It reads the OS socket table with `listeners` and the process table with `sysinfo`:

1. Collect the PIDs of the tunnel's process tree (its process group on Unix, its job object on Windows, or descendants of the root PID).
2. The tunnel is ready when every listen address has a `LISTEN` socket owned by one of those PIDs. A socket on `0.0.0.0` or `::` counts for any address with the same port.

One shared `Prober` takes a snapshot on request and caches it for 500 ms, so many tunnels probing at once cost one system call round.

There is no fallback to "someone is listening". Spike S2 showed that such a fallback would count a port held by an unrelated process as ready, which is exactly the port-conflict case. If ownership cannot be read, readiness fails and `doctor` reports why. S2 confirmed ownership on macOS for ssh, kubectl, wrapped commands and Python. gcloud runs as a single Python process, because its launcher script uses `exec`. Linux and Windows are still to be confirmed (`docs/spikes.md`).

For `gcloud compute start-iap-tunnel`, a listening port is a stronger signal than for most commands: gcloud opens the local port only after a test connection through IAP succeeds.

**As implemented (M1 step 4).** `probe::Snapshot` holds the listening TCP sockets with their owner PIDs, plus a PID-to-parent map from `sysinfo`. Further rules:

- **The tunnel's processes** are its command and all descendants, found through parent PIDs. The process group is not used, because `sysinfo` does not expose it. A command that has exited is handled by the supervisor, not the probe.
- **A check has three results.** `Yes`, `Waiting(missing addresses)`, or `HeldByOther(address, holder)`. The third means a port the tunnel wants is held by a process outside the tunnel. The supervisor reports that as a port conflict instead of waiting out the startup grace.
- **Shared sockets.** One socket can have several owners, for example after a child inherits it. Sockets are sorted by PID, so the reported holder is stable, and usually the process that created the socket.
- **Before a spawn**, the supervisor uses `fresh_snapshot()`, which skips the 500 ms cache, so a port taken a moment ago is not missed.

### 8.2 State store

`<state>/state.json`, written atomically on every transition:

```json
{
  "version": 1,
  "daemon": { "pid": 4120, "started_at": "2026-11-26T09:00:00Z", "protocol": 1 },
  "tunnels": {
    "staging-db": {
      "desired": "running",
      "state": "running",
      "reason": null,
      "process": { "pid": 4188, "pgid": 4188, "start_time": 1795600803 },
      "restarts": 1,
      "since": "2026-11-26T09:12:03Z",
      "last_error": null
    }
  }
}
```

`desired` is what the user asked for. It is `running` after a start and `stopped` after a user stop or a plain `daemon stop` (5.1). Restore (FR-15) starts the tunnels whose `desired` is `running`. The file is only read at daemon start; at runtime the supervisors are the source of truth.

### 8.3 Logs (FR-27, FR-28, FR-30)

- `<state>/logs/<tunnel>.log`, mode `0600`. Each line: `2026-11-26T09:12:03.120Z out|err|tw <text>`, where `tw` marks Tunwarden's own events.
- Rotation: when the file passes 5 MB, rename it to `<tunnel>.log.1` (replacing an older one) and start a new file.
- Activity: `<state>/activity.jsonl` with the same rotation, plus the last 500 events in memory for `subscribe` and `status`.
- The daemon's own diagnostics: `<state>/daemon.log` through `tracing`.

### 8.4 As implemented (M1 step 6)

- **`persist::Persister`** is the supervisor `Observer`. It writes log lines to the tunnel's log, each state change to the activity log, and every status to `state.json`. A write error never stops a tunnel; the last one is kept for `doctor`.
- **`state::StateStore`** writes `state.json` atomically: a private temp file in the same directory, `fsync`, rename, then `fsync` of the directory. It writes while holding its lock, so writes never reorder.
  - A file that does not parse is moved to `state.json.corrupt`, and the daemon starts with no saved state and a note for `doctor`. A damaged file never blocks the daemon.
  - A file with a newer `version` is refused and left in place.
  - Failure reasons are stored as their FR-48 names, for example `"port conflict"`.
- **Logs.** `logs::Logs::tail(n)` reads backwards from the end in 64 KB chunks and continues into `<tunnel>.log.1` when the current file is short (FR-29). Line breaks inside a line are replaced by spaces, so one entry is always one line.
- **Activity.** `logs::Activity` loads the last 500 events from `activity.jsonl` at start, so the activity view survives a daemon restart (FR-30).
- **Permissions.** On Unix the state directory and `logs/` are mode 0700, and the files 0600. On Windows, `%LOCALAPPDATA%` is already private to the user; explicit ACLs are not set yet.
- **Timestamps** are RFC 3339 UTC with milliseconds, using `humantime`.

## 9. Logins

Nothing here is specific to a tool. A `[[logins]]` entry supplies the commands and patterns; the example config has entries for gcloud and AWS SSO.

- **Which tunnels use a login.** A tunnel uses the login of its profile. Profiles that name the same login share it, so one expiry fails all of their tunnels and one re-login recovers them.
- **Scheduled check.** Every `login_check_interval_minutes`, for logins with a `check` and at least one active tunnel (FR-22): run the `check` command line with the login's `env`, a 30-second timeout, **stdin and stdout set to null** and stderr captured. Null stdout means a tool that prints a token never reaches Tunwarden (FR-26). Null stdin is required, because spike S2 found gcloud can hang when stdin is a terminal.
- **Classifying the result** (`auth::classify`):
  - exit 0 is `ok`;
  - a failure whose stderr has a line matching an `expired` pattern (case-insensitive) is `expired`, with that line as the detail;
  - any other failure, a timeout or a program that cannot be run is `unknown`, which never stops tunnels, so a flaky network does not look like an expired login;
  - a login with no `expired` patterns treats every failure as `expired`. That suits tools whose check only fails when the login is gone; for others the example config lists patterns.
- **On `expired`.** Send `AuthExpired` to every active tunnel using that login. They are stopped and marked `failed(auth expired)` with "the NAME login has expired; run `tunwarden login NAME`", because a tunnel with expired credentials can still listen locally while it cannot open new connections.
- **From tunnel output.** A tunnel's output is also matched against its login's `expired` patterns (FR-24). A match fails that tunnel and makes the daemon check the login right away, at most once every 10 seconds, so the other tunnels sharing it fail too.
- **Login (FR-25).** The *client* runs the interactive `login` command with the login's `env`, so the browser and any prompts belong to the user's session:
  - The CLI runs it attached to the user's terminal and first prints which login it runs and which profiles use it.
  - The app runs the same command as a child process (M3).
  - Afterwards the client calls `auth.recovered` with the login name. The daemon runs the check again and, only if it returns `ok` (or the login has no check), sends `AuthRecovered` to that login's tunnels in `failed(auth expired)`. Without a daemon, the CLI runs the check itself.
  - A login with no `login` command gets a message to log in with the tool, then run `tunwarden auth NAME`.
- **`tunwarden auth [NAME]`** shows each login, the profiles using it, the result and the detail. NAME may be a login or a profile (meaning its login). It goes through the daemon (`auth.check`) when it runs, so the daemon acts on the result; otherwise it checks directly. The exit code is non-zero if any login has expired.
- **Open:** the example gcloud check (`config config-helper --force-auth-refresh`) has not yet been seen to fail during a real expired session (`docs/spikes.md`). The patterns are in the user's config, so a fix needs no release.

## 10. Testing

**`fake-tunnel`.** A small Rust binary that behaves according to flags. The process tests live in `crates/fake-tunnel/tests/`, because Cargo gives a binary's path (`CARGO_BIN_EXE_*`) only to tests in the same package. Readiness checks in tests connect to the port, because a second bind can succeed on macOS (`SO_REUSEADDR`).

```
fake-tunnel [--listen ADDR:PORT]... [--listen-after 2s] [--exit-after 10s] [--exit-code 1]
            [--print "TEXT"] [--print-stdout "TEXT"] [--print-after 3s] [--print-env NAME]
            [--spawn-child] [--ignore-sigterm]
```

Logins are tested with a fake login tool: a small shell script with `status` and `login` commands whose answers are controlled by marker files (expired per account, network down). It records each call with its environment, so tests can check that a check ran with the login's `env` and that its token output was never read. On Windows a `.cmd` wrapper around `fake-tunnel` will test the batch-file path (6.5).

**Test layers:**

| Layer | What | Runs on |
| --- | --- | --- |
| Unit | `step` rules, config validation and errors, edit and diff, backoff, pattern corpus, launch-plan snapshots | all 3 OSes |
| Process | Spawn, stop, no orphans (AC-4), crash sweep after killing the daemon (AC-6), port conflict (AC-5) | all 3 OSes |
| Daemon | Real daemon in a temp state directory with `fake-tunnel`: AC-1, AC-2, AC-3, AC-8, AC-12, handshake mismatch | all 3 OSes |
| Integration | Docker OpenSSH server and jump host, `kind` for a kubectl `command` tunnel (AC-11) | Linux CI only |
| Manual | AC-11 against real GCP (gcloud `command` tunnels and the example `google` login) before each release | developer machine |

Tests override locations with `TUNWARDEN_CONFIG` and `TUNWARDEN_STATE_DIR` and pick free ports, so they can run in parallel.

**CI.** On every push: `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` on macOS, Windows and Linux, plus a Linux-only integration job.

## 11. Later milestones (sketch)

- **Sleep/wake and network (M2).** One `platform` module emits `Wake` and `NetworkChanged` events: IOKit power notifications on macOS, logind `PrepareForSleep` over D-Bus on Linux, `WM_POWERBROADCAST` on Windows. On every OS, a clock-jump check (wall time advancing much more than monotonic time between ticks) also emits `Wake`, as a fallback. The registry forwards `Wake` to all supervisors (FR-19).
- **Autostart (M2).** `autostart enable` writes a launchd agent, a systemd user unit, or a Windows startup entry that runs `tunwarden daemon run`.
- **Desktop app (M3).** The Tauri backend embeds the `client` module and relays `subscribe` events to the React frontend. It holds no tunnel logic.

## 12. M1 build order and spikes

Spikes come first, each time-boxed to about a day, because they can change sections 5 and 6:

Results are in `docs/spikes.md`. S2 and S4 are done. S1 and S3 still need a Windows machine; until then the Windows stop is forced (6.2) and the named pipe has no user-only security descriptor (5.4).

- **S1** Windows stop: `CTRL_BREAK_EVENT` from a detached daemon, kill-on-close job objects, `gcloud.cmd` escaping.
- **S2** Socket ownership through `listeners` for ssh, gcloud and kubectl on all three OSes.
- **S3** Named pipe restricted to the current user with `interprocess`.
- **S4** Crash sweep on macOS and Linux after `kill -9` of the daemon, including grandchildren.

Then, in order, each step with its tests:

1. `paths`, config model and validation with locations.
2. `types`: trait, SSH builder, the built-in types, launch-plan snapshots, pattern corpus. (Draft 2: the three SSH types and `command`.)
3. `process` and `fake-tunnel`: spawn, stop, sweep.
4. `probe`.
5. Supervisor `step` and runtime.
6. `logs` and `state`.
7. Daemon, protocol, client, start on demand.
8. CLI: `init`, `list`, `status`, `start`, `stop`, `restart`, `logs`, `config-path`, `show-command`, `daemon`.
9. Auth watcher, `auth`, `login`.
10. Config edits and watcher: `add`, `edit`, `remove`, reload diff.
11. `doctor`, `import`, `trust`, shell completions.

All eleven steps are done (M1 complete except the Windows spikes S1 and S3 and the Windows-specific parts of 6.2 and 5.2 that depend on them).

## 13. Refinements carried into the PRD

These design decisions changed the PRD. They are in PRD Draft v2.1:

- **FR-52.** The sweep applies to macOS and Linux (6.3). Spike S4 showed Linux's parent-death signal is no help, so the PRD's "may still be used as an extra safeguard" should be removed.
- **FR-25.** The client runs the interactive login and then tells the daemon; the daemon never opens a browser itself (section 9).
- **FR-42.** `tunwarden trust HOST` runs `ssh` interactively with `StrictHostKeyChecking=ask` and the tunnel's jump hosts. ssh shows the fingerprint and writes `known_hosts`; Tunwarden never edits that file.
- **FR-22.** An `expired` auth check stops that context's active tunnels and marks them `failed(auth expired)`. An `unknown` result never stops tunnels.
- **FR-12.** On Windows, graceful stop may be replaced by forced termination, depending on spike S1.
- **FR-4.** Names must also start with a letter or digit, be at most 64 characters, avoid Windows device names, and be unique ignoring case. All are needed because names become file names.

## 14. Resolved questions

1. **Login shell PATH on macOS: not in v1.** Tools are found through `[tools]` paths, the daemon's PATH and known install locations (6.4). `doctor` shows the path used and its source. Reading the login shell's PATH runs the user's profile, which can be slow or print output, so it waits until users report missing tools. (PRD FR-36.)
2. **Restore after `daemon stop`: no.** A plain `daemon stop` means "stop everything" and nothing is restored. `daemon stop --keep`, `daemon restart`, upgrades, OS logout or shutdown, and crashes restore running tunnels (5.1). (PRD FR-15.)

## 15. Decisions from the spikes

1. **SSH connection sharing is off.** SSH types always pass `-o ControlMaster=no -o ControlPath=none` (section 4, PRD FR-42). This closes the known gap in 6.3 for SSH: a user's `ControlMaster auto` with `ControlPersist` can no longer make a tunnel attach to a connection it doesn't own, or fork a master that outlives a stop. Each tunnel pays for its own SSH handshake.

## 16. Generic refactor (Draft 2)

The user asked for no tool-specific behaviour. What changed in code:

| Before | After |
| --- | --- |
| `[[projects]]`: `gcloud_project`, `account`, `gcloud_config_dir`, `production` | `[[profiles]]`: `env`, `production`, `login` |
| Auth contexts derived from projects | `[[logins]]`: `check`, `login`, `expired`, `env`, shared by name |
| Fixed gcloud check and `gcloud auth login` | The login's own commands, `auth::check` and `auth::classify` |
| `gcloud-iap`, `kubectl-port-forward`, `custom` | `command` (the default); the SSH types stay |
| gcloud and kubectl output patterns | A login's `expired` patterns and a tunnel's `credential_patterns` |
| `[tools]` `ssh`, `gcloud`, `kubectl`; gcloud SDK folders searched | `[tools]` `ssh` and `path` |
| gcloud variables cleared, prompts disabled | Environment only added (section 7) |
| `--project`, `auth [project]`, `login PROJECT` | `--profile`, `auth [NAME]`, `login NAME` (login or profile) |
| `config.add_project` | `config.add_entry` |
| `auth_check_interval_minutes` | `login_check_interval_minutes` |

The SSH types were kept because SSH is everywhere, needs safe defaults (section 4) and is what `trust` and `import` understand. Everything else was turning one tool's knowledge into code. gcloud, kubectl and AWS tunnels now appear only in `examples/config.toml`.
