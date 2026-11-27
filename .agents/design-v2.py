"""Updates docs/design.md for generic profiles, logins and the command type."""

P = "docs/design.md"
s = open(P, encoding="utf-8").read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:100])
    s = s.replace(old, new)


def section(start, end, new):
    global s
    a = s.index(start)
    b = s.index(end, a)
    s = s[:a] + new + s[b:]


rep(
    "Status: Draft 1 · Date: 26 Nov 2026 · Scope: milestone M1 (core, daemon, CLI)",
    "Status: Draft 2 · Date: 27 Nov 2026 · Scope: milestone M1 (core, daemon, CLI)\n\nDraft 2 makes Tunwarden generic (PRD Draft v3): profiles and logins replace gcloud projects, and the `command` type replaces `custom`, `gcloud-iap` and `kubectl-port-forward`. Section 16 summarises the change.",
)

rep(
    "| `auth` | Auth contexts and the scheduled gcloud check |",
    "| `auth` | Running a login's `check` and classifying the result |",
)
rep(
    "| `tools` | Locating ssh, gcloud and kubectl; `doctor` checks |",
    "| `tools` | Locating ssh and the config's commands; `doctor` checks |",
)
rep(
    "| Command splitting | `shlex` 2 | `custom` command lines (FR-8) |",
    "| Command splitting | `shlex` 2 | `command` tunnels and login commands (FR-8) |",
)

rep(
    """    #[serde(default)] tools: ToolPaths, // optional explicit paths for ssh, gcloud, kubectl
    #[serde(default)] projects: Vec<Project>,
    #[serde(default)] tunnels: Vec<RawTunnel>,
}

struct RawTunnel {
    name: Spanned<String>,
    #[serde(default = "custom")] r#type: Spanned<String>,
    project: Option<Spanned<String>>,""",
    """    #[serde(default)] tools: ToolPaths, // `ssh` path, `path` = extra search directories
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
    profile: Option<Spanned<String>>,""",
)
rep(
    "3. Project references.",
    "3. Profile references from tunnels, login references from profiles, and login commands and patterns.",
)
rep(
    "- Project changes flow through the launch plans of their tunnels, so they need no special case.",
    "- Profile and login changes flow through the launch plans and patterns of their tunnels, so they need no special case.",
)
rep(
    "New tunnels are appended with `name`, `type`, `project` and `groups` first",
    "New tunnels are appended with `name`, `type`, `profile` and `groups` first",
)
rep(
    "\"Changed\" means its own fields, its project's fields, or the `[tools]` paths.",
    "\"Changed\" means its own fields, its profile's or login's fields, or the `[tools]` settings.",
)
rep(
    "take `--type`, `--project`, `--group` (repeat)",
    "take `--type`, `--profile`, `--group` (repeat)",
)
rep(
    "  - When editing, `--group` and `--forward` replace the whole list.\n",
    "  - When editing, `--group` and `--forward` replace the whole list.\n- **Profiles and logins** are added with `config.add_entry` (`table`, `name`, `fields`), which `import` uses. Editing them is done in the file for now.\n",
)

# Section 4
rep(
    "    fn uses_project(&self) -> ProjectUse;               // Required, Optional, Never\n",
    "",
)
rep(
    "    pub env: Vec<(OsString, OsString)>,   // project variables (FR-7)",
    "    pub env: Vec<(OsString, OsString)>,   // login env, then profile env (FR-7)",
)
rep(
    "    AliveFor(Duration),                   // ssh-remote, custom \"process alive\"",
    "    AliveFor(Duration),                   // ssh-remote, `command` without forwards",
)
rep(
    "    pub hint: &'static str,               // \"run `tunwarden login {project}`\"",
    "    pub hint: String,                     // \"run `tunwarden login google`\"",
)
rep(
    "A `custom` tunnel adds its user-supplied patterns to the type's list at load time.",
    "A `command` tunnel adds its `credential_patterns` to the type's list at load time, and every tunnel with a profile adds its login's `expired` patterns (reason `auth expired`).",
)
rep(
    "`LaunchContext` provides the tunnel name, the resolved project (if any) and the global settings. The project's environment is added in one shared function, so built-in gcloud types and `custom` tunnels with a project get exactly the same variables (FR-7, FR-47).",
    "`launch_plan(profile, login)` adds the environment in one shared place, so every type gets the same variables (FR-7, FR-47): the login's `env`, then the profile's, the profile winning on a clash. The login's variables are included so the tunnel runs as the account the check verifies.",
)
rep(
    "- **Values passed to a tool** (`host`, `user`, `jump`, `instance`, `zone`, `resource`, `context`, `namespace`) must not be empty,",
    "- **Values passed to ssh** (`host`, `user`, `jump`) must not be empty,",
)
rep(
    "- **Forward fields per type.** Each type rejects forward fields it does not use. `gcloud-iap` takes exactly one forward and only IPv4 bind addresses. `kubectl-port-forward` needs the same bind address for all forwards, because kubectl takes one `--address` for all ports.\n- **gcloud prompts are off.** Every tunnel with a project also gets `CLOUDSDK_CORE_DISABLE_PROMPTS=1`, so gcloud never waits for input.\n- **A `custom` command that runs ssh, gcloud or kubectl** is located like a built-in tool (6.4) and gets that tool's output patterns. With a project, it also gets the gcloud patterns.\n- **Windows caveat for `custom`.**",
    "- **Forward fields per type.** Each type rejects forward fields it does not use. A `command` tunnel's forwards only need `local_port` (and `bind`); they are what readiness checks.\n- **No tool-specific environment.** Tunwarden adds no variables of its own (Draft 1 added `CLOUDSDK_CORE_DISABLE_PROMPTS`). stdin is null, so a tool that prompts fails instead of hanging; users can add such variables to a profile.\n- **A `command` tunnel that runs `ssh`** is located like the SSH types (6.4) and gets ssh's output patterns. Any other program is looked up as a command.\n- **Windows caveat for `command`.**",
)

