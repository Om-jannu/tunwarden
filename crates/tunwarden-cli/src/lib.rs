//! The `tunwarden` command line (PRD 7.7). Kept in a library so the system
//! tests can run exactly this code next to their fake tunnels.

mod table;
mod ui;

// Print through anstream, which drops colour codes when the output is not a
// terminal, NO_COLOR is set, or TERM=dumb.
use anstream::{eprintln, print, println};

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};
use serde_json::Value;
use tunwarden_core::auth::{self as gauth, Outcome as AuthOutcome};
use tunwarden_core::client::{Client, ClientError, Launcher};
use tunwarden_core::config::{self, Config, LoadError, Login};
use tunwarden_core::daemon::{self, DaemonError, tunnel_info};
use tunwarden_core::fsutil::parse_timestamp;
use tunwarden_core::paths::{self, Os, SystemEnv};
use tunwarden_core::proto::{
    ActionResult, AuthParams, AuthResult, ConfigAddEntryParams, ConfigAddParams, ConfigEditParams,
    ConfigEditResult, ConfigRemoveParams, ListResult, LogNotice, LoginView, LogsParams, LogsResult,
    RecoveredParams, RecoveredResult, Selector, ShutdownParams, StatusParams, StatusResult,
    SubscribeParams, TunnelInfo, TunnelView, method, notice,
};
use tunwarden_core::state::Desired;
use tunwarden_core::supervisor::State;

use table::Table;

/// Exit code when the daemon is not running (`daemon status`).
const EXIT_NOT_RUNNING: u8 = 3;
/// Exit code when the user cancels a prompt, as for Ctrl-C in a shell.
const EXIT_CANCELLED: u8 = 130;

const EXAMPLES_START: &str = "Examples:
  tunwarden start staging-db bastion-redis
  tunwarden start --group daily
  tunwarden start --profile prod --yes
  tunwarden start            pick from a list";
const EXAMPLES_ADD: &str = "Examples:
  tunwarden add                                   asks for each field
  tunwarden add grafana --command \"kubectl port-forward svc/grafana 3000:80\" --forward 3000
  tunwarden add redis --type ssh-local --set host=bastion --forward 6380:redis.internal:6379";
const EXAMPLES_COMPLETIONS: &str = "Setup:
  bash: echo 'source <(tunwarden completions bash)' >> ~/.bashrc
  zsh:  echo 'source <(tunwarden completions zsh)' >> ~/.zshrc
  fish: tunwarden completions fish > ~/.config/fish/completions/tunwarden.fish
  PowerShell: tunwarden completions powershell | Out-String | Invoke-Expression";
/// The example config written by `init`.
const EXAMPLE_CONFIG: &str = include_str!("../example-config.toml");

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
    command: Option<Cmd>,
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
    #[command(after_help = EXAMPLES_START)]
    Start(Action),
    /// Stop tunnels
    #[command(
        after_help = "Examples:\n  tunwarden stop staging-db\n  tunwarden stop --all\n  tunwarden stop          pick from the running tunnels"
    )]
    Stop(Action),
    /// Restart tunnels
    Restart(Action),
    /// Show a tunnel's log
    #[command(
        after_help = "Examples:\n  tunwarden logs staging-db -f\n  tunwarden logs staging-db -n 200 | grep -i error\n  tunwarden logs          pick a tunnel"
    )]
    Logs {
        /// The tunnel; asks when missing
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: Option<String>,
        /// Number of lines to show
        #[arg(short = 'n', long = "lines", default_value_t = 50)]
        lines: usize,
        /// Keep printing new lines
        #[arg(short, long)]
        follow: bool,
    },
    /// Print the command a tunnel runs
    ShowCommand {
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: String,
    },
    /// Add a tunnel to the config
    #[command(after_help = EXAMPLES_ADD)]
    Add {
        /// The new tunnel's name; asks when missing
        name: Option<String>,
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
    Remove {
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: String,
    },
    /// Turn pasted tunnel commands or aliases into config entries
    #[command(
        after_help = "Examples:\n  tunwarden import ~/.zshrc            preview the entries\n  tunwarden import ~/.zshrc --write    pick which ones to add\n  pbpaste | tunwarden import"
    )]
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
        #[arg(add = ArgValueCandidates::new(complete_logins))]
        name: Option<String>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a login's `login` command, then restart the tunnels waiting for it
    #[command(
        after_help = "Examples:\n  tunwarden login google\n  tunwarden login staging   a profile means its login\n  tunwarden login           pick a login"
    )]
    Login {
        /// A login, or a profile meaning its login; asks when missing
        #[arg(add = ArgValueCandidates::new(complete_logins))]
        name: Option<String>,
    },
    /// Print a shell completion script; completes your tunnel, group, profile and login names
    #[command(after_help = EXAMPLES_COMPLETIONS)]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Write man pages for every command to DIR
    #[command(hide = true)]
    Man { dir: PathBuf },
    /// Manage the background daemon that owns all tunnels
    #[command(subcommand)]
    Daemon(DaemonCommand),
    /// Start the daemon at login, so tunnels come back after a reboot
    #[command(subcommand)]
    Autostart(AutostartCommand),
}

