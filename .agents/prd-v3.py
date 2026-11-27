"""Rewrites the PRD from gcloud projects to generic profiles and logins (Draft v3)."""

P = "PRD Tunwarden – Tunnel Manager.md"
s = open(P, encoding="utf-8").read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:90])
    s = s.replace(old, new)


rep(
    "Status: Draft v2.1 · Date: 26 Nov 2026",
    "Status: Draft v3 · Date: 27 Nov 2026",
)

rep(
    "## 0. Changes from Draft v1\n",
    """## 0. Changes

### Draft v3

Tunwarden is now generic. Nothing in the code is specific to gcloud, kubectl or any other tool:

- `[[projects]]` (a gcloud project, account and config directory) is replaced by two general tables. A **profile** is a set of environment variables, a `production` flag and an optional login. A **login** is a credential the user renews now and then, described by a `check` command, a `login` command, `expired` patterns and its own environment. Several profiles can share one login.
- The `gcloud-iap` and `kubectl-port-forward` types are removed. Those tunnels are `command` tunnels (the renamed `custom` type, now the default) and the example config shows them. The SSH types stay, because SSH needs safe defaults (FR-42) and supports `trust`.
- The auth check, expiry detection and re-login (FR-22 to FR-25) work for any tool through the login's commands and patterns. `auth` and `login` take a login or profile name; `--project` becomes `--profile`.
- `[tools]` keeps only `ssh` and gains `path`, a list of extra directories to find commands in. The gcloud SDK install folders are no longer searched.
- A profile's environment is added on top of the inherited environment. Nothing is removed from it.
- `auth_check_interval_minutes` is renamed `login_check_interval_minutes`.

### Draft v2 (from Draft v1)
""",
)

rep(
    "- gcloud login is tracked per auth context (account plus config directory), not per project.\n",
    "- gcloud login is tracked per auth context (account plus config directory), not per project. (Generalised to logins in v3.)\n",
)
rep(
    "- `gcloud-ssh` moves past v1. `custom` tunnels may now use a project.\n",
    "- `gcloud-ssh` moves past v1. `custom` tunnels may now use a project. (v3 removes all gcloud types.)\n",
)

# 1. Overview
rep(
    "manage many tunnels of any kind (plain SSH, gcloud IAP, Kubernetes port-forward or any custom command) from one place. It replaces a pile of personal zsh/.zshrc scripts with a reliable tool that can start, stop and add tunnels, show live health, reconnect automatically, keep logs, and handle gcloud's 12-hour login expiry across multiple gcloud projects.",
    "manage many tunnels of any kind (plain SSH, or any command that opens a local port, such as `gcloud compute start-iap-tunnel`, `kubectl port-forward` or `aws ssm start-session`) from one place. It replaces a pile of personal zsh/.zshrc scripts with a reliable tool that can start, stop and add tunnels, show live health, reconnect automatically, keep logs, and handle logins that expire (for example gcloud's 12-hour session) across several accounts and environments.",
)
rep(
    "The tool is generic. Nothing company-specific (project IDs, hostnames, VM names) is bundled; everything comes from the user's own config file.",
    "The tool is generic. It has no code specific to a cloud or tool, and nothing company-specific (project IDs, hostnames, VM names) is bundled; everything comes from the user's own config file.",
)

# 3. Goals
rep(
    "- G3. Detect expired gcloud auth, make re-login one step, and recover affected tunnels automatically afterwards.\n- G4. Support multiple gcloud projects/accounts at once without them interfering.",
    "- G3. Detect expired logins of any tool, make re-login one step, and recover affected tunnels automatically afterwards.\n- G4. Support several environments and accounts at once (through profiles) without them interfering.",
)
rep(
    "The tools a tunnel type uses (ssh, gcloud, kubectl)",
    "The tools the user's tunnels run (ssh, gcloud, kubectl and so on)",
)
rep(
    "- G9. Support every kind of tunnel through one model: built-in types for plain SSH, gcloud IAP and kubectl port-forward, plus a custom type for any command that opens a local port (see 7.10).",
    "- G9. Support every kind of tunnel through one model: built-in types for plain SSH, plus a `command` type for any command that opens a local port (see 7.10).",
)
rep(
    "- No `gcloud-ssh` built-in type. Users run `gcloud compute ssh ... -- -L ...` as a `custom` tunnel. See section 14.",
    "- No tool-specific built-in types beyond SSH. gcloud, kubectl, AWS and other tunnels are `command` tunnels. See section 14.",
)

