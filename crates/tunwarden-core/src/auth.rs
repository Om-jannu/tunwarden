//! Logins the user renews now and then, for any tool (PRD 7.5, design 9).
//!
//! A `[[logins]]` entry has an optional `check` command that succeeds while
//! the login is valid, an optional interactive `login` command, `expired`
//! patterns for output that means "log in again", and its own environment.
//! Tunwarden knows nothing about any particular tool; the example config has
//! ready-made entries for gcloud, AWS SSO and others.
//!
//! The check's standard output is discarded unread, so a tool that prints a
//! token there never leaks it (FR-26). Only its error output is matched.

use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::{Login, ToolPaths};
use crate::paths::{Os, SystemEnv};
use crate::tools;
use crate::types::Program;

/// How long one check may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// The result of one check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    /// The user has to log in again.
    Expired {
        detail: String,
    },
    /// The check itself failed, for example no network. Never stops tunnels.
    Unknown {
        detail: String,
    },
}

impl Outcome {
    pub fn name(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Expired { .. } => "expired",
            Outcome::Unknown { .. } => "unknown",
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            Outcome::Ok => None,
            Outcome::Expired { detail } | Outcome::Unknown { detail } => Some(detail.clone()),
        }
    }
}

/// Runs the login's `check` command (FR-22). A login without one is `Ok`.
pub async fn check(login: &Login, tool_paths: &ToolPaths) -> Outcome {
    let Some(words) = &login.check else {
        return Outcome::Ok;
    };
    let program = match tools::resolve(
        &Program::Command(words[0].clone()),
        tool_paths,
        &SystemEnv,
        Os::current(),
    ) {
        Ok(r) => r.path,
        Err(e) => {
            return Outcome::Unknown {
                detail: e.to_string(),
            };
        }
    };
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&words[1..])
        .envs(login.env.iter().map(|(k, v)| (k, v)))
        // Null stdin: some tools wait for input when a terminal is attached.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(CHECK_TIMEOUT, async {
        cmd.spawn()?.wait_with_output().await
    })
    .await
    {
        Err(_) => {
            return Outcome::Unknown {
                detail: format!(
                    "`{}` did not finish within {} seconds",
                    words[0],
                    CHECK_TIMEOUT.as_secs()
                ),
            };
        }
        Ok(Err(e)) => {
            return Outcome::Unknown {
                detail: format!("cannot run {}: {e}", program.display()),
            };
        }
        Ok(Ok(output)) => output,
    };
    classify(login, output.status.success(), &output.stderr)
}

/// Exit 0 is `ok`. A failure whose error output matches an `expired`
/// pattern is `expired`; any other failure is `unknown`. Without `expired`
/// patterns, every failure counts as `expired`.
pub fn classify(login: &Login, success: bool, stderr: &[u8]) -> Outcome {
    if success {
        return Outcome::Ok;
    }
    let text = String::from_utf8_lossy(stderr);
    let last = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_owned());
    if login.expired.is_empty() {
        return Outcome::Expired {
            detail: last.unwrap_or_else(|| "the check command failed".into()),
        };
    }
    let patterns: Vec<Regex> = login
        .expired
        .iter()
        .filter_map(|p| Regex::new(&format!("(?i){p}")).ok())
        .collect();
    if let Some(line) = text
        .lines()
        .find(|l| patterns.iter().any(|p| p.is_match(l)))
    {
        return Outcome::Expired {
            detail: line.trim().to_owned(),
        };
    }
    Outcome::Unknown {
        detail: last.unwrap_or_else(|| "the check command failed without a message".into()),
    }
}

/// The interactive login, run by the client in the user's terminal (FR-25).
pub fn login_command(login: &Login, program: &std::path::Path) -> Option<std::process::Command> {
    let words = login.login.as_ref()?;
    let mut cmd = std::process::Command::new(program);
    cmd.args(&words[1..])
        .envs(login.env.iter().map(|(k, v)| (k, v)));
    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(expired: &[&str]) -> Login {
        Login {
            name: "work".into(),
            check: Some(vec!["tool".into(), "status".into()]),
            login: Some(vec!["tool".into(), "login".into()]),
            expired: expired.iter().map(|s| s.to_string()).collect(),
            env: vec![("TOOL_ACCOUNT".into(), "me@example.com".into())],
        }
    }

    #[test]
    fn classification() {
        let l = login(&["Reauthentication failed", "session (has )?expired"]);
        assert_eq!(classify(&l, true, b""), Outcome::Ok);
        assert_eq!(
            classify(
                &l,
                false,
                b"ERROR: (gcloud.x) Reauthentication failed. cannot prompt\n"
            ),
            Outcome::Expired {
                detail: "ERROR: (gcloud.x) Reauthentication failed. cannot prompt".into()
            }
        );
        assert_eq!(
            classify(&l, false, b"Error: Session Has Expired\n").name(),
            "expired",
            "ignores case"
        );
        assert_eq!(
            classify(&l, false, b"connection timed out\n"),
            Outcome::Unknown {
                detail: "connection timed out".into()
            },
            "other failures never stop tunnels"
        );
        let strict = login(&[]);
        assert_eq!(
            classify(&strict, false, b"nope\n").name(),
            "expired",
            "no patterns: any failure"
        );
    }

    #[test]
    fn login_command_uses_the_login_env() {
        let cmd = login_command(&login(&[]), std::path::Path::new("/usr/bin/tool")).unwrap();
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["login"]);
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&(
            std::ffi::OsStr::new("TOOL_ACCOUNT"),
            Some(std::ffi::OsStr::new("me@example.com"))
        )));
        let mut no_login = login(&[]);
        no_login.login = None;
        assert!(login_command(&no_login, std::path::Path::new("/x")).is_none());
    }
}