#[derive(Args)]
struct Action {
    /// Tunnel names; asks when none are given and no --all, --group or --profile
    #[arg(add = ArgValueCandidates::new(complete_tunnels))]
    names: Vec<String>,
    /// All tunnels
    #[arg(long)]
    all: bool,
    /// The tunnels in a group
    #[arg(long, add = ArgValueCandidates::new(complete_groups))]
    group: Option<String>,
    /// The tunnels of a profile
    #[arg(long, add = ArgValueCandidates::new(complete_profiles))]
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

#[derive(Subcommand)]
enum AutostartCommand {
    /// Start the daemon at every login (launchd, systemd user unit, or the Windows Run key)
    Enable,
    /// Stop starting the daemon at login
    Disable,
    /// Show whether the daemon starts at login; exits 3 if it does not
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

impl From<ui::Cancelled> for Failure {
    fn from(_: ui::Cancelled) -> Failure {
        Failure {
            message: "cancelled".into(),
            hint: None,
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
    // `COMPLETE=<shell> tunwarden ...` is the shell asking for completions.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse_from(args);
    let Some(command) = cli.command else {
        return welcome();
    };
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
    match runtime.block_on(dispatch(&ctx, command)) {
        Ok(code) => code,
        Err(e) => report(e),
    }
}

/// A bare `tunwarden`: what it is and where to start.
fn welcome() -> ExitCode {
    let b = |s: &str| ui::paint(ui::BOLD, s);
    const START: [(&str, &str); 4] = [
        ("tunwarden init", "write an example config"),
        (
            "tunwarden import ~/.zshrc",
            "turn your tunnel aliases into entries",
        ),
        ("tunwarden start", "start, then see what runs"),
        ("tunwarden doctor", "check tools, logins and ports"),
    ];
    let width = START.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    println!("{} keeps your tunnels running.", b("Tunwarden"));
    println!();
    for (command, what) in START {
        // Padded before painting, so the colour codes do not count as width.
        println!("  {}  {what}", b(&format!("{command:<width$}")));
    }
    println!();
    println!("Every command: {}", b("tunwarden --help"));
    ExitCode::SUCCESS
}

fn report(e: Failure) -> ExitCode {
    if e.message == "cancelled" && e.hint.is_none() {
        return ExitCode::from(EXIT_CANCELLED);
    }
    eprintln!("{} {}", ui::paint(ui::RED.bold(), "error:"), e.message);
    if let Some(hint) = e.hint {
        eprintln!("{} {hint}", ui::paint(ui::CYAN, "hint:"));
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
        Cmd::Login { name } => {
            let name = match name {
                Some(n) => n,
                None => pick_login(ctx)?,
            };
            login(ctx, &name).await
        }
        Cmd::Status { names, json } => status(ctx, names, json).await,
        Cmd::Start(a) => action(ctx, Kind::Start, a).await,
        Cmd::Stop(a) => action(ctx, Kind::Stop, a).await,
        Cmd::Restart(a) => action(ctx, Kind::Restart, a).await,
        Cmd::Logs {
            name,
            lines,
            follow,
        } => {
            let name = match name {
                Some(n) => n,
                None => pick_tunnel(ctx, "Show the log of")?,
            };
            logs(ctx, &name, lines, follow).await
        }
        Cmd::Completions { shell } => {
            let shells = clap_complete::env::Shells::builtins();
            let completer = shells
                .completer(&shell.to_string())
                .ok_or_else(|| format!("no completions for {shell}"))?;
            completer
                .write_registration(
                    "COMPLETE",
                    "tunwarden",
                    "tunwarden",
                    "tunwarden",
                    &mut std::io::stdout(),
                )
                .map_err(|e| format!("cannot write the script: {e}"))?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Man { dir } => {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            clap_mangen::generate_to(Cli::command(), &dir)
                .map_err(|e| format!("cannot write man pages: {e}"))?;
            println!("wrote man pages to {}", dir.display());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Daemon(cmd) => daemon_command(ctx, cmd).await,
        Cmd::Autostart(cmd) => autostart_command(ctx, cmd),
    }
}

// --- Autostart (FR-15) -------------------------------------------------------

fn autostart_command(ctx: &Ctx, cmd: AutostartCommand) -> Outcome {
    use tunwarden_core::autostart;
    match cmd {
        AutostartCommand::Enable => {
            let exe =
                std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?;
            let entry = autostart::Entry {
                exe,
                config: ctx.config.clone(),
                state_dir_override: std::env::var_os(paths::STATE_DIR_ENV)
                    .filter(|v| !v.is_empty())
                    .map(|_| ctx.state_dir.clone()),
                log: ctx.state_dir.join("daemon.log"),
                path: std::env::var_os("PATH").filter(|_| Os::current() != Os::Windows),
                app: false,
            };
            let done = autostart::enable(&entry, &SystemEnv).map_err(|e| e.to_string())?;
            println!("autostart enabled: {}", done.location);
            println!("at login it runs: {}", done.command);
            if entry.path.is_some() {
                println!("it uses the PATH of this shell, so the daemon finds the same tools");
            }
            for note in &done.notes {
                println!("note: {note}");
            }
            if let Some(w) = &done.warning {
                eprintln!("warning: {w}");
            }
            Ok(ExitCode::SUCCESS)
        }
        AutostartCommand::Disable => {
            match autostart::disable(&SystemEnv).map_err(|e| e.to_string())? {
                Some(location) => {
                    println!("autostart disabled; removed {location}");
                    println!("the daemon running now is not changed");
                }
                None => println!("autostart was not enabled"),
            }
            Ok(ExitCode::SUCCESS)
        }
        AutostartCommand::Status => {
            let s = autostart::status(&SystemEnv).map_err(|e| e.to_string())?;
            if !s.enabled {
                println!("autostart is not enabled");
                println!("hint: run `tunwarden autostart enable` to start the daemon at login");
                return Ok(ExitCode::from(EXIT_NOT_RUNNING));
            }
            println!("autostart is enabled: {}", s.location);
            if let Some(exe) = &s.exe {
                println!("it starts {}", exe.display());
            }
            match s.problem {
                Some(problem) => Err(problem.into()),
                None => Ok(ExitCode::SUCCESS),
            }
        }
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
            message: config_errors_message(&ctx.config, &errors),
            hint: None,
        },
    })
}

/// The problems in the config file: drawn with the lines they are on when
/// stderr is a terminal, one `file:line:column: ...` per line otherwise.
fn config_errors_message(path: &std::path::Path, errors: &config::ConfigErrors) -> String {
    let count = errors.0.len();
    let drawn = std::io::stderr()
        .is_terminal()
        .then(|| ui::render_config_errors(path, errors))
        .flatten();
    match drawn {
        Some(d) => format!(
            "the config file has {count} problem(s):\n\n{}",
            d.trim_end()
        ),
        None => format!("the config file has {count} problem(s):\n{errors}"),
    }
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
        eprintln!("{} {e}", ui::paint(ui::RED, "config:"));
    }
    for n in &result.notes {
        eprintln!("{} {n}", ui::paint(ui::DIM, "note:"));
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
            ui::state(t.state),
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

async fn action(ctx: &Ctx, kind: Kind, mut a: Action) -> Outcome {
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

    // Nothing selected: pick from a list on a terminal. Otherwise the daemon
    // answers "no tunnels selected" with the hint.
    let nothing = a.names.is_empty() && !a.all && a.group.is_none() && a.profile.is_none();
    if nothing && ui::interactive() {
        let all: StatusResult = client
            .call(method::STATUS, StatusParams { names: Vec::new() })
            .await?;
        let active = |t: &&TunnelView| t.state != State::Stopped;
        let choices: Vec<&TunnelView> = match kind {
            Kind::Stop => all.tunnels.iter().filter(active).collect(),
            _ => all.tunnels.iter().collect(),
        };
        if choices.is_empty() {
            println!(
                "{}",
                if kind == Kind::Stop {
                    "no tunnels are running"
                } else {
                    "no tunnels are configured"
                }
            );
            return Ok(ExitCode::SUCCESS);
        }
        let width = choices.iter().map(|t| t.name.len()).max().unwrap_or(0);
        let items: Vec<String> = choices
            .iter()
            .map(|t| {
                format!(
                    "{:<width$}  {}  {}",
                    t.name,
                    ui::state_padded(t.state, 12),
                    ui::paint(ui::DIM, t.local.join(", "))
                )
            })
            .collect();
        let checked = vec![false; items.len()];
        let picked = ui::multi_select(&format!("Which tunnels to {verb}?"), &items, &checked)?;
        if picked.is_empty() {
            println!("nothing selected");
            return Ok(ExitCode::SUCCESS);
        }
        a.names = picked
            .into_iter()
            .map(|i| choices[i].name.clone())
            .collect();
    }

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
    let progress = ui::live().then(|| Progress::new(&done.tunnels));
    let views = loop {
        let now: StatusResult = client
            .call(
                method::STATUS,
                StatusParams {
                    names: done.tunnels.clone(),
                },
            )
            .await?;
        if let Some(p) = &progress {
            p.update(&now.tunnels, &settled);
        }
        if now.tunnels.iter().all(settled) || Instant::now() >= deadline {
            break now.tunnels;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    let mut table = Table::new(&["NAME", "STATE", "LOCAL", "DETAIL"]);
    let mut ok = true;
    let mut results = Vec::new();
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
        results.push((t.name.clone(), good, d.clone()));
        table.row(vec![
            t.name.clone(),
            ui::state(t.state),
            or_dash(t.local.join(", ")),
            d,
        ]);
    }
    // On a terminal the finished progress lines are the result; a pipe gets
    // the table.
    let drawn = progress.is_some_and(|p| p.finish(&views, &results));
    if !(drawn && std::io::stdout().is_terminal()) {
        print!("{table}");
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// One live line per tunnel on stderr while `start`, `stop` or `restart`
/// waits, ending as `✓ name running 127.0.0.1:5433` or `✗ name failed ...`.
struct Progress {
    bars: Vec<(String, indicatif::ProgressBar)>,
    width: usize,
}

impl Progress {
    fn new(names: &[String]) -> Progress {
        let multi =
            indicatif::MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::stderr());
        let style = indicatif::ProgressStyle::with_template("{spinner:.yellow} {msg}")
            .expect("a valid template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
        let width = names.iter().map(|n| n.len()).max().unwrap_or(0);
        let bars = names
            .iter()
            .map(|n| {
                let bar = multi.add(indicatif::ProgressBar::new_spinner());
                bar.set_style(style.clone());
                bar.set_message(format!("{n:<width$}  waiting"));
                bar.enable_steady_tick(Duration::from_millis(100));
                (n.clone(), bar)
            })
            .collect();
        Progress { bars, width }
    }

    fn update(&self, views: &[TunnelView], settled: &dyn Fn(&TunnelView) -> bool) {
        let width = self.width;
        for (name, bar) in &self.bars {
            if let Some(t) = views.iter().find(|t| &t.name == name)
                && !settled(t)
                && t.state != State::Stopped
            {
                bar.set_message(format!(
                    "{name:<width$}  {}  {}",
                    ui::state_padded(t.state, 12),
                    ui::paint(ui::DIM, t.local.join(", "))
                ));
            }
        }
    }

    /// Ends every line. Returns true when lines were drawn.
    fn finish(self, views: &[TunnelView], results: &[(String, bool, String)]) -> bool {
        let width = self.width;
        let done = indicatif::ProgressStyle::with_template("{msg}").expect("a valid template");
        for (name, bar) in &self.bars {
            let view = views.iter().find(|t| &t.name == name);
            let (good, detail) = results
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, g, d)| (*g, d.as_str()))
                .unwrap_or((false, ""));
            let mark = if good {
                ui::paint(ui::GREEN, "✓")
            } else {
                ui::paint(ui::RED, "✗")
            };
            let state = view
                .map(|t| ui::state_padded(t.state, 12))
                .unwrap_or_default();
            let local = view.map(|t| t.local.join(", ")).unwrap_or_default();
            let tail = if detail.is_empty() {
                ui::paint(ui::DIM, local)
            } else {
                format!("{}  {detail}", ui::paint(ui::DIM, local))
            };
            bar.set_style(done.clone());
            bar.finish_with_message(format!("{mark} {name:<width$}  {state}  {tail}"));
        }
        !self.bars.is_empty()
    }
}

fn confirm(verb: &str, production: &[&str]) -> Result<(), Failure> {
    let list = production.join(", ");
    if !std::io::stdin().is_terminal() {
        return Err(Failure {
            message: format!("refusing to {verb} production tunnels ({list}) without confirmation"),
            hint: Some("pass --yes to confirm".into()),
        });
    }
    let question = format!(
        "{} belong to a production profile. {verb} anyway?",
        ui::paint(ui::YELLOW.bold(), &list)
    );
    if ui::confirm(&question)? {
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
            let pretty = std::io::stdout().is_terminal();
            for line in tunwarden_core::logs::tail(&path, lines)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            {
                println!("{}", log_line(&line, pretty));
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
    let pretty = std::io::stdout().is_terminal();
    for line in &past.lines {
        println!("{}", log_line(line, pretty));
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
        println!("{}", log_line(&l.line, pretty));
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
    #[arg(long, add = ArgValueCandidates::new(complete_profiles))]
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

async fn add(ctx: &Ctx, name: Option<String>, fields: Fields) -> Outcome {
    let mut fields = fields.to_map()?;
    let name = match name {
        Some(n) => n,
        None if ui::interactive() => ui::input("Tunnel name", false)?.trim().to_owned(),
        None => {
            return Err(Failure {
                message: "name the new tunnel".into(),
                hint: Some("for example `tunwarden add grafana --command \"...\" --forward 3000`, or run it in a terminal to be asked".into()),
            });
        }
    };
    if fields.is_empty() && ui::interactive() {
        fields = ask_fields(ctx)?;
    }
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

use tunwarden_core::doctor::Mark;

fn print_report(report: &tunwarden_core::doctor::Report) -> bool {
    let mut section = "";
    for row in &report.rows {
        if row.section != section {
            if !section.is_empty() {
                println!();
            }
            println!("{}", ui::paint(ui::BOLD, &row.section));
            section = &row.section;
        }
        let (label, style) = match row.mark {
            Mark::Ok => ("ok", ui::GREEN),
            Mark::Warning => ("warning", ui::YELLOW),
            Mark::Problem => ("problem", ui::RED),
        };
        let line = format!(
            "  {} {:<24} {}",
            ui::paint(style, format!("{label:<8}")),
            row.item,
            row.detail
        );
        println!("{}", line.trim_end());
    }
    report.has_problems()
}

async fn doctor(ctx: &Ctx) -> Outcome {
    let exe = std::env::current_exe().ok();
    let report =
        tunwarden_core::doctor::run(&ctx.config, &ctx.state_dir, "cli", exe.as_deref()).await;
    let problems = print_report(&report);
    println!();
    if problems {
        println!("Something needs fixing: see the problems above.");
        Ok(ExitCode::FAILURE)
    } else {
        println!("All clear.");
        Ok(ExitCode::SUCCESS)
    }
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
    // From a file on a terminal: choose which entries to add.
    let mut tunnels: Vec<_> = imported.tunnels.iter().collect();
    let from_file = file.as_ref().is_some_and(|f| f.as_os_str() != "-");
    if from_file && ui::interactive() {
        let width = tunnels.iter().map(|t| t.name.len()).max().unwrap_or(0);
        let items: Vec<String> = tunnels
            .iter()
            .map(|t| {
                let what = t
                    .fields
                    .get("command")
                    .or_else(|| t.fields.get("host"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let short: String = what.chars().take(60).collect();
                format!("{:<width$}  {}", t.name, ui::paint(ui::DIM, short))
            })
            .collect();
        let picked = ui::multi_select("Add which tunnels?", &items, &vec![true; items.len()])?;
        if picked.is_empty() {
            println!("nothing added");
            return Ok(ExitCode::SUCCESS);
        }
        tunnels = picked.into_iter().map(|i| tunnels[i]).collect();
    }
    let used: Vec<&str> = tunnels
        .iter()
        .filter_map(|t| t.fields.get("profile").and_then(Value::as_str))
        .collect();
    let mut client = connect_or_start(ctx).await?;
    for p in imported
        .profiles
        .iter()
        .filter(|p| used.contains(&p.name.as_str()))
    {
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
    for t in tunnels {
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
            message: config_errors_message(&ctx.config, &errors),
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
    gauth::find(config, name).map_err(|e| Failure {
        message: e.message,
        hint: e.hint,
    })
}

fn login_view(config: &Config, login: &Login, outcome: Option<&AuthOutcome>) -> LoginView {
    gauth::view(config, login, outcome)
}

/// Checks logins directly, without a daemon.
async fn check_logins(config: &Config, name: Option<&str>) -> Result<AuthResult, Failure> {
    match name {
        Some(n) => {
            let l = find_login(config, n)?;
            let outcome = l
                .check
                .is_some()
                .then_some(gauth::check(l, &config.tools).await);
            Ok(AuthResult {
                logins: vec![login_view(config, l, outcome.as_ref())],
            })
        }
        None => Ok(gauth::check_all(config).await),
    }
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
            let result = match l.result.as_str() {
                "ok" => ui::paint(ui::GREEN, "ok"),
                "expired" => ui::paint(ui::RED, "expired"),
                "unknown" => ui::paint(ui::YELLOW, "unknown"),
                other => ui::paint(ui::DIM, other),
            };
            let checked = l
                .checked_at
                .as_deref()
                .and_then(parse_timestamp)
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .map(|d| format!(" (checked {} ago)", human_duration(d)))
                .unwrap_or_default();
            table.row(vec![
                l.name.clone(),
                or_dash(l.profiles.join(", ")),
                result,
                format!("{detail}{}", ui::paint(ui::DIM, checked)),
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
                system_events: None,
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

// --- Pickers and completions -------------------------------------------------

/// The config for completions: the `TUNWARDEN_CONFIG` or default file. The
/// `--config` flag is not visible to the completion engine.
fn completion_config() -> Option<Config> {
    let path = paths::config_file(None, &SystemEnv, Os::current()).ok()?;
    config::load(&path).ok()
}

fn complete_tunnels() -> Vec<CompletionCandidate> {
    let Some(c) = completion_config() else {
        return Vec::new();
    };
    c.tunnels
        .iter()
        .map(|t| {
            let info = tunnel_info(&c, t);
            CompletionCandidate::new(&t.name).help(Some(
                format!("{} {}", info.kind, info.local.join(", ")).into(),
            ))
        })
        .collect()
}

fn complete_groups() -> Vec<CompletionCandidate> {
    let Some(c) = completion_config() else {
        return Vec::new();
    };
    let mut groups: Vec<&String> = c.tunnels.iter().flat_map(|t| &t.groups).collect();
    groups.sort();
    groups.dedup();
    groups.into_iter().map(CompletionCandidate::new).collect()
}

fn complete_profiles() -> Vec<CompletionCandidate> {
    completion_config()
        .map(|c| {
            c.profiles
                .iter()
                .map(|p| CompletionCandidate::new(&p.name))
                .collect()
        })
        .unwrap_or_default()
}

fn complete_logins() -> Vec<CompletionCandidate> {
    completion_config()
        .map(|c| {
            let mut v: Vec<_> = c
                .logins
                .iter()
                .map(|l| CompletionCandidate::new(&l.name).help(Some("login".into())))
                .collect();
            v.extend(
                c.profiles
                    .iter()
                    .map(|p| CompletionCandidate::new(&p.name).help(Some("profile".into()))),
            );
            v
        })
        .unwrap_or_default()
}

/// A missing NAME: asked for on a terminal, an error with the choices otherwise.
fn pick_tunnel(ctx: &Ctx, prompt: &str) -> Result<String, Failure> {
    let config = load(ctx)?;
    let names: Vec<String> = config.tunnels.iter().map(|t| t.name.clone()).collect();
    if names.is_empty() {
        return Err(Failure {
            message: "no tunnels are configured".into(),
            hint: Some("add one with `tunwarden add` or `tunwarden import`".into()),
        });
    }
    if !ui::interactive() {
        return Err(Failure {
            message: "name a tunnel".into(),
            hint: Some(format!("configured tunnels: {}", names.join(", "))),
        });
    }
    let items: Vec<String> = config
        .tunnels
        .iter()
        .map(|t| {
            let info = tunnel_info(&config, t);
            format!(
                "{:<20} {}",
                t.name,
                ui::paint(ui::DIM, info.local.join(", "))
            )
        })
        .collect();
    let i = ui::select(prompt, &items)?;
    Ok(names[i].clone())
}

fn pick_login(ctx: &Ctx) -> Result<String, Failure> {
    let config = load(ctx)?;
    if config.logins.is_empty() {
        return Err(Failure {
            message: "no logins are configured".into(),
            hint: Some("add a [[logins]] entry to the config".into()),
        });
    }
    let names: Vec<String> = config.logins.iter().map(|l| l.name.clone()).collect();
    if !ui::interactive() {
        return Err(Failure {
            message: "name a login".into(),
            hint: Some(format!("configured logins: {}", names.join(", "))),
        });
    }
    let items: Vec<String> = config
        .logins
        .iter()
        .map(|l| {
            let view = login_view(&config, l, None);
            format!(
                "{:<16} {}",
                l.name,
                ui::paint(
                    ui::DIM,
                    format!("used by {}", or_dash(view.profiles.join(", ")))
                )
            )
        })
        .collect();
    let i = ui::select("Log in to", &items)?;
    Ok(names[i].clone())
}

/// A stored log line (`<RFC 3339 UTC> <source> <text>`), for a terminal: the
/// time in the local time zone and the source coloured. Unchanged otherwise,
/// so pipes get the exact stored lines.
fn log_line(line: &str, pretty: bool) -> String {
    if !pretty {
        return line.to_owned();
    }
    let mut parts = line.splitn(3, ' ');
    let (Some(ts), Some(source), rest) = (parts.next(), parts.next(), parts.next()) else {
        return line.to_owned();
    };
    let Ok(t) = ts.parse::<jiff::Timestamp>() else {
        return line.to_owned();
    };
    let local = t.to_zoned_datetime(jiff::tz::TimeZone::system());
    let time = local.strftime("%m-%d %H:%M:%S%.3f").to_string();
    let style = match source {
        "err" => ui::YELLOW,
        "tw" => ui::CYAN,
        _ => anstyle::Style::new(),
    };
    format!(
        "{} {} {}",
        ui::paint(ui::DIM, time),
        ui::paint(style, format!("{source:<3}")),
        rest.unwrap_or_default()
    )
}

/// Asks for a new tunnel's fields, one at a time (`tunwarden add` on a terminal).
fn ask_fields(ctx: &Ctx) -> Result<serde_json::Map<String, Value>, Failure> {
    let mut m = serde_json::Map::new();
    let kinds = [
        ("command", "any command that opens a local port"),
        ("ssh-local", "SSH local forward (ssh -L)"),
        ("ssh-remote", "SSH remote forward (ssh -R)"),
        ("ssh-dynamic", "SOCKS proxy over SSH (ssh -D)"),
    ];
    let items: Vec<String> = kinds
        .iter()
        .map(|(k, d)| format!("{k:<12} {}", ui::paint(ui::DIM, d)))
        .collect();
    let kind = kinds[ui::select("Type", &items)?].0;
    if kind != "command" {
        m.insert("type".into(), Value::String(kind.into()));
    }
    let list = |text: &str| -> Vec<String> {
        text.split([',', ' '])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let forwards_prompt = match kind {
        "command" => "Local ports it opens (comma separated; empty: ready while it runs)",
        "ssh-local" => "Forwards as LOCAL:HOST:REMOTE_PORT (comma separated)",
        "ssh-remote" => "Forwards as LOCAL:REMOTE_PORT (comma separated)",
        _ => "Local SOCKS port",
    };
    match kind {
        "command" => {
            let command = ui::input("Command line", false)?;
            m.insert("command".into(), Value::String(command));
        }
        _ => {
            let host = ui::input("SSH host or alias from ~/.ssh/config", false)?;
            m.insert("host".into(), Value::String(host.trim().into()));
            let jump = list(&ui::input("Jump hosts (comma separated, optional)", true)?);
            if !jump.is_empty() {
                m.insert("jump".into(), Value::from(jump));
            }
        }
    }
    let specs = list(&ui::input(forwards_prompt, kind == "command")?);
    if !specs.is_empty() {
        let forwards: Result<Vec<Value>, Failure> = specs.iter().map(|f| forward(f)).collect();
        m.insert("forwards".into(), Value::Array(forwards?));
    }
    if let Ok(config) = config::load(&ctx.config)
        && !config.profiles.is_empty()
    {
        let mut items = vec![ui::paint(ui::DIM, "(no profile)")];
        items.extend(config.profiles.iter().map(|p| {
            let prod = if p.production {
                ui::paint(ui::YELLOW, " production")
            } else {
                String::new()
            };
            format!("{}{prod}", p.name)
        }));
        let i = ui::select("Profile", &items)?;
        if i > 0 {
            m.insert(
                "profile".into(),
                Value::String(config.profiles[i - 1].name.clone()),
            );
        }
    }
    let groups = list(&ui::input("Groups (comma separated, optional)", true)?);
    if !groups.is_empty() {
        m.insert("groups".into(), Value::from(groups));
    }
    Ok(m)
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