# 6. Key concepts
rep(
    """- **Project:** a gcloud context (gcloud project ID, optional account, optional isolated gcloud config directory). Optional: only gcloud-based and custom tunnels use one.
- **Auth context:** the pair (gcloud account, gcloud config directory) that a project uses. gcloud login belongs to an auth context, not a project, so projects that share an account and config directory share one login. Tunwarden derives auth contexts from the projects; they are not configured separately.
- **Tunnel:** a named connection of a given type with one or more forwards, optionally belonging to one project.""",
    """- **Profile:** settings shared by tunnels: environment variables (for example `CLOUDSDK_CORE_PROJECT` or `AWS_PROFILE`), a `production` flag, and optionally the login its tunnels depend on. Optional; any tunnel may use one.
- **Login:** a credential the user renews now and then, for any tool. It has an optional `check` command that succeeds while the login is valid, an optional interactive `login` command, `expired` patterns (output that means "log in again") and its own environment. Several profiles can share one login, so one re-login recovers all of them.
- **Tunnel:** a named connection of a given type with one or more forwards, optionally using one profile.""",
)
rep(
    "Built-in types cover plain SSH, gcloud IAP, kubectl port-forward and a custom command (see 7.10).",
    "Built-in types cover SSH local, remote and dynamic forwards and any command (see 7.10).",
)
rep(
    "- **Prerequisite tool:** the program a type depends on (ssh, gcloud or kubectl).",
    "- **Prerequisite tool:** a program the config runs: ssh for SSH types, a `command` tunnel's program, and each login's `check` and `login` programs.",
)

# 7.1
rep(
    "defines settings, projects and tunnels.",
    "defines settings, tools, logins, profiles and tunnels.",
)
rep(
    "- FR-2. Project fields (a project is optional; only gcloud-based and custom tunnels use one): name (required), gcloud project ID, account, optional separate gcloud config directory (supports `~`), `production` flag (default false, see FR-33).",
    """- FR-2. Profile and login fields:
  - Profile: name (required), `env` (a map of environment variables), `production` flag (default false, see FR-33), `login` (the name of a login, optional).
  - Login: name (required), `check` (a command line that exits 0 while the login is valid, optional), `login` (an interactive command line that renews it, optional), `expired` (regular expressions, matched ignoring case, optional), `env` (a map of environment variables for both commands).""",
)
rep(
    "- FR-3. Tunnel fields: name, type (default: custom), project (only for gcloud-based types and `custom`),",
    "- FR-3. Tunnel fields: name, type (default: `command`), profile (optional, any type),",
)
rep(
    "  - unique tunnel and project names;",
    "  - unique tunnel, profile and login names;",
)
rep(
    "  - a tunnel that names a project references an existing one;\n  - each type's required fields are present and valid (for `custom`, the command line parses under the rules in FR-8 and is non-empty);",
    "  - a tunnel that names a profile, and a profile that names a login, reference existing ones;\n  - each type's required fields are present and valid (for `command`, and for a login's `check` and `login`, the command line parses under the rules in FR-8 and is non-empty), and `expired` patterns are valid regular expressions;",
)

rep(
    """[settings]
restore_on_start = true
auth_check_interval_minutes = 5

[[projects]]
name = "staging"
gcloud_project = "example-staging-123"
account = "me@example.com"

[[projects]]
name = "prod"
gcloud_project = "example-prod-456"
account = "me@example.com"
gcloud_config_dir = "~/.config/gcloud-prod"
production = true

[[tunnels]]
name = "staging-db"
type = "gcloud-iap"
project = "staging"
instance = "db-1"
zone = "europe-west1-b"
groups = ["daily"]
forwards = [{ local_port = 5433, remote_port = 5432 }]
""",
    """[settings]
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
""",
)

