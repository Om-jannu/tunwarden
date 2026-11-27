"""Moves `doctor` and the login helpers from the CLI into tunwarden-core."""
import re

CLI = "crates/tunwarden-cli/src/lib.rs"
s = open(CLI, encoding="utf-8").read()

# --- 1. The doctor body ----------------------------------------------------------
start = s.index("async fn doctor(ctx: &Ctx) -> Outcome {")
end = s.index("    Ok(if r.print() {", start)
body = s[s.index("\n", start) + 1 : end]
body = body.replace("    let mut r = Report { rows: Vec::new() };\n", "")

# Config: a plain message instead of the CLI's Failure.
old_cfg = body[body.index("    let config = match load(ctx) {") : body.index("    // Daemon")]
new_cfg = '''    let config = match crate::config::load(config_path) {
        Ok(c) => {
            r.add(
                "Config",
                Mark::Ok,
                config_path.display().to_string(),
                format!(
                    "{} tunnels, {} profiles, {} logins",
                    c.tunnels.len(),
                    c.profiles.len(),
                    c.logins.len()
                ),
            );
            Some(c)
        }
        Err(LoadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            r.add(
                "Config",
                Mark::Problem,
                config_path.display().to_string(),
                "no config file; run `tunwarden init` to create one",
            );
            None
        }
        Err(e) => {
            r.add("Config", Mark::Problem, config_path.display().to_string(), e.to_string());
            None
        }
    };

'''
body = body.replace(old_cfg, new_cfg)
body = body.replace('Client::connect(&ctx.state_dir, "cli")', "Client::connect(state_dir, client_name)")
body = body.replace(
    '''    let Some(config) = config else {
        r.print();
        return Ok(ExitCode::FAILURE);
    };''',
    '''    let Some(config) = config else {
        return r;
    };''',
)
# Daemon calls: a failure becomes a report line, not an early return.
body = body.replace(
    '''    if let Some(c) = client.as_mut() {
        let status: StatusResult = c
            .call(method::STATUS, StatusParams { names: Vec::new() })
            .await?;''',
    '''    if let Some(c) = client.as_mut()
        && let Ok(status) = c
            .call::<_, StatusResult>(method::STATUS, StatusParams { names: Vec::new() })
            .await
    {''',
)
body = body.replace(
    '''        let d: DoctorResult = c.call(method::DOCTOR, serde_json::json!({})).await?;
        if let Some(e) = &d.write_error {
            r.add("Daemon", Mark::Problem, "state directory", e.clone());
        }
        daemon_status = Some(status);
        daemon_doctor = Some(d);''',
    '''        if let Ok(d) = c
            .call::<_, DoctorResult>(method::DOCTOR, serde_json::json!({}))
            .await
        {
            if let Some(e) = &d.write_error {
                r.add("Daemon", Mark::Problem, "state directory", e.clone());
            }
            daemon_doctor = Some(d);
        }
        daemon_status = Some(status);''',
)
body = body.replace(
    "tunwarden_core::autostart::status(&SystemEnv)", "crate::autostart::status(&SystemEnv)"
)
body = body.replace(
    "Ok(s) => match (&s.problem, &s.exe, std::env::current_exe().ok()) {",
    "Ok(s) => match (&s.problem, &s.exe, this_exe.map(Path::to_path_buf)) {",
)
body = body.replace("tunwarden_core::tools::check_all", "crate::tools::check_all")
body = body.replace("tunwarden_core::probe::Snapshot::take", "crate::probe::Snapshot::take")
body = body.replace(
    '''        let auth: AuthResult = match client.as_mut() {
            Some(c) => c.call(method::AUTH_CHECK, AuthParams::default()).await?,
            None => check_logins(&config, None).await?,
        };''',
    '''        let from_daemon = match client.as_mut() {
            Some(c) => c
                .call::<_, AuthResult>(method::AUTH_CHECK, AuthParams::default())
                .await
                .ok(),
            None => None,
        };
        let auth = match from_daemon {
            Some(a) => a,
            None => crate::auth::check_all(&config).await,
        };''',
)
body = body.replace(
    '''    let snapshot = tokio::task::spawn_blocking(crate::probe::Snapshot::take)
        .await
        .map_err(|e| e.to_string())?;''',
    '''    let snapshot = tokio::task::spawn_blocking(crate::probe::Snapshot::take)
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string()).into()));''',
)

core = '''//! `tunwarden doctor` and the desktop app's Doctor view (PRD FR-36): checks
//! the config, the daemon, autostart, tools, logins and ports, and returns
//! one line per finding.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::client::{Client, ClientError};
use crate::config::LoadError;
use crate::paths::SystemEnv;
use crate::proto::*;

/// How a finding reads: fine, worth a look, or something to fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Mark {
    Ok,
    Warning,
    Problem,
}

/// One line: its section (Config, Daemon, Tools, Logins, Ports), mark, the
/// thing checked and what was found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Row {
    pub section: String,
    pub mark: Mark,
    pub item: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Report {
    pub rows: Vec<Row>,
}

impl Report {
    pub fn add(&mut self, section: &str, mark: Mark, item: impl Into<String>, detail: impl Into<String>) {
        self.rows.push(Row {
            section: section.to_owned(),
            mark,
            item: item.into(),
            detail: detail.into(),
        });
    }

    pub fn has_problems(&self) -> bool {
        self.rows.iter().any(|r| r.mark == Mark::Problem)
    }
}

/// Runs every check. `this_exe` is the tunwarden binary asking, so an
/// autostart entry that starts another one can be pointed out.
pub async fn run(
    config_path: &Path,
    state_dir: &Path,
    client_name: &str,
    this_exe: Option<&Path>,
) -> Report {
    let mut r = Report::default();
''' + body + '''    r
}

#[allow(dead_code)]
fn _uses(_: ClientError) {}
'''
open("crates/tunwarden-core/src/doctor.rs", "w", encoding="utf-8").write(core)

