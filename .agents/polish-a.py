"""CLI polish, part A: colours, error display, optional names with pickers,
dynamic completions, man pages, help examples."""

P = "crates/tunwarden-cli/src/lib.rs"
s = open(P, encoding="utf-8").read()


def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (s.count(old), old[:90])
    s = s.replace(old, new)


# --- Help examples, optional names, completions, man ------------------------
rep('''    /// Start tunnels
    Start(Action),
    /// Stop tunnels
    Stop(Action),
    /// Restart tunnels
    Restart(Action),
    /// Show a tunnel's log
    Logs {
        name: String,''', '''    /// Start tunnels
    #[command(after_help = EXAMPLES_START)]
    Start(Action),
    /// Stop tunnels
    #[command(after_help = "Examples:\\n  tunwarden stop staging-db\\n  tunwarden stop --all\\n  tunwarden stop          pick from the running tunnels")]
    Stop(Action),
    /// Restart tunnels
    Restart(Action),
    /// Show a tunnel's log
    #[command(after_help = "Examples:\\n  tunwarden logs staging-db -f\\n  tunwarden logs staging-db -n 200 | grep -i error\\n  tunwarden logs          pick a tunnel")]
    Logs {
        /// The tunnel; asks when missing
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: Option<String>,''')
rep('''    /// Print the command a tunnel runs
    ShowCommand { name: String },
    /// Add a tunnel to the config
    Add {''', '''    /// Print the command a tunnel runs
    ShowCommand {
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: String,
    },
    /// Add a tunnel to the config
    #[command(after_help = EXAMPLES_ADD)]
    Add {''')
rep('''    /// Remove a tunnel from the config (stops it if it runs)
    Remove { name: String },
    /// Turn pasted tunnel commands or aliases into config entries
    Import {''', '''    /// Remove a tunnel from the config (stops it if it runs)
    Remove {
        #[arg(add = ArgValueCandidates::new(complete_tunnels))]
        name: String,
    },
    /// Turn pasted tunnel commands or aliases into config entries
    #[command(after_help = "Examples:\\n  tunwarden import ~/.zshrc            preview the entries\\n  tunwarden import ~/.zshrc --write    pick which ones to add\\n  pbpaste | tunwarden import")]
    Import {''')
rep('''    /// Run a login's `login` command, then restart the tunnels waiting for it
    Login {
        /// A login, or a profile meaning its login
        name: String,
    },
    /// Print a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },''', '''    /// Run a login's `login` command, then restart the tunnels waiting for it
    #[command(after_help = "Examples:\\n  tunwarden login google\\n  tunwarden login staging   a profile means its login\\n  tunwarden login           pick a login")]
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
    Man { dir: PathBuf },''')
rep('''struct Action {
    /// Tunnel names
    names: Vec<String>,''', '''struct Action {
    /// Tunnel names; asks when none are given and no --all, --group or --profile
    #[arg(add = ArgValueCandidates::new(complete_tunnels))]
    names: Vec<String>,''')
rep('''    /// The tunnels in a group
    #[arg(long)]
    group: Option<String>,
    /// The tunnels of a profile
    #[arg(long)]
    profile: Option<String>,''', '''    /// The tunnels in a group
    #[arg(long, add = ArgValueCandidates::new(complete_groups))]
    group: Option<String>,
    /// The tunnels of a profile
    #[arg(long, add = ArgValueCandidates::new(complete_profiles))]
    profile: Option<String>,''')
rep('''    /// Profile: shared environment, production flag and login
    #[arg(long)]
    profile: Option<String>,''', '''    /// Profile: shared environment, production flag and login
    #[arg(long, add = ArgValueCandidates::new(complete_profiles))]
    profile: Option<String>,''')
rep('''    Auth {
        /// A login, or a profile meaning its login; all logins if missing
        name: Option<String>,''', '''    Auth {
        /// A login, or a profile meaning its login; all logins if missing
        #[arg(add = ArgValueCandidates::new(complete_logins))]
        name: Option<String>,''')

rep('''/// Exit code when the daemon is not running (`daemon status`).
const EXIT_NOT_RUNNING: u8 = 3;''', '''/// Exit code when the daemon is not running (`daemon status`).
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
  tunwarden add grafana --command \\"kubectl port-forward svc/grafana 3000:80\\" --forward 3000
  tunwarden add redis --type ssh-local --set host=bastion --forward 6380:redis.internal:6379";
const EXAMPLES_COMPLETIONS: &str = "Setup:
  bash: echo 'source <(tunwarden completions bash)' >> ~/.bashrc
  zsh:  echo 'source <(tunwarden completions zsh)' >> ~/.zshrc
  fish: tunwarden completions fish > ~/.config/fish/completions/tunwarden.fish
  PowerShell: tunwarden completions powershell | Out-String | Invoke-Expression";''')

# --- Imports -----------------------------------------------------------------
rep('''use clap::{Args, CommandFactory, Parser, Subcommand};''',
    '''use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};''')

# --- Cancelled prompts become exit 130 ----------------------------------------
rep('''impl From<String> for Failure {''', '''impl From<ui::Cancelled> for Failure {
    fn from(_: ui::Cancelled) -> Failure {
        Failure {
            message: "cancelled".into(),
            hint: None,
        }
    }
}

impl From<String> for Failure {''')