# 7.2
rep(
    "### 7.2 Project isolation\n\n- FR-7. Every tunnel process that uses a project is started with the project's settings applied through environment variables (`CLOUDSDK_CORE_PROJECT`, `CLOUDSDK_CORE_ACCOUNT`, and `CLOUDSDK_CONFIG` when an isolated config directory is set). The tool must never modify the user's global gcloud configuration.",
    "### 7.2 Profile isolation\n\n- FR-7. Every tunnel process that uses a profile is started with the inherited environment plus the login's environment (if the profile has a login), so the tunnel runs as the account the check verifies, plus the profile's environment. The profile's value wins over the login's for the same variable. This is how tools are pointed at an account, project or config directory (for example `CLOUDSDK_CORE_PROJECT`, `CLOUDSDK_CONFIG`, `AWS_PROFILE`, `KUBECONFIG`). The tool never modifies any tool's global configuration.",
)
rep(
    "A `custom` tunnel's command line is split",
    "A `command` tunnel's command line, and a login's `check` and `login` commands, are split",
)

# 7.3
rep(
    "a group, a project, or all tunnels.",
    "a group, a profile, or all tunnels.",
)
rep(
    "ssh, gcloud and kubectl hold no state that a forced stop loses.",
    "ssh, gcloud and kubectl hold no state that a forced stop loses; other commands are expected to behave the same.",
)

# 7.4
rep(
    "Doing so would create a new IAP or kubectl connection every interval",
    "Doing so would create a new remote session (for example IAP or kubectl) every interval",
)
rep(
    "  - the scheduled auth check (FR-22).",
    "  - the scheduled login check (FR-22).",
)

# 7.5
old_75_start = s.index("### 7.5 gcloud authentication")
old_75_end = s.index("### 7.6 Logging")
s = (
    s[:old_75_start]
    + """### 7.5 Logins

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

"""
    + s[old_75_end:]
)

# 7.7 CLI table
rep(
    "| `start`, `stop`, `restart` | Accept names, `--all`, `--project`, `--group`;",
    "| `start`, `stop`, `restart` | Accept names, `--all`, `--profile`, `--group`;",
)
rep(
    "| `auth [project]` | Check gcloud logins; exits non-zero if one has expired |\n| `login PROJECT` | Re-login the project's auth context and recover auth-failed tunnels |",
    "| `auth [NAME]` | Check logins (all, or one login or profile); exits non-zero if one has expired |\n| `login NAME` | Run a login's `login` command (NAME is a login or profile) and recover auth-failed tunnels |",
)
rep(
    "- Starting a tunnel in a `production` project asks",
    "- Starting a tunnel in a `production` profile asks",
)

# 7.8
rep(
    "re-login per project, open dashboard, quit.",
    "re-login per login, open dashboard, quit.",
)
rep(
    "- FR-33. Dashboard window listing tunnels grouped by project, with tunnels that have no project under \"No project\". Each row shows state, failure reason, local ports, uptime and restarts. Projects with `production = true`",
    "- FR-33. Dashboard window listing tunnels grouped by profile, with tunnels that have no profile under \"No profile\". Each row shows state, failure reason, local ports, uptime and restarts. Profiles with `production = true`",
)
rep(
    "Tools are found in this order: the path set in the config, the daemon's PATH, then known install locations per OS (for example Homebrew and the gcloud SDK folder).",
    "Tools are found in this order: the path set in the config (`tools.ssh`, or a command given as a path), the daemon's PATH, the directories in `tools.path`, then known install locations per OS (for example Homebrew, `/usr/local/bin` and `~/.local/bin`).",
)
rep("  - auth per auth context;", "  - each login's check;")

# 7.9
rep(
    "because Linux's parent-death signal only reaches the direct child, and tools such as gcloud start their own child processes.",
    "because Linux's parent-death signal only reaches the direct child, and many tools (gcloud, for example) start their own child processes.",
)