# --- 2. CLI: print the core report -----------------------------------------------
# Drop the CLI's Mark/Report types, keep a printer.
mstart = s.index("/// One line of the doctor report.")
mend = s.index("async fn doctor(ctx: &Ctx) -> Outcome {")
s = s[:mstart] + '''use tunwarden_core::doctor::Mark;

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

''' + s[mend:]
dstart = s.index("async fn doctor(ctx: &Ctx) -> Outcome {")
dend_marker = s.index("    Ok(if r.print() {", dstart)
dend = s.index("\n}\n", dend_marker) + 3
s = s[:dstart] + '''async fn doctor(ctx: &Ctx) -> Outcome {
    let exe = std::env::current_exe().ok();
    let report =
        tunwarden_core::doctor::run(&ctx.config, &ctx.state_dir, "cli", exe.as_deref()).await;
    Ok(if print_report(&report) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
''' + s[dend:]

# --- 3. Login helpers to core ----------------------------------------------------
fstart = s.index("fn find_login<'c>(config: &'c Config, name: &str) -> Result<&'c Login, Failure> {")
fend = s.index("/// Checks logins through the daemon if it runs")
s = s[:fstart] + '''fn find_login<'c>(config: &'c Config, name: &str) -> Result<&'c Login, Failure> {
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
            let outcome = l.check.is_some().then_some(gauth::check(l, &config.tools).await);
            Ok(AuthResult {
                logins: vec![login_view(config, l, outcome.as_ref())],
            })
        }
        None => Ok(gauth::check_all(config).await),
    }
}

''' + s[fend:]
open(CLI, "w", encoding="utf-8").write(s)

A = "crates/tunwarden-core/src/auth.rs"
a = open(A, encoding="utf-8").read()
marker = "#[cfg(test)]\nmod tests {"
a = a.replace(marker, '''/// A login that cannot be found, with what to do instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupError {
    pub message: String,
    pub hint: Option<String>,
}

/// The login named `name`, or the login of the profile named `name`.
pub fn find<'c>(config: &'c crate::config::Config, name: &str) -> Result<&'c Login, LookupError> {
    if let Some(l) = config.login(name) {
        return Ok(l);
    }
    if let Some(p) = config.profile(name) {
        return match p.login.as_deref().and_then(|l| config.login(l)) {
            Some(l) => Ok(l),
            None => Err(LookupError {
                message: format!("profile \\"{name}\\" has no login"),
                hint: Some("set `login = \\"NAME\\"` in the profile to use one".into()),
            }),
        };
    }
    let known: Vec<&str> = config.logins.iter().map(|l| l.name.as_str()).collect();
    Err(LookupError {
        message: format!("no login or profile named \\"{name}\\""),
        hint: Some(if known.is_empty() {
            "no logins are configured; add a [[logins]] entry".into()
        } else {
            format!("configured logins: {}", known.join(", "))
        }),
    })
}

/// A login as `auth` shows it, with the profiles that use it.
pub fn view(
    config: &crate::config::Config,
    login: &Login,
    outcome: Option<&Outcome>,
) -> crate::proto::LoginView {
    crate::proto::LoginView {
        name: login.name.clone(),
        profiles: config
            .profiles
            .iter()
            .filter(|p| p.login.as_ref() == Some(&login.name))
            .map(|p| p.name.clone())
            .collect(),
        checkable: login.check.is_some(),
        result: outcome.map_or("not checked", Outcome::name).to_owned(),
        detail: outcome.and_then(Outcome::detail),
        checked_at: outcome.map(|_| crate::fsutil::format_timestamp_now()),
        active_tunnels: Vec::new(),
    }
}

/// Checks every login with a `check` command, without a daemon.
pub async fn check_all(config: &crate::config::Config) -> crate::proto::AuthResult {
    let mut logins = Vec::new();
    for l in &config.logins {
        let outcome = match l.check {
            Some(_) => Some(check(l, &config.tools).await),
            None => None,
        };
        logins.push(view(config, l, outcome.as_ref()));
    }
    crate::proto::AuthResult { logins }
}

''' + marker, 1)
open(A, "w", encoding="utf-8").write(a)

L = "crates/tunwarden-core/src/lib.rs"
l = open(L).read()
l = l.replace("pub mod daemon;\n", "pub mod daemon;\npub mod doctor;\n", 1)
open(L, "w").write(l)
print("ok")