# 5.3
rep(
    "| `start`, `stop`, `restart` | Selector: names, group, project or all |",
    "| `start`, `stop`, `restart` | Selector: names, group, profile or all |",
)
rep(
    "| `config.get`, `config.add`, `config.edit`, `config.remove` | Config access and edits (section 3.3) |",
    "| `config.get`, `config.add`, `config.edit`, `config.remove` | Config access and tunnel edits (section 3.3) |\n| `config.add_entry` | Add a profile or login (`table`, `name`, `fields`) |",
)
rep(
    "| `auth.status`, `auth.check` | Auth context status, on-demand check |",
    "| `auth.status`, `auth.check` | Login results, on-demand check (`name`: a login or profile, or all) |",
)
rep(
    "If any is in a production project, it asks",
    "If any is in a production profile, it asks",
)

# 5.6 import
rep(
    """  - **Built-in types.** A command becomes a built-in type only if every part of it maps to type fields:
    - `gcloud compute start-iap-tunnel` with `--zone` and `--local-host-port` becomes `gcloud-iap`;
    - `ssh` with only `-L`, `-R` or `-D`, plus `-J`, `-l`, `-p` and harmless flags, becomes the matching ssh type;
    - `kubectl port-forward` with fixed local ports becomes `kubectl-port-forward`.
  - **Everything else becomes `custom`** with the original command. Its local ports are guessed from `-L`, `-D` and `--local-host-port`.
  - **Removed parts.** `ssh -f`, a trailing `&` and `NAME=value` prefixes are removed with a warning. `CLOUDSDK_CORE_PROJECT` and `CLOUDSDK_CORE_ACCOUNT` become a project, and an existing project with the same id is reused.
  - **Names** come from the alias, or from the host, instance or resource plus the local port. They are sanitized and made unique against the config.
  - **Writing.** Without `--write` it prints TOML for review. `--write` adds projects (`config.add_project`), then tunnels, through the daemon.""",
    """  - **SSH types.** `ssh` with only `-L`, `-R` or `-D`, plus `-J`, `-l`, `-p` and harmless flags, becomes the matching ssh type. Anything ssh-specific the types cannot express (remote commands, other `-o` options, mixed forward kinds) keeps it a `command` tunnel, with a note.
  - **Everything else becomes `command`** with the original command. Local ports are guessed from `-L` and `-D`, any `--*port*=N` option, `localhost:N` or `127.0.0.1:N`, and `N:M` pairs, with a warning to check them.
  - **Removed parts.** `ssh -f` and a trailing `&` are removed with a warning.
  - **Environment prefixes.** Leading `NAME=value` assignments become a profile named `imported` (then `imported-2`, ...) with that `env`. An existing profile with exactly the same `env` is reused.
  - **Names** come from the alias, or from the host or program plus the local port. They are sanitized and made unique against the config.
  - **Writing.** Without `--write` it prints TOML for review. `--write` adds profiles (`config.add_entry`), then tunnels, through the daemon.""",
)