# 7.10
rep(
    "| `gcloud-iap` | `gcloud compute start-iap-tunnel` | project, instance, zone, remote port, local bind address and port |\n| `kubectl-port-forward` | `kubectl port-forward` | kube context, namespace, resource, forwards (local bind address and port, remote port) |\n| `custom` | Any command the user provides | full command line, optional project, forwards (local ports to check), optional extra credential-error patterns |",
    "| `command` (default) | Any command the user provides | full command line, forwards (local ports to check), optional credential-error patterns |\n\nEvery type can use a profile. Tool-specific tunnels such as `gcloud compute start-iap-tunnel`, `gcloud compute ssh --tunnel-through-iap`, `kubectl port-forward` and `aws ssm start-session` are `command` tunnels; the example config shows each.",
)
rep(
    "Only the `custom` type takes a full command line.",
    "Only the `command` type takes a full command line.",
)
rep(
    "- FR-43. Creating SSH tunnels can start from hosts found in `~/.ssh/config`. Pasted commands (FR-6) are matched to a built-in type when possible, otherwise saved as `custom`.",
    "- FR-43. Creating SSH tunnels can start from hosts found in `~/.ssh/config`. Pasted `ssh` commands (FR-6) are matched to an SSH type when possible; everything else is saved as `command`, with local ports guessed from the command. Leading `NAME=value` assignments become a profile with that environment (an existing profile with the same environment is reused).",
)
rep(
    "for `ssh-local`, `ssh-dynamic`, `gcloud-iap`, `kubectl-port-forward` and `custom`.",
    "for `ssh-local`, `ssh-dynamic` and `command`.",
)
rep(
    "A `custom` tunnel may choose \"process alive\" instead of a port check.",
    "A `command` tunnel without forwards counts as ready while its process runs.",
)
rep(
    "- FR-45. Credential problems are detected per type and stop retries with an actionable message, as FR-24 does for gcloud. Examples: SSH permission denied, key not in agent, or changed host key; kubectl expired credentials or a failing exec auth plugin; IAP refusing the connection (`4033: 'not authorized'`), which means a missing IAM permission or a port the IAP policy does not allow, not an expired login. Detection patterns live in each type definition and are covered by tests.",
    "- FR-45. Credential problems stop retries with an actionable message, as FR-24 does for expired logins. The SSH types detect SSH permission denied, a key not in the agent, and an unknown or changed host key. A `command` tunnel lists its own `credential_patterns` (for example kubectl's \"You must be logged in to the server\", or IAP's `4033: 'not authorized'`, which means a missing permission rather than an expired login). Built-in patterns live in the type definition and are covered by tests.",
)
rep(
    "- FR-46. Before starting, the tool checks that the type's prerequisite tools are installed",
    "- FR-46. Before starting, the tool checks that the tunnel's program is installed",
)
rep(
    "- FR-47. Groups, projects, status, logs, retry and notifications work identically for every type. A `custom` tunnel may add its own credential-error patterns. A `custom` tunnel with a project gets the project's environment (FR-7) and gcloud auth handling (FR-22 to FR-25), so a gcloud command run as `custom` behaves like a built-in gcloud type.",
    "- FR-47. Groups, profiles, logins, status, logs, retry and notifications work identically for every type. Any tunnel with a profile gets the profile's environment (FR-7) and its login's handling (FR-22 to FR-25).",
)

# 7.11
rep(
    "`credentials` (non-gcloud credential error)",
    "`credentials` (a credential error other than an expired login)",
)
rep(
    "- FR-54. On Windows, gcloud is installed as `gcloud.cmd`, a batch script that can only run through `cmd.exe`.",
    "- FR-54. On Windows, some tools are installed as batch scripts (gcloud as `gcloud.cmd`, for example), which can only run through `cmd.exe`.",
)

# 8
rep(
    "→ `failed` on a credential error in the output or an `expired` auth check (FR-22).",
    "→ `failed` on a credential error in the output or an `expired` login check (FR-22).",
)
rep(
    "after auth recovery (FR-25)",
    "after login recovery (FR-25)",
)

# 9
rep(
    "The auth check runs gcloud, which takes about a second, so it only runs for auth contexts with active tunnels (FR-22).",
    "A login check runs the user's tool, which can take a second or more, so it only runs for logins with active tunnels (FR-22).",
)
rep(
    "No network calls except those made by the user's own tunnel commands, gcloud, and `tunwarden trust`",
    "No network calls except those made by the user's own tunnel and login commands, and `tunwarden trust`",
)
rep(
    "- **Testability:** the core must be testable without ssh, gcloud or kubectl, using the `fake-tunnel` helper.",
    "- **Testability:** the core must be testable without ssh or any cloud tool, using the `fake-tunnel` helper and a fake login tool.",
)

