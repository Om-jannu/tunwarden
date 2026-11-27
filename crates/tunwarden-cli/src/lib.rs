//! The `tunwarden` command line (PRD 7.7). Kept in a library so the system
//! tests can run exactly this code next to their fake tunnels.

mod table;

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde_json::Value;
use tunwarden_core::auth::{self as gauth, Outcome as AuthOutcome};
use tunwarden_core::client::{Client, ClientError, Launcher};
use tunwarden_core::config::{self, Config, LoadError, Login};
use tunwarden_core::daemon::{self, DaemonError, tunnel_info};
use tunwarden_core::fsutil::parse_timestamp;
use tunwarden_core::paths::{self, Os, SystemEnv};
use tunwarden_core::proto::{
    ActionResult, AuthParams, AuthResult, ConfigAddEntryParams, ConfigAddParams, ConfigEditParams,
    ConfigEditResult, ConfigRemoveParams, DoctorResult, ListResult, LogNotice, LoginView,
    LogsParams, LogsResult, RecoveredParams, RecoveredResult, Selector, ShutdownParams,
    StatusParams, StatusResult, SubscribeParams, TunnelInfo, TunnelView, method, notice,
};
use tunwarden_core::state::Desired;
use tunwarden_core::supervisor::State;

use table::Table;

/// Exit code when the daemon is not running (`daemon status`).
const EXIT_NOT_RUNNING: u8 = 3;
/// The example config written by `init`.
const EXAMPLE_CONFIG: &str = include_str!("../../../examples/config.toml");