# 6
rep(
    "ssh, gcloud and kubectl hold no state that needs a graceful exit, so forced termination is acceptable if the spike fails.",
    "ssh, gcloud and kubectl hold no state that needs a graceful exit, so forced termination is acceptable if the spike fails.",
)
rep(
    "A `custom` command can still do it.",
    "A `command` tunnel can still do it.",
)
section(
    "### 6.4 Locating tools",
    "### 6.5",
    """### 6.4 Locating tools

The daemon is often started by the desktop app or at login, and on macOS those processes do not get the shell's PATH. Programs are `ssh` (for the SSH types and `command` tunnels that run ssh) or a command name (the first word of a `command` tunnel or a login's `check` or `login`). Resolution order:

1. `ssh`: `tools.ssh` if set; a missing file there is an error, not a fallback. A command given as a path (it contains `/` or `\\`, `~` expanded) is used as is.
2. `PATH` of the daemon process (which also finds `.cmd` and `.exe` on Windows).
3. The directories in `tools.path`, in order.
4. Known install locations per OS with no vendor-specific folders: `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `~/.local/bin` on macOS; `/usr/local/bin`, `/usr/bin`, `/snap/bin`, `~/.local/bin` on Linux. Vendor folders such as `~/google-cloud-sdk/bin` go in `tools.path`.

The resolved path and its source appear in `doctor`, which checks every program the config uses, including login commands. A missing program fails the tunnel with reason `missing tool` and says to install it, use its full path, or add its directory to `tools.path` (FR-46).

""",
)
rep(
    "### 6.5 `gcloud.cmd` on Windows (FR-54)",
    "### 6.5 Batch-file tools on Windows (FR-54)",
)
rep(
    "covers it with tests that run a fake `gcloud.cmd` (section 10).",
    "covers it with tests that run a fake `.cmd` tool (section 10). gcloud is the common case (`gcloud.cmd`).",
)

# 7
rep(
    "- **gcloud variables are cleared.** A tunnel with a project first has `CLOUDSDK_CORE_PROJECT`, `CLOUDSDK_CORE_ACCOUNT` and `CLOUDSDK_CONFIG` removed from its inherited environment, then gets the project's own values (FR-7). Without this, a variable set in the daemon's environment could leak into a project that does not set it. A test caught this.",
    "- **The environment is added, never cleared.** A tunnel inherits the daemon's environment plus its login's and profile's `env` (FR-7). Draft 1 removed gcloud's variables first; that was tool-specific and is gone. A variable that must not leak from the daemon's environment has to be set in the profile.",
)
rep(
    "| Auth expiry from the scheduled check | `AuthExpired`: terminate, `failed(auth expired)` (section 9) |",
    "| Login expiry from the scheduled check | `AuthExpired`: terminate, `failed(auth expired)` (section 9) |",
)
rep(
    "| Auth recovery (FR-25) | `AuthRecovered` in `failed(auth expired)`:",
    "| Login recovery (FR-25) | `AuthRecovered` in `failed(auth expired)`:",
)

# 8.1
rep(
    "For `gcloud-iap`, a listening port is a stronger signal than for other types: gcloud opens the local port only after a test connection through IAP succeeds.",
    "For `gcloud compute start-iap-tunnel`, a listening port is a stronger signal than for most commands: gcloud opens the local port only after a test connection through IAP succeeds.",
)

# 9
section(
    "## 9. gcloud auth",
    "## 10. Testing",
    """## 9. Logins

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

""",
)

# 10
rep(
    "It is also a multi-call binary. Copied or linked as `gcloud`, `ssh` or `kubectl`, it reads a scenario file named by `FAKE_TUNNEL_SCENARIO` and pretends to be that tool. This lets the built-in types run end to end on all three OSes without the real tools, including `auth print-access-token`. On Windows a `gcloud.cmd` wrapper around the fake tests the batch-file path (6.5).",
    "Logins are tested with a fake login tool: a small shell script with `status` and `login` commands whose answers are controlled by marker files (expired per account, network down). It records each call with its environment, so tests can check that a check ran with the login's `env` and that its token output was never read. On Windows a `.cmd` wrapper around `fake-tunnel` will test the batch-file path (6.5).",
)
rep(
    "| Integration | Docker OpenSSH server and jump host, `kind` for kubectl (AC-11) | Linux CI only |\n| Manual | AC-11 against real GCP before each release | developer machine |",
    "| Integration | Docker OpenSSH server and jump host, `kind` for a kubectl `command` tunnel (AC-11) | Linux CI only |\n| Manual | AC-11 against real GCP (gcloud `command` tunnels and the example `google` login) before each release | developer machine |",
)

# 12
rep(
    "2. `types`: trait, SSH builder, the five built-in types and `custom`, launch-plan snapshots, pattern corpus.",
    "2. `types`: trait, SSH builder, the built-in types, launch-plan snapshots, pattern corpus. (Draft 2: the three SSH types and `command`.)",
)

s = s.rstrip("\n") + """

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
"""

open(P, "w", encoding="utf-8").write(s)
print("ok")