# 10
rep(
    "the tests that use `fake-tunnel` and a fake gcloud.",
    "the tests that use `fake-tunnel` and a fake login tool.",
)
rep(
    "integration tests with a Docker SSH server and jump host, and `kind` for kubectl.",
    "integration tests with a Docker SSH server and jump host, and `kind` for a kubectl `command` tunnel.",
)

# 11
rep(
    "   - Tunnel types: SSH (local, remote, dynamic, jump hosts), gcloud IAP, kubectl and custom.",
    "   - Tunnel types: SSH (local, remote, dynamic, jump hosts) and command. Profiles and logins.",
)
rep(
    "deeper health checks and latency, `gcloud-ssh` type.",
    "deeper health checks and latency, a library of ready-made login and tunnel templates.",
)

# 12
rep(
    "- AC-1. With two projects configured and four tunnels,",
    "- AC-1. With two profiles configured and four tunnels,",
)
rep(
    "- AC-3. Simulating gcloud auth expiry (fake command printing a re-authentication error) leads to `failed` with reason `auth expired` and the login hint within 10 seconds, with no further retries. After `login`, the affected tunnels restart without extra commands, including tunnels in other projects that share the same auth context.",
    "- AC-3. Simulating a login expiry (a fake tool whose check fails with a re-authentication error, or a tunnel printing one) leads to `failed` with reason `auth expired` and the login hint within 10 seconds, with no further retries. After `login`, the affected tunnels restart without extra commands, including tunnels in other profiles that share the same login. A check failing for another reason (no network) stops nothing.",
)
rep(
    "- AC-8. Two tunnels in different projects run at once, each against its own project, without touching the global gcloud config.",
    "- AC-8. Two tunnels in different profiles run at once, each with its own environment (for example its own gcloud project), without touching any tool's global config.",
)
rep(
    "- AC-11. One config containing an SSH local forward, an SSH forward through a jump host, a gcloud IAP tunnel and a kubectl port-forward runs side by side.",
    "- AC-11. One config containing an SSH local forward, an SSH forward through a jump host, a gcloud IAP `command` tunnel and a kubectl port-forward `command` tunnel runs side by side.",
)
rep(
    "  - The gcloud part is automated with a fake gcloud.",
    "  - The login part is automated with a fake login tool.",
)

# 13
rep(
    "- **gcloud auth error text varies by version:** keep the detection patterns in one place, covered by tests, and easy to extend; fall back to the scheduled auth check.",
    "- **A tool's login error text varies by version:** the patterns live in the user's config, so they can be fixed without a release; the example config's patterns are checked against real output (`docs/spikes.md`). The scheduled check is the fallback.\n- **A login check that fails for other reasons looks expired:** without `expired` patterns, any failure counts as expired. The example config lists patterns for every login, and the docs recommend them.",
)
rep(
    "- **Windows batch-file quoting for `gcloud.cmd`:**",
    "- **Windows batch-file quoting for tools such as `gcloud.cmd`:**",
)

# 14
rep(
    "- `gcloud-ssh` built-in type (`gcloud compute ssh --tunnel-through-iap` with port forwards). Deferred because it needs gcloud's SSH key setup, and on Windows gcloud uses PuTTY by default. Until then, `custom` with a project covers it.\n- More tunnel types: AWS SSM Session Manager, Azure Bastion, Cloudflare Tunnel, Docker or Podman port forwards, as built-in types or contributed templates.\n- Per-tunnel environment variables and pre/post-start hooks.",
    "- A library of ready-made login and tunnel templates (gcloud IAP, kubectl, AWS SSM, Azure Bastion, Cloudflare Tunnel, Docker or Podman port forwards) that `add` and `init` can offer, as data rather than code.\n- Per-tunnel environment variables (today they come from the profile) and pre/post-start hooks.",
)
rep(
    "- Plugin or template library for common tunnel commands.\n",
    "",
)

# 15
rep(
    "2. v1 ships only the types in 7.10. `gcloud-ssh` and other types go to section 14.",
    "2. v1 ships only the types in 7.10: the SSH types and `command`. Tool-specific behaviour lives in the user's config (profiles, logins, patterns), not in code (v3).",
)

# 16
rep(
    "the per-project environment approach,",
    "the per-environment approach (now profiles),",
)

open(P, "w", encoding="utf-8").write(s)
print("ok")