#[derive(Parser)]
#[command(
    name = "tunwarden",
    version,
    about = "Keep tunnels running: any command that opens a local port, plus built-in SSH forwards"
)]
struct Cli {
    /// Config file to use [default: per-OS location; env: TUNWARDEN_CONFIG]
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an example config file
    Init,
    /// Print where the config file is
    ConfigPath,
    /// List the configured tunnels
    List {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Show the state of tunnels
    Status {
        /// Tunnels to show (default: all)
        names: Vec<String>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Start tunnels
    Start(Action),
    /// Stop tunnels
    Stop(Action),
    /// Restart tunnels
    Restart(Action),
    /// Show a tunnel's log
    Logs {
        name: String,
        /// Number of lines to show
        #[arg(short = 'n', long = "lines", default_value_t = 50)]
        lines: usize,
        /// Keep printing new lines
        #[arg(short, long)]
        follow: bool,
    },
    /// Print the command a tunnel runs
    ShowCommand { name: String },
    /// Add a tunnel to the config
    Add {
        name: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Change a tunnel in the config
    Edit {
        name: String,
        #[command(flatten)]
        fields: Fields,
        /// Remove a field; repeat for several
        #[arg(long = "unset", value_name = "KEY")]
        unset: Vec<String>,
    },
    /// Remove a tunnel from the config (stops it if it runs)
    Remove { name: String },
    /// Turn pasted tunnel commands or aliases into config entries
    Import {
        /// File to read; standard input if missing or `-`
        file: Option<PathBuf>,
        /// Add the entries to the config instead of printing them
        #[arg(long)]
        write: bool,
    },
    /// Accept an SSH host key (asks before trusting it)
    Trust {
        /// An SSH tunnel's name, or a host
        target: String,
    },
    /// Check the setup: config, tools, daemon, logins and ports
    Doctor,
    /// Check logins (runs each login's `check` command)
    Auth {
        /// A login, or a profile meaning its login; all logins if missing
        name: Option<String>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a login's `login` command, then restart the tunnels waiting for it
    Login {
        /// A login, or a profile meaning its login
        name: String,
    },
    /// Print a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Manage the background daemon that owns all tunnels
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Args)]
struct Action {
    /// Tunnel names
    names: Vec<String>,
    /// All tunnels
    #[arg(long)]
    all: bool,
    /// The tunnels in a group
    #[arg(long)]
    group: Option<String>,
    /// The tunnels of a profile
    #[arg(long)]
    profile: Option<String>,
    /// Start production tunnels without asking
    #[arg(long, short = 'y')]
    yes: bool,
    /// Seconds to wait for the tunnels to settle; 0 returns at once
    #[arg(long, default_value_t = 30, value_name = "SECONDS")]
    wait: u64,
}

impl Action {
    fn selector(&self, dry_run: bool) -> Selector {
        Selector {
            names: self.names.clone(),
            group: self.group.clone(),
            profile: self.profile.clone(),
            all: self.all,
            dry_run,
        }
    }
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Run the daemon in the foreground (used by `start` and autostart)
    Run,
    /// Start the daemon in the background if it is not running
    Start,
    /// Stop the daemon and all tunnels
    Stop {
        /// Start the running tunnels again the next time the daemon starts
        #[arg(long)]
        keep: bool,
    },
    /// Stop the daemon, keeping running tunnels, and start it again
    Restart,
    /// Show whether the daemon is running
    Status,
}

struct Ctx {
    config: PathBuf,
    state_dir: PathBuf,
}

/// A failure with an optional next step, shown as `error:` and `hint:`.
struct Failure {
    message: String,
    hint: Option<String>,
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Failure {
        Failure {
            hint: e.hint().map(str::to_owned),
            message: e.to_string(),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Failure {
        Failure {
            message,
            hint: None,
        }
    }
}

type Outcome = Result<ExitCode, Failure>;

/// Runs the command line with `args` (including the program name).
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let ctx = match locations(cli.config) {
        Ok(ctx) => ctx,
        Err(e) => return report(e),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return report(format!("cannot start: {e}").into()),
    };
    match runtime.block_on(dispatch(&ctx, cli.command)) {
        Ok(code) => code,
        Err(e) => report(e),
    }
}

fn report(e: Failure) -> ExitCode {
    eprintln!("error: {}", e.message);
    if let Some(hint) = e.hint {
        eprintln!("hint: {hint}");
    }
    ExitCode::FAILURE
}

fn locations(flag: Option<PathBuf>) -> Result<Ctx, Failure> {
    let os = Os::current();
    let config = paths::config_file(flag.as_deref(), &SystemEnv, os).map_err(|e| e.to_string())?;
    let config = std::path::absolute(&config).unwrap_or(config);
    let state_dir = paths::state_dir(&SystemEnv, os).map_err(|e| e.to_string())?;
    Ok(Ctx { config, state_dir })
}

async fn dispatch(ctx: &Ctx, command: Cmd) -> Outcome {
    match command {
        Cmd::Init => init(ctx),
        Cmd::ConfigPath => {
            println!("{}", ctx.config.display());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::List { json } => list(ctx, json),
        Cmd::ShowCommand { name } => show_command(ctx, &name),
        Cmd::Add { name, fields } => add(ctx, name, fields).await,
        Cmd::Edit {
            name,
            fields,
            unset,
        } => edit(ctx, name, fields, unset).await,
        Cmd::Remove { name } => remove(ctx, name).await,
        Cmd::Import { file, write } => import_cmd(ctx, file, write).await,
        Cmd::Trust { target } => trust(ctx, &target),
        Cmd::Doctor => doctor(ctx).await,
        Cmd::Auth { name, json } => auth(ctx, name, json).await,
        Cmd::Login { name } => login(ctx, &name).await,
        Cmd::Status { names, json } => status(ctx, names, json).await,
        Cmd::Start(a) => action(ctx, Kind::Start, a).await,
        Cmd::Stop(a) => action(ctx, Kind::Stop, a).await,
        Cmd::Restart(a) => action(ctx, Kind::Restart, a).await,
        Cmd::Logs {
            name,
            lines,
            follow,
        } => logs(ctx, &name, lines, follow).await,
        Cmd::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "tunwarden",
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Daemon(cmd) => daemon_command(ctx, cmd).await,
    }
}

// --- Config-only commands ----------------------------------------------------

fn init(ctx: &Ctx) -> Outcome {
    if ctx.config.exists() {
        return Err(Failure {
            message: format!("{} already exists", ctx.config.display()),
            hint: Some("edit it, or pass --config with another path".into()),
        });
    }
    if let Some(dir) = ctx.config.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&ctx.config, EXAMPLE_CONFIG)
        .map_err(|e| format!("cannot write {}: {e}", ctx.config.display()))?;
    println!("created {}", ctx.config.display());
    println!(
        "Replace the example logins, profiles and tunnels with your own, then run `tunwarden list`."
    );
    Ok(ExitCode::SUCCESS)
}

fn load(ctx: &Ctx) -> Result<Config, Failure> {
    config::load(&ctx.config).map_err(|e| match e {
        LoadError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => Failure {
            message: format!("no config file at {}", ctx.config.display()),
            hint: Some("run `tunwarden init` to create one".into()),
        },
        LoadError::Io { .. } => Failure::from(e.to_string()),
        LoadError::Invalid(errors) => Failure {
            message: format!(
                "the config file has {} problem(s):\n{errors}",
                errors.0.len()
            ),
            hint: None,
        },
    })
}

fn list(ctx: &Ctx, json: bool) -> Outcome {
    let config = load(ctx)?;
    let tunnels: Vec<TunnelInfo> = config
        .tunnels
        .iter()
        .map(|t| tunnel_info(&config, t))
        .collect();
    if json {
        print_json(&ListResult { tunnels });
        return Ok(ExitCode::SUCCESS);
    }
    if tunnels.is_empty() {
        println!("no tunnels configured in {}", ctx.config.display());
        return Ok(ExitCode::SUCCESS);
    }
    let mut table = Table::new(&["NAME", "TYPE", "PROFILE", "LOCAL", "GROUPS"]);
    for t in &tunnels {
        let profile = match (&t.profile, t.production) {
            (Some(p), true) => format!("{p} (production)"),
            (Some(p), false) => p.clone(),
            (None, _) => "-".into(),
        };
        table.row(vec![
            t.name.clone(),
            t.kind.clone(),
            profile,
            or_dash(t.local.join(", ")),
            or_dash(t.groups.join(", ")),
        ]);
    }
    print!("{table}");
    Ok(ExitCode::SUCCESS)
}

fn show_command(ctx: &Ctx, name: &str) -> Outcome {
    let config = load(ctx)?;
    let t = config
        .tunnels
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| unknown_tunnel(&config, name))?;
    println!("{}", tunnel_info(&config, t).command);
    Ok(ExitCode::SUCCESS)
}

fn unknown_tunnel(config: &Config, name: &str) -> Failure {
    let names: Vec<&str> = config.tunnels.iter().map(|t| t.name.as_str()).collect();
    Failure {
        message: format!("no tunnel named \"{name}\""),
        hint: Some(if names.is_empty() {
            "no tunnels are configured".into()
        } else {
            format!("configured tunnels: {}", names.join(", "))
        }),
    }
}

// --- Daemon-backed commands --------------------------------------------------

async fn connect_or_start(ctx: &Ctx) -> Result<Client, Failure> {
    let launcher =
        Launcher::current_exe(&ctx.config).map_err(|e| format!("cannot find this program: {e}"))?;
    Ok(Client::connect_or_start(&ctx.state_dir, "cli", &launcher).await?)
}

async fn status(ctx: &Ctx, names: Vec<String>, json: bool) -> Outcome {
    let result: StatusResult = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(mut c) => c.call(method::STATUS, StatusParams { names }).await?,
        Err(ClientError::NotRunning) => offline_status(ctx, &names)?,
        Err(e) => return Err(e.into()),
    };
    if json {
        print_json(&result);
        return Ok(ExitCode::SUCCESS);
    }
    for e in &result.config_errors {
        eprintln!("config: {e}");
    }
    for n in &result.notes {
        eprintln!("note: {n}");
    }
    if result.tunnels.is_empty() {
        println!("no tunnels configured");
        return Ok(ExitCode::SUCCESS);
    }
    print!("{}", status_table(&result.tunnels));
    Ok(ExitCode::SUCCESS)
}

/// Without a daemon nothing runs, so every configured tunnel is stopped.
fn offline_status(ctx: &Ctx, names: &[String]) -> Result<StatusResult, Failure> {
    let config = load(ctx)?;
    for n in names {
        if !config.tunnels.iter().any(|t| &t.name == n) {
            return Err(unknown_tunnel(&config, n));
        }
    }
    let tunnels = config
        .tunnels
        .iter()
        .filter(|t| names.is_empty() || names.contains(&t.name))
        .map(|t| {
            let info = tunnel_info(&config, t);
            TunnelView {
                name: info.name,
                kind: info.kind,
                profile: info.profile,
                production: info.production,
                groups: info.groups,
                local: info.local,
                state: State::Stopped,
                reason: None,
                last_error: None,
                pid: None,
                restarts: 0,
                retries: 0,
                since: String::new(),
                desired: Desired::Stopped,
                pending_restart: false,
                note: None,
            }
        })
        .collect();
    Ok(StatusResult {
        tunnels,
        config_errors: Vec::new(),
        notes: vec!["the daemon is not running, so no tunnels are running".into()],
    })
}

fn status_table(tunnels: &[TunnelView]) -> Table {
    let mut table = Table::new(&[
        "NAME", "STATE", "LOCAL", "PID", "UPTIME", "RESTARTS", "DETAIL",
    ]);
    for t in tunnels {
        let uptime = match t.state {
            State::Running | State::Unhealthy => parse_timestamp(&t.since)
                .and_then(|since| SystemTime::now().duration_since(since).ok())
                .map(human_duration)
                .unwrap_or_default(),
            _ => String::new(),
        };
        table.row(vec![
            t.name.clone(),
            t.state.as_str().to_owned(),
            or_dash(t.local.join(", ")),
            t.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            or_dash(uptime),
            t.restarts.to_string(),
            detail(t),
        ]);
    }
    table
}

fn detail(t: &TunnelView) -> String {
    let mut parts = Vec::new();
    match (t.state, t.reason, &t.last_error) {
        (State::Failed, Some(reason), Some(e)) => parts.push(format!("{reason}: {e}")),
        (State::Failed, Some(reason), None) => parts.push(reason.to_string()),
        (State::Unhealthy | State::Reconnecting | State::Starting, _, Some(e)) => {
            parts.push(e.clone())
        }
        _ => {}
    }
    if t.pending_restart {
        parts.push("config changed; restart to apply".into());
    }
    if let Some(note) = &t.note {
        parts.push(note.clone());
    }
    parts.join("; ")
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Start,
    Stop,
    Restart,
}

async fn action(ctx: &Ctx, kind: Kind, a: Action) -> Outcome {
    let mut client = if kind == Kind::Stop {
        match Client::connect(&ctx.state_dir, "cli").await {
            Ok(c) => c,
            Err(ClientError::NotRunning) => {
                println!("the daemon is not running, so no tunnels are running");
                return Ok(ExitCode::SUCCESS);
            }
            Err(e) => return Err(e.into()),
        }
    } else {
        connect_or_start(ctx).await?
    };
    let (rpc, verb) = match kind {
        Kind::Start => (method::START, "start"),
        Kind::Stop => (method::STOP, "stop"),
        Kind::Restart => (method::RESTART, "restart"),
    };

    // Ask before starting production tunnels (FR-33).
    if kind != Kind::Stop && !a.yes {
        let chosen: ActionResult = client.call(rpc, a.selector(true)).await?;
        let before: StatusResult = client
            .call(
                method::STATUS,
                StatusParams {
                    names: chosen.tunnels.clone(),
                },
            )
            .await?;
        let production: Vec<&str> = before
            .tunnels
            .iter()
            .filter(|t| t.production)
            .map(|t| t.name.as_str())
            .collect();
        if !production.is_empty() {
            confirm(verb, &production)?;
        }
    }

    let before: StatusResult = client
        .call(method::STATUS, StatusParams { names: Vec::new() })
        .await?;
    let done: ActionResult = client.call(rpc, a.selector(false)).await?;
    if a.wait == 0 {
        println!("{verb} requested for {}", done.tunnels.join(", "));
        return Ok(ExitCode::SUCCESS);
    }

    let settled = |now: &TunnelView| -> bool {
        let old = before.tunnels.iter().find(|t| t.name == now.name);
        let changed = old.is_none_or(|o| o.since != now.since);
        match kind {
            Kind::Stop => now.state == State::Stopped,
            Kind::Start if old.is_some_and(|o| o.state == State::Running) => {
                now.state == State::Running
            }
            Kind::Start | Kind::Restart => {
                changed && matches!(now.state, State::Running | State::Failed)
            }
        }
    };
    let deadline = Instant::now() + Duration::from_secs(a.wait);
    let views = loop {
        let now: StatusResult = client
            .call(
                method::STATUS,
                StatusParams {
                    names: done.tunnels.clone(),
                },
            )
            .await?;
        if now.tunnels.iter().all(settled) || Instant::now() >= deadline {
            break now.tunnels;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    let mut table = Table::new(&["NAME", "STATE", "LOCAL", "DETAIL"]);
    let mut ok = true;
    for t in &views {
        let good = match kind {
            Kind::Stop => t.state == State::Stopped,
            _ => t.state == State::Running,
        };
        ok &= good;
        let mut d = detail(t);
        if !settled(t) {
            d = format!(
                "still {} after {}s; see `tunwarden logs {}`",
                t.state.as_str(),
                a.wait,
                t.name
            );
        }
        table.row(vec![
            t.name.clone(),
            t.state.as_str().into(),
            or_dash(t.local.join(", ")),
            d,
        ]);
    }
    print!("{table}");
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn confirm(verb: &str, production: &[&str]) -> Result<(), Failure> {
    let list = production.join(", ");
    if !std::io::stdin().is_terminal() {
        return Err(Failure {
            message: format!("refusing to {verb} production tunnels ({list}) without confirmation"),
            hint: Some("pass --yes to confirm".into()),
        });
    }
    eprint!("{list} belong to a production profile. {verb} anyway? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(format!("{verb} cancelled").into())
    }
}

async fn logs(ctx: &Ctx, name: &str, lines: usize, follow: bool) -> Outcome {
    let mut client = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(c) => c,
        Err(ClientError::NotRunning) => {
            // The log file is still there without a daemon.
            let path = ctx.state_dir.join("logs").join(format!("{name}.log"));
            let config = load(ctx)?;
            if !config.tunnels.iter().any(|t| t.name == name) {
                return Err(unknown_tunnel(&config, name));
            }
            for line in tunwarden_core::logs::tail(&path, lines)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            {
                println!("{line}");
            }
            if follow {
                eprintln!("note: the daemon is not running, so no new lines will appear");
            }
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => return Err(e.into()),
    };
    if follow {
        let _: Value = client
            .call(
                method::SUBSCRIBE,
                SubscribeParams {
                    topics: vec![format!("log:{name}")],
                },
            )
            .await?;
    }
    let past: LogsResult = client
        .call(
            method::LOGS_TAIL,
            LogsParams {
                name: name.into(),
                lines,
            },
        )
        .await?;
    for line in &past.lines {
        println!("{line}");
    }
    if !follow {
        return Ok(ExitCode::SUCCESS);
    }
    // Lines written between subscribing and reading the tail arrive twice.
    // Lines start with a fixed-width timestamp, so text order is time order.
    let shown: std::collections::HashSet<String> = past.lines.iter().cloned().collect();
    let newest = past
        .lines
        .last()
        .map(|l| l.get(..24).unwrap_or(l).to_owned());
    loop {
        let n = client.next_notification().await?;
        if n.method != notice::LOG {
            continue;
        }
        let Ok(l) = serde_json::from_value::<LogNotice>(n.params) else {
            continue;
        };
        let older = newest
            .as_deref()
            .is_some_and(|n| l.line.get(..24).unwrap_or(&l.line) < n);
        if older || shown.contains(&l.line) {
            continue;
        }
        println!("{}", l.line);
        let _ = std::io::stdout().flush();
    }
}

// --- Config edits (FR-5) ---------------------------------------------------------

#[derive(Args)]
struct Fields {
    /// Tunnel type: command (the default), ssh-local, ssh-remote or ssh-dynamic
    #[arg(long = "type", value_name = "TYPE")]
    kind: Option<String>,
    /// Profile: shared environment, production flag and login
    #[arg(long)]
    profile: Option<String>,
    /// Group; repeat for several. Replaces the current groups when editing
    #[arg(long = "group", value_name = "GROUP")]
    groups: Vec<String>,
    /// Forward: LOCAL, LOCAL:REMOTE_PORT, LOCAL:HOST:REMOTE_PORT, or KEY=VALUE,...
    /// Repeat for several. Replaces the current forwards when editing
    #[arg(long = "forward", value_name = "SPEC")]
    forwards: Vec<String>,
    /// The command line of a command tunnel
    #[arg(long)]
    command: Option<String>,
    /// Any other field, for example host=bastion or 'jump=["j1","j2"]'
    #[arg(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
}

impl Fields {
    fn to_map(&self) -> Result<serde_json::Map<String, Value>, Failure> {
        let mut m = serde_json::Map::new();
        if let Some(k) = &self.kind {
            m.insert("type".into(), Value::String(k.clone()));
        }
        if let Some(p) = &self.profile {
            m.insert("profile".into(), Value::String(p.clone()));
        }
        if !self.groups.is_empty() {
            m.insert("groups".into(), Value::from(self.groups.clone()));
        }
        if let Some(c) = &self.command {
            m.insert("command".into(), Value::String(c.clone()));
        }
        if !self.forwards.is_empty() {
            let list: Result<Vec<Value>, Failure> =
                self.forwards.iter().map(|f| forward(f)).collect();
            m.insert("forwards".into(), Value::Array(list?));
        }
        for item in &self.set {
            let (key, value) = item.split_once('=').ok_or_else(|| Failure {
                message: format!("--set {item}: expected KEY=VALUE"),
                hint: Some("for example --set host=bastion".into()),
            })?;
            m.insert(key.trim().to_owned(), value_of(value));
        }
        Ok(m)
    }
}

/// A TOML value if `text` is one (`22`, `true`, `["a", "b"]`), else a string.
fn value_of(text: &str) -> Value {
    toml::from_str::<toml::Table>(&format!("v = {text}"))
        .ok()
        .and_then(|t| t.get("v").cloned())
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| Value::String(text.to_owned()))
}

/// Parses a `--forward` spec into a forward table.
fn forward(spec: &str) -> Result<Value, Failure> {
    let bad = || {
        Failure {
        message: format!("--forward {spec}: not a forward"),
        hint: Some("use LOCAL, LOCAL:REMOTE_PORT, LOCAL:HOST:REMOTE_PORT, or local_port=5433,remote_port=5432".into()),
    }
    };
    let mut m = serde_json::Map::new();
    if spec.contains('=') {
        for part in spec.split(',') {
            let (k, v) = part.split_once('=').ok_or_else(bad)?;
            m.insert(k.trim().to_owned(), value_of(v.trim()));
        }
        return Ok(Value::Object(m));
    }
    let port = |s: &str| s.parse::<u16>().map(Value::from).map_err(|_| bad());
    let parts: Vec<&str> = spec.split(':').collect();
    match parts[..] {
        [local] => {
            m.insert("local_port".into(), port(local)?);
        }
        [local, remote] => {
            m.insert("local_port".into(), port(local)?);
            m.insert("remote_port".into(), port(remote)?);
        }
        [local, host, remote] => {
            m.insert("local_port".into(), port(local)?);
            m.insert("remote_host".into(), Value::String(host.into()));
            m.insert("remote_port".into(), port(remote)?);
        }
        _ => return Err(bad()),
    }
    Ok(Value::Object(m))
}

async fn add(ctx: &Ctx, name: String, fields: Fields) -> Outcome {
    let fields = fields.to_map()?;
    let mut client = connect_or_start(ctx).await?;
    let r: ConfigEditResult = client
        .call(method::CONFIG_ADD, ConfigAddParams { name, fields })
        .await?;
    println!("added {} to {}", r.tunnel, r.config_path);
    println!("start it with `tunwarden start {}`", r.tunnel);
    Ok(ExitCode::SUCCESS)
}

async fn edit(ctx: &Ctx, name: String, fields: Fields, unset: Vec<String>) -> Outcome {
    let set = fields.to_map()?;
    if set.is_empty() && unset.is_empty() {
        return Err(Failure {
            message: "nothing to change".into(),
            hint: Some(
                "pass fields such as --set host=bastion, --forward 5433:5432 or --unset jump"
                    .into(),
            ),
        });
    }
    let mut client = connect_or_start(ctx).await?;
    let r: ConfigEditResult = client
        .call(method::CONFIG_EDIT, ConfigEditParams { name, set, unset })
        .await?;
    println!("updated {} in {}", r.tunnel, r.config_path);
    if r.restart_to_apply {
        println!(
            "it is running with its old settings; run `tunwarden restart {}` to apply",
            r.tunnel
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn remove(ctx: &Ctx, name: String) -> Outcome {
    let mut client = connect_or_start(ctx).await?;
    let r: ConfigEditResult = client
        .call(method::CONFIG_REMOVE, ConfigRemoveParams { name })
        .await?;
    println!("removed {} from {}", r.tunnel, r.config_path);
    Ok(ExitCode::SUCCESS)
}

// --- doctor (FR-36) --------------------------------------------------------------

/// One line of the doctor report.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Ok,
    Warning,
    Problem,
}

struct Report {
    rows: Vec<(String, Mark, String, String)>,
}

impl Report {
    fn add(
        &mut self,
        section: &str,
        mark: Mark,
        item: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.rows
            .push((section.to_owned(), mark, item.into(), detail.into()));
    }

    fn print(&self) -> bool {
        let mut problems = false;
        let mut section = "";
        for (s, mark, item, detail) in &self.rows {
            if s != section {
                if !section.is_empty() {
                    println!();
                }
                println!("{s}");
                section = s;
            }
            let label = match mark {
                Mark::Ok => "ok",
                Mark::Warning => "warning",
                Mark::Problem => {
                    problems = true;
                    "problem"
                }
            };
            let line = format!("  {label:<8} {item:<24} {detail}");
            println!("{}", line.trim_end());
        }
        problems
    }
}

async fn doctor(ctx: &Ctx) -> Outcome {
    let mut r = Report { rows: Vec::new() };

    // Config
    let config = match load(ctx) {
        Ok(c) => {
            r.add(
                "Config",
                Mark::Ok,
                ctx.config.display().to_string(),
                format!(
                    "{} tunnels, {} profiles, {} logins",
                    c.tunnels.len(),
                    c.profiles.len(),
                    c.logins.len()
                ),
            );
            Some(c)
        }
        Err(f) => {
            r.add(
                "Config",
                Mark::Problem,
                ctx.config.display().to_string(),
                f.message,
            );
            if let Some(h) = f.hint {
                r.add("Config", Mark::Problem, "", format!("hint: {h}"));
            }
            None
        }
    };

    // Daemon
    let mut client = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(c) => {
            let h = c.hello();
            r.add(
                "Daemon",
                Mark::Ok,
                "running",
                format!("PID {}, version {}", h.pid, h.daemon_version),
            );
            Some(c)
        }
        Err(ClientError::NotRunning) => {
            r.add(
                "Daemon",
                Mark::Ok,
                "not running",
                "`tunwarden start` starts it when needed",
            );
            None
        }
        Err(e) => {
            let hint = e.hint().map(|h| format!("; {h}")).unwrap_or_default();
            r.add("Daemon", Mark::Problem, "unreachable", format!("{e}{hint}"));
            None
        }
    };
    let mut daemon_status: Option<StatusResult> = None;
    let mut daemon_doctor: Option<DoctorResult> = None;
    if let Some(c) = client.as_mut() {
        let status: StatusResult = c
            .call(method::STATUS, StatusParams { names: Vec::new() })
            .await?;
        for e in &status.config_errors {
            r.add("Daemon", Mark::Problem, "config", e.clone());
        }
        for n in &status.notes {
            r.add("Daemon", Mark::Warning, "note", n.clone());
        }
        let d: DoctorResult = c.call(method::DOCTOR, serde_json::json!({})).await?;
        if let Some(e) = &d.write_error {
            r.add("Daemon", Mark::Problem, "state directory", e.clone());
        }
        daemon_status = Some(status);
        daemon_doctor = Some(d);
    }

    let Some(config) = config else {
        r.print();
        return Ok(ExitCode::FAILURE);
    };

    // Tools: the daemon's view decides, because it starts the tunnels.
    let mine = tunwarden_core::tools::check_all(&config).await;
    if mine.is_empty() {
        r.add(
            "Tools",
            Mark::Ok,
            "none needed",
            "no tunnels are configured",
        );
    }
    for t in &mine {
        let short = std::path::Path::new(&t.program)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| t.program.clone());
        let daemon = daemon_doctor
            .as_ref()
            .and_then(|d| d.tools.iter().find(|x| x.program == t.program));
        let seen = daemon.unwrap_or(t);
        match (&seen.path, &seen.error) {
            (Some(path), _) => {
                let version = seen
                    .version
                    .clone()
                    .map(|v| format!(", {v}"))
                    .unwrap_or_default();
                r.add(
                    "Tools",
                    Mark::Ok,
                    &short,
                    format!(
                        "{path} ({}{version})",
                        seen.source.clone().unwrap_or_default()
                    ),
                );
                if let (Some(d), Some(p)) = (daemon, &t.path)
                    && d.path.as_ref() != Some(p)
                {
                    r.add(
                        "Tools",
                        Mark::Warning,
                        "",
                        format!(
                            "your shell finds {} at {p}, but the daemon uses {path}",
                            t.program
                        ),
                    );
                }
            }
            (None, error) => {
                let mut detail = error.clone().unwrap_or_default();
                if let (Some(_), Some(p)) = (daemon, &t.path) {
                    // The macOS GUI PATH problem: the shell finds it, the daemon does not.
                    detail = format!(
                        "the daemon cannot find {0}, but your shell finds it at {p}; set `{0} = \"{p}\"` under [tools] in the config",
                        t.program
                    );
                }
                r.add("Tools", Mark::Problem, &short, detail);
            }
        }
    }

    // Logins
    if config.logins.is_empty() {
        r.add("Logins", Mark::Ok, "none", "no logins are configured");
    } else {
        let auth: AuthResult = match client.as_mut() {
            Some(c) => c.call(method::AUTH_CHECK, AuthParams::default()).await?,
            None => check_logins(&config, None).await?,
        };
        for l in &auth.logins {
            let item = if l.profiles.is_empty() {
                l.name.clone()
            } else {
                format!("{} ({})", l.name, l.profiles.join(", "))
            };
            let detail = l.detail.clone().unwrap_or_default();
            match l.result.as_str() {
                "ok" => r.add("Logins", Mark::Ok, item, detail),
                "not checked" => r.add("Logins", Mark::Ok, item, "no check command"),
                "expired" => r.add(
                    "Logins",
                    Mark::Problem,
                    item,
                    format!("expired; run `tunwarden login {}`", l.name),
                ),
                _ => r.add(
                    "Logins",
                    Mark::Warning,
                    item,
                    format!("could not check: {detail}"),
                ),
            }
        }
    }

    // Ports of tunnels that are not running must be free (FR-13).
    let running: Vec<String> = daemon_status
        .as_ref()
        .map(|s| {
            s.tunnels
                .iter()
                .filter(|t| t.pid.is_some())
                .map(|t| t.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let snapshot = tokio::task::spawn_blocking(tunwarden_core::probe::Snapshot::take)
        .await
        .map_err(|e| e.to_string())?;
    match snapshot {
        Err(e) => r.add(
            "Ports",
            Mark::Warning,
            "socket table",
            format!("cannot read it: {e}"),
        ),
        Ok(snapshot) => {
            let mut busy = 0;
            for t in config
                .tunnels
                .iter()
                .filter(|t| t.kind.listens_locally() && !running.contains(&t.name))
            {
                for f in &t.forwards {
                    let addr = std::net::SocketAddr::new(f.bind, f.local_port);
                    if let Some(h) = snapshot.holder(addr) {
                        busy += 1;
                        r.add(
                            "Ports",
                            Mark::Problem,
                            &t.name,
                            format!(
                                "{addr} is in use by {} (PID {}); `tunwarden start {}` would fail",
                                h.name, h.pid, t.name
                            ),
                        );
                    }
                }
            }
            if busy == 0 {
                r.add(
                    "Ports",
                    Mark::Ok,
                    "local ports",
                    "free for every tunnel that is not running",
                );
            }
        }
    }

    Ok(if r.print() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

// --- import (FR-6, FR-43) --------------------------------------------------------

async fn import_cmd(ctx: &Ctx, file: Option<PathBuf>, write: bool) -> Outcome {
    let text = match &file {
        Some(path) if path.as_os_str() != "-" => std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        _ => {
            if std::io::stdin().is_terminal() {
                eprintln!("Paste tunnel commands or alias lines, then press Ctrl-D:");
            }
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| format!("cannot read input: {e}"))?;
            text
        }
    };
    let config = match config::load(&ctx.config) {
        Ok(c) => c,
        Err(LoadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            config::parse(
                tunwarden_core::config::edit::NEW_FILE,
                &ctx.config,
                &SystemEnv,
            )
            .map_err(|e| e.to_string())?
        }
        Err(e) => return Err(load_error(ctx, e)),
    };
    let imported = tunwarden_core::import::import(&text, &config);
    for w in &imported.warnings {
        eprintln!("warning: {w}");
    }
    if imported.tunnels.is_empty() {
        return Err("no tunnel commands found in the input".to_owned().into());
    }
    if !write {
        print!("{}", imported.to_toml());
        eprintln!(
            "\nReview the entries above, then run the same command with --write to add them to {}.",
            ctx.config.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut client = connect_or_start(ctx).await?;
    for p in &imported.profiles {
        let _: ConfigEditResult = client
            .call(
                method::CONFIG_ADD_ENTRY,
                ConfigAddEntryParams {
                    table: "profiles".into(),
                    name: p.name.clone(),
                    fields: p.fields.clone(),
                },
            )
            .await?;
        println!("added profile {}", p.name);
    }
    for t in &imported.tunnels {
        let _: ConfigEditResult = client
            .call(
                method::CONFIG_ADD,
                ConfigAddParams {
                    name: t.name.clone(),
                    fields: t.fields.clone(),
                },
            )
            .await?;
        println!("added tunnel {}", t.name);
    }
    Ok(ExitCode::SUCCESS)
}

fn load_error(ctx: &Ctx, e: LoadError) -> Failure {
    match e {
        LoadError::Invalid(errors) => Failure {
            message: format!(
                "the config file has {} problem(s):\n{errors}",
                errors.0.len()
            ),
            hint: Some(format!("fix {} first", ctx.config.display())),
        },
        other => other.to_string().into(),
    }
}

// --- trust (FR-42) ---------------------------------------------------------------

/// Lets ssh ask the user to accept a host key, in this terminal. Tunwarden
/// never edits `known_hosts` itself.
fn trust(ctx: &Ctx, target: &str) -> Outcome {
    use tunwarden_core::types::SshTarget;
    let config = config::load(&ctx.config).ok();
    let tool_paths = config.as_ref().map(|c| c.tools.clone()).unwrap_or_default();
    let ssh_target: SshTarget = match config
        .as_ref()
        .and_then(|c| c.tunnels.iter().find(|t| t.name == target))
    {
        Some(t) => match t.type_settings.ssh_target() {
            Some(s) => s.clone(),
            None => {
                return Err(Failure {
                    message: format!("{target} is a {} tunnel, not an SSH tunnel", t.kind),
                    hint: Some("pass an SSH tunnel's name or a host".into()),
                });
            }
        },
        None => SshTarget {
            host: target.to_owned(),
            user: None,
            port: None,
            jump: Vec::new(),
        },
    };
    let ssh = tunwarden_core::tools::resolve(
        &tunwarden_core::types::Program::Tool(tunwarden_core::types::Tool::Ssh),
        &tool_paths,
        &SystemEnv,
        Os::current(),
    )
    .map_err(|e| e.to_string())?
    .path;

    eprintln!(
        "Connecting to {} to check its host key. ssh asks before trusting a new key.",
        ssh_target.host
    );
    // No authentication is attempted, so no password or passphrase is asked for.
    // ssh asks its yes/no question on the terminal itself; only its error
    // output is filtered, to hide the expected "Permission denied".
    let asked = std::process::Command::new(&ssh)
        .args(trust_args(&ssh_target, "ask"))
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", ssh.display()))?;
    for line in String::from_utf8_lossy(&asked.stderr).lines() {
        if !line.contains("Permission denied") {
            eprintln!("{line}");
        }
    }

    let check = std::process::Command::new(&ssh)
        .args(trust_args(&ssh_target, "yes"))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", ssh.display()))?;
    let err = String::from_utf8_lossy(&check.stderr);
    if err.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return Err(Failure {
            message: format!(
                "the host key of {} has changed since it was trusted",
                ssh_target.host
            ),
            hint: Some(format!(
                "if you know why it changed, remove the old key with `ssh-keygen -R {}` and run this again",
                ssh_target.host
            )),
        });
    }
    if err.contains("Host key verification failed") {
        return Err(Failure {
            message: format!("the host key of {} is still not trusted", ssh_target.host),
            hint: Some("answer `yes` when ssh asks, after checking the fingerprint".into()),
        });
    }
    if err.contains("Could not resolve hostname")
        || err.contains("Connection refused")
        || err.contains("timed out")
    {
        let line = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        return Err(format!("cannot reach {}: {line}", ssh_target.host).into());
    }
    println!("the host key of {} is trusted", ssh_target.host);
    Ok(ExitCode::SUCCESS)
}

/// ssh arguments that check only the host key: no authentication methods, so
/// the connection ends with "Permission denied" once the key is accepted.
fn trust_args(t: &tunwarden_core::types::SshTarget, strict: &str) -> Vec<String> {
    let mut args = vec![
        "-o".to_owned(),
        format!("StrictHostKeyChecking={strict}"),
        "-o".into(),
        format!("BatchMode={}", if strict == "ask" { "no" } else { "yes" }),
        "-o".into(),
        "PreferredAuthentications=none".into(),
        "-o".into(),
        "ControlMaster=no".into(),
        "-o".into(),
        "ControlPath=none".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
    ];
    if let Some(p) = t.port {
        args.push("-p".into());
        args.push(p.to_string());
    }
    if let Some(u) = &t.user {
        args.push("-l".into());
        args.push(u.clone());
    }
    if !t.jump.is_empty() {
        args.push("-J".into());
        args.push(t.jump.join(","));
    }
    args.push(t.host.clone());
    args.push("true".into());
    args
}

// --- Logins (FR-22, FR-25) -------------------------------------------------------

/// The login named `name`, or the login of the profile named `name`.
fn find_login<'c>(config: &'c Config, name: &str) -> Result<&'c Login, Failure> {
    if let Some(l) = config.login(name) {
        return Ok(l);
    }
    if let Some(p) = config.profile(name) {
        return match p.login.as_deref().and_then(|l| config.login(l)) {
            Some(l) => Ok(l),
            None => Err(Failure {
                message: format!("profile \"{name}\" has no login"),
                hint: Some("set `login = \"NAME\"` in the profile to use one".into()),
            }),
        };
    }
    let known: Vec<&str> = config.logins.iter().map(|l| l.name.as_str()).collect();
    Err(Failure {
        message: format!("no login or profile named \"{name}\""),
        hint: Some(if known.is_empty() {
            "no logins are configured; add a [[logins]] entry".into()
        } else {
            format!("configured logins: {}", known.join(", "))
        }),
    })
}

fn login_view(config: &Config, login: &Login, outcome: Option<&AuthOutcome>) -> LoginView {
    LoginView {
        name: login.name.clone(),
        profiles: config
            .profiles
            .iter()
            .filter(|p| p.login.as_ref() == Some(&login.name))
            .map(|p| p.name.clone())
            .collect(),
        checkable: login.check.is_some(),
        result: outcome.map_or("not checked", AuthOutcome::name).to_owned(),
        detail: outcome.and_then(AuthOutcome::detail),
        checked_at: outcome.map(|_| tunwarden_core::fsutil::format_timestamp_now()),
        active_tunnels: Vec::new(),
    }
}

/// Checks logins directly, without a daemon.
async fn check_logins(config: &Config, name: Option<&str>) -> Result<AuthResult, Failure> {
    let logins: Vec<&Login> = match name {
        Some(n) => vec![find_login(config, n)?],
        None => config.logins.iter().collect(),
    };
    let mut views = Vec::new();
    for l in logins {
        let outcome = l
            .check
            .is_some()
            .then_some(gauth::check(l, &config.tools).await);
        views.push(login_view(config, l, outcome.as_ref()));
    }
    Ok(AuthResult { logins: views })
}

/// Checks logins through the daemon if it runs (so it acts on the result),
/// otherwise directly.
async fn auth(ctx: &Ctx, name: Option<String>, json: bool) -> Outcome {
    let config = load(ctx)?;
    let result: AuthResult = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(mut c) => c.call(method::AUTH_CHECK, AuthParams { name }).await?,
        Err(ClientError::NotRunning) => check_logins(&config, name.as_deref()).await?,
        Err(e) => return Err(e.into()),
    };
    if json {
        print_json(&result);
    } else if result.logins.is_empty() {
        println!("no logins are configured");
    } else {
        let mut table = Table::new(&["LOGIN", "PROFILES", "RESULT", "DETAIL"]);
        for l in &result.logins {
            let detail = match l.result.as_str() {
                "expired" => format!(
                    "{}; run `tunwarden login {}`",
                    l.detail.clone().unwrap_or_default(),
                    l.name
                ),
                _ if !l.checkable => "no check command".into(),
                _ => l.detail.clone().unwrap_or_default(),
            };
            table.row(vec![
                l.name.clone(),
                or_dash(l.profiles.join(", ")),
                l.result.clone(),
                detail,
            ]);
        }
        print!("{table}");
    }
    let expired = result.logins.iter().any(|l| l.result == "expired");
    Ok(if expired {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Runs the login's `login` command here in the terminal, then lets the
/// daemon check it and restart the tunnels that were waiting for it (FR-25).
async fn login(ctx: &Ctx, name: &str) -> Outcome {
    let config = load(ctx)?;
    let login = find_login(&config, name)?;
    let Some(words) = &login.login else {
        return Err(Failure {
            message: format!("the {} login has no `login` command", login.name),
            hint: Some(format!(
                "log in with your tool, then run `tunwarden auth {}`",
                login.name
            )),
        });
    };
    let program = tunwarden_core::tools::resolve(
        &tunwarden_core::types::Program::Command(words[0].clone()),
        &config.tools,
        &SystemEnv,
        Os::current(),
    )
    .map_err(|e| e.to_string())?
    .path;

    let view = login_view(&config, login, None);
    let used_by = if view.profiles.is_empty() {
        String::new()
    } else {
        format!(" (used by {})", view.profiles.join(", "))
    };
    eprintln!(
        "Running the {} login{used_by}: {}",
        login.name,
        shell_line(words)
    );
    let status = gauth::login_command(login, &program)
        .expect("checked above")
        .stdin(std::process::Stdio::inherit())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if !status.success() {
        return Err(Failure {
            message: format!("the login command did not finish ({status})"),
            hint: Some(format!("run `tunwarden login {}` again", login.name)),
        });
    }

    match Client::connect(&ctx.state_dir, "cli").await {
        Ok(mut c) => {
            let result: RecoveredResult = c
                .call(
                    method::AUTH_RECOVERED,
                    RecoveredParams {
                        login: login.name.clone(),
                    },
                )
                .await?;
            if result.restarted.is_empty() {
                println!("logged in; no tunnels were waiting for this login");
            } else {
                println!("logged in; restarting {}", result.restarted.join(", "));
            }
        }
        Err(ClientError::NotRunning) => match gauth::check(login, &config.tools).await {
            AuthOutcome::Ok => println!("logged in"),
            other => {
                return Err(Failure {
                    message: format!(
                        "the {} login still fails its check: {}",
                        login.name,
                        other.detail().unwrap_or_default()
                    ),
                    hint: Some(format!("run `tunwarden login {}` again", login.name)),
                });
            }
        },
        Err(e) => return Err(e.into()),
    }
    Ok(ExitCode::SUCCESS)
}

fn shell_line(words: &[String]) -> String {
    words
        .iter()
        .map(|w| tunwarden_core::types::quote(w))
        .collect::<Vec<_>>()
        .join(" ")
}

// --- daemon ------------------------------------------------------------------

async fn daemon_command(ctx: &Ctx, cmd: DaemonCommand) -> Outcome {
    match cmd {
        DaemonCommand::Run => {
            let opts = daemon::Options {
                config_path: ctx.config.clone(),
                state_dir: ctx.state_dir.clone(),
                handle_signals: true,
                auth_interval: None,
            };
            match daemon::run(opts).await {
                Ok(()) => Ok(ExitCode::SUCCESS),
                Err(e @ DaemonError::AlreadyRunning(_)) => Err(Failure {
                    message: e.to_string(),
                    hint: Some("use `tunwarden daemon status`".into()),
                }),
                Err(e) => Err(e.to_string().into()),
            }
        }
        DaemonCommand::Start => {
            let client = connect_or_start(ctx).await?;
            let h = client.hello();
            println!(
                "daemon running (PID {}, version {})",
                h.pid, h.daemon_version
            );
            Ok(ExitCode::SUCCESS)
        }
        DaemonCommand::Stop { keep } => {
            if stop_daemon(ctx, keep).await? {
                let extra = if keep {
                    "; running tunnels will be restored on the next start"
                } else {
                    ""
                };
                println!("daemon stopped{extra}");
            } else {
                println!("daemon is not running");
            }
            Ok(ExitCode::SUCCESS)
        }
        DaemonCommand::Restart => {
            stop_daemon(ctx, true).await?;
            let client = connect_or_start(ctx).await?;
            println!("daemon restarted (PID {})", client.hello().pid);
            Ok(ExitCode::SUCCESS)
        }
        DaemonCommand::Status => match Client::connect(&ctx.state_dir, "cli").await {
            Ok(client) => {
                let h = client.hello();
                println!("daemon running");
                println!("  PID:       {}", h.pid);
                println!(
                    "  version:   {} (protocol {})",
                    h.daemon_version, h.protocol
                );
                println!("  config:    {}", h.config_path);
                println!("  state dir: {}", h.state_dir);
                Ok(ExitCode::SUCCESS)
            }
            Err(ClientError::NotRunning) => {
                println!("daemon is not running");
                Ok(ExitCode::from(EXIT_NOT_RUNNING))
            }
            Err(e) => Err(e.into()),
        },
    }
}

/// Asks the daemon to stop and waits until it is gone. `false` if it was not running.
async fn stop_daemon(ctx: &Ctx, keep: bool) -> Result<bool, Failure> {
    let mut client = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(c) => c,
        Err(ClientError::NotRunning) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let _: Value = client
        .call(method::SHUTDOWN, ShutdownParams { keep })
        .await?;
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match Client::connect(&ctx.state_dir, "cli").await {
            Err(ClientError::NotRunning) => return Ok(true),
            _ if Instant::now() > deadline => {
                return Err("the daemon did not stop within 30 seconds"
                    .to_owned()
                    .into());
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

// --- helpers -------------------------------------------------------------------

fn print_json(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

fn or_dash(s: String) -> String {
    if s.is_empty() { "-".into() } else { s }
}

/// `45s`, `12m 5s`, `3h 20m`, `2d 4h`.
fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        3600..86400 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(human_duration(Duration::from_secs(45)), "45s");
        assert_eq!(human_duration(Duration::from_secs(725)), "12m 5s");
        assert_eq!(human_duration(Duration::from_secs(12_000)), "3h 20m");
        assert_eq!(human_duration(Duration::from_secs(187_200)), "2d 4h");
    }

    #[test]
    fn trust_checks_only_the_host_key() {
        let t = tunwarden_core::types::SshTarget {
            host: "10.0.0.5".into(),
            user: Some("deploy".into()),
            port: Some(2222),
            jump: vec!["jump1".into(), "jump2".into()],
        };
        assert_eq!(
            trust_args(&t, "ask").join(" "),
            "-o StrictHostKeyChecking=ask -o BatchMode=no -o PreferredAuthentications=none -o ControlMaster=no \
             -o ControlPath=none -o ConnectTimeout=15 -p 2222 -l deploy -J jump1,jump2 10.0.0.5 true"
        );
        assert!(trust_args(&t, "yes").contains(&"BatchMode=yes".to_owned()));
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