# --- run: completions engine, coloured errors --------------------------------
rep('''    let cli = Cli::parse_from(args);''', '''    // `COMPLETE=<shell> tunwarden ...` is the shell asking for completions.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse_from(args);''')
rep('''fn report(e: Failure) -> ExitCode {
    eprintln!("error: {}", e.message);
    if let Some(hint) = e.hint {
        eprintln!("hint: {hint}");
    }
    ExitCode::FAILURE
}''', '''fn report(e: Failure) -> ExitCode {
    if e.message == "cancelled" && e.hint.is_none() {
        return ExitCode::from(EXIT_CANCELLED);
    }
    eprintln!("{} {}", ui::paint(ui::RED.bold(), "error:"), e.message);
    if let Some(hint) = e.hint {
        eprintln!("{} {hint}", ui::paint(ui::CYAN, "hint:"));
    }
    ExitCode::FAILURE
}''')

# --- dispatch ----------------------------------------------------------------
rep('''        Cmd::Login { name } => login(ctx, &name).await,''',
    '''        Cmd::Login { name } => {
            let name = match name {
                Some(n) => n,
                None => pick_login(ctx)?,
            };
            login(ctx, &name).await
        }''')
rep('''        } => logs(ctx, &name, lines, follow).await,''', '''        } => {
            let name = match name {
                Some(n) => n,
                None => pick_tunnel(ctx, "Show the log of")?,
            };
            logs(ctx, &name, lines, follow).await
        }''')
rep('''        Cmd::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "tunwarden",
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }''', '''        Cmd::Completions { shell } => {
            let completer = clap_complete::env::Shells::builtins()
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
        }''')

# --- Config errors with miette on a terminal ---------------------------------
rep('''        LoadError::Invalid(errors) => Failure {
            message: format!(
                "the config file has {} problem(s):\\n{errors}",
                errors.0.len()
            ),
            hint: None,
        },
    })
}''', '''        LoadError::Invalid(errors) => Failure {
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
        Some(d) => format!("the config file has {count} problem(s):\\n\\n{}", d.trim_end()),
        None => format!("the config file has {count} problem(s):\\n{errors}"),
    }
}''')
rep('''        LoadError::Invalid(errors) => Failure {
            message: format!(
                "the config file has {} problem(s):\\n{errors}",
                errors.0.len()
            ),
            hint: Some(format!("fix {} first", ctx.config.display())),
        },''', '''        LoadError::Invalid(errors) => Failure {
            message: config_errors_message(&ctx.config, &errors),
            hint: Some(format!("fix {} first", ctx.config.display())),
        },''')

# --- Colours in tables and reports -------------------------------------------
rep('''        table.row(vec![
            t.name.clone(),
            t.state.as_str().to_owned(),
            or_dash(t.local.join(", ")),
            t.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),''', '''        table.row(vec![
            t.name.clone(),
            ui::state(t.state),
            or_dash(t.local.join(", ")),
            t.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),''')
rep('''    for e in &result.config_errors {
        eprintln!("config: {e}");
    }
    for n in &result.notes {
        eprintln!("note: {n}");
    }''', '''    for e in &result.config_errors {
        eprintln!("{} {e}", ui::paint(ui::RED, "config:"));
    }
    for n in &result.notes {
        eprintln!("{} {n}", ui::paint(ui::DIM, "note:"));
    }''')
rep('''            let label = match mark {
                Mark::Ok => "ok",
                Mark::Warning => "warning",
                Mark::Problem => {
                    problems = true;
                    "problem"
                }
            };
            let line = format!("  {label:<8} {item:<24} {detail}");
            println!("{}", line.trim_end());''', '''            let (label, style) = match mark {
                Mark::Ok => ("ok", ui::GREEN),
                Mark::Warning => ("warning", ui::YELLOW),
                Mark::Problem => {
                    problems = true;
                    ("problem", ui::RED)
                }
            };
            let line = format!("  {} {item:<24} {detail}", ui::paint(style, format!("{label:<8}")));
            println!("{}", line.trim_end());''')
rep('''                println!("{s}");
                section = s;''', '''                println!("{}", ui::paint(ui::BOLD, s));
                section = s;''')
rep('''            table.row(vec![
                l.name.clone(),
                or_dash(l.profiles.join(", ")),
                l.result.clone(),
                detail,
            ]);''', '''            let result = match l.result.as_str() {
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
            ]);''')

# --- Production confirmation through the prompt library ----------------------
rep('''    eprint!("{list} belong to a production profile. {verb} anyway? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(format!("{verb} cancelled").into())
    }''', '''    let question = format!(
        "{} belong to a production profile. {verb} anyway?",
        ui::paint(ui::YELLOW.bold(), &list)
    );
    if ui::confirm(&question)? {
        Ok(())
    } else {
        Err(format!("{verb} cancelled").into())
    }''')

# --- Log lines: local time and coloured source on a terminal ------------------
rep('''            for line in tunwarden_core::logs::tail(&path, lines)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            {
                println!("{line}");
            }''', '''            let pretty = std::io::stdout().is_terminal();
            for line in tunwarden_core::logs::tail(&path, lines)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            {
                println!("{}", log_line(&line, pretty));
            }''')
rep('''    for line in &past.lines {
        println!("{line}");
    }
    if !follow {''', '''    let pretty = std::io::stdout().is_terminal();
    for line in &past.lines {
        println!("{}", log_line(line, pretty));
    }
    if !follow {''')
rep('''        println!("{}", l.line);
        let _ = std::io::stdout().flush();''', '''        println!("{}", log_line(&l.line, pretty));
        let _ = std::io::stdout().flush();''')

s += '''
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
            CompletionCandidate::new(&t.name)
                .help(Some(format!("{} {}", info.kind, info.local.join(", ")).into()))
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
            format!("{:<20} {}", t.name, ui::paint(ui::DIM, info.local.join(", ")))
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
                ui::paint(ui::DIM, format!("used by {}", or_dash(view.profiles.join(", "))))
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
    let local = t.to_zoned(jiff::tz::TimeZone::system());
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
'''

open(P, "w", encoding="utf-8").write(s)
print("ok")
