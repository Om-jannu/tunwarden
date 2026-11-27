//! `tunwarden doctor` and the desktop app's Doctor view (PRD FR-36): checks
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
    pub fn add(
        &mut self,
        section: &str,
        mark: Mark,
        item: impl Into<String>,
        detail: impl Into<String>,
    ) {
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

    // Config
    let config = match crate::config::load(config_path) {
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
            r.add(
                "Config",
                Mark::Problem,
                config_path.display().to_string(),
                e.to_string(),
            );
            None
        }
    };

    // Daemon
    let mut client = match Client::connect(state_dir, client_name).await {
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
    if let Some(c) = client.as_mut()
        && let Ok(status) = c
            .call::<_, StatusResult>(method::STATUS, StatusParams { names: Vec::new() })
            .await
    {
        for e in &status.config_errors {
            r.add("Daemon", Mark::Problem, "config", e.clone());
        }
        for n in &status.notes {
            r.add("Daemon", Mark::Warning, "note", n.clone());
        }
        if let Ok(d) = c
            .call::<_, DoctorResult>(method::DOCTOR, serde_json::json!({}))
            .await
        {
            if let Some(e) = &d.write_error {
                r.add("Daemon", Mark::Problem, "state directory", e.clone());
            }
            daemon_doctor = Some(d);
        }
        daemon_status = Some(status);
    }
    match crate::autostart::status(&SystemEnv) {
        Ok(s) if !s.enabled => r.add(
            "Daemon",
            Mark::Ok,
            "autostart off",
            "run `tunwarden autostart enable` to start the daemon at login",
        ),
        Ok(s) => match (&s.problem, &s.exe, this_exe.map(Path::to_path_buf)) {
            (Some(problem), _, _) => r.add("Daemon", Mark::Problem, "autostart", problem.clone()),
            (None, Some(exe), Some(me)) if *exe != me => r.add(
                "Daemon",
                Mark::Warning,
                "autostart",
                format!(
                    "starts {}, not this tunwarden ({}); run `tunwarden autostart enable` to switch",
                    exe.display(),
                    me.display()
                ),
            ),
            _ => r.add("Daemon", Mark::Ok, "autostart on", s.location.clone()),
        },
        Err(e) => r.add("Daemon", Mark::Warning, "autostart", e.to_string()),
    }

    let Some(config) = config else {
        return r;
    };

    // Tools: the daemon's view decides, because it starts the tunnels.
    let mine = crate::tools::check_all(&config).await;
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
        let from_daemon = match client.as_mut() {
            Some(c) => c
                .call::<_, AuthResult>(method::AUTH_CHECK, AuthParams::default())
                .await
                .ok(),
            None => None,
        };
        let auth = match from_daemon {
            Some(a) => a,
            None => crate::auth::check_all(&config).await,
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
    let snapshot = tokio::task::spawn_blocking(crate::probe::Snapshot::take)
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));
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

    r
}
