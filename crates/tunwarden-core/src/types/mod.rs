//! Tunnel types (PRD 7.10, design section 4).
//!
//! Tunwarden supervises any command that opens a local port (`command`). The
//! only tool it knows itself is ssh, because ssh is on every OS and safe
//! defaults for it cannot be added to an arbitrary command line: the `ssh-*`
//! types build the ssh command from fields.
//!
//! A type knows four things (FR-41): how to build its command, how to check
//! readiness, which output means a problem retrying cannot fix, and which tool
//! it needs. Everything else (supervisor, logs, state, UI) only sees the
//! generic [`LaunchPlan`], [`Readiness`] and [`OutputPattern`] (FR-39).

mod command;
mod fields;
mod kind;
mod ssh;

use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use regex::Regex;

pub use command::{ALIVE_READY_AFTER, CommandTunnel};
pub use fields::{Fields, Report};
pub use kind::TunnelKind;
pub use ssh::{REMOTE_READY_AFTER, SshDynamic, SshLocal, SshRemote, SshTarget};

use crate::config::{FieldPath, Forward, Login, Profile, TunnelConfig};
use crate::failure::FailureReason;
use crate::paths::{Env, expand_tilde};

/// A tool Tunwarden builds commands for itself (FR-46).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Ssh,
}

impl Tool {
    /// The command name on PATH.
    pub const fn command(self) -> &'static str {
        match self {
            Tool::Ssh => "ssh",
        }
    }

    pub(crate) fn patterns(self) -> Vec<OutputPattern> {
        match self {
            Tool::Ssh => ssh::SSH_PATTERNS.clone(),
        }
    }
}

/// The program a tunnel runs. ssh is located through `[tools]`; any other
/// command is looked up on PATH and `tools.path` (design 6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    Tool(Tool),
    Command(String),
}

impl Program {
    pub fn name(&self) -> &str {
        match self {
            Program::Tool(tool) => tool.command(),
            Program::Command(command) => command,
        }
    }
}

/// Everything needed to spawn a tunnel. Always run as an argument list, never
/// through a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: Program,
    pub args: Vec<String>,
    /// Extra environment variables from the tunnel's profile (FR-7).
    pub env: Vec<(String, String)>,
}

impl LaunchPlan {
    /// The command as a POSIX shell line, for showing and copying (FR-40).
    /// Only for display: Tunwarden itself never runs it through a shell.
    pub fn display(&self) -> String {
        let mut words: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{k}={}", quote(v)))
            .collect();
        words.push(quote(self.program.name()));
        words.extend(self.args.iter().map(|a| quote(a)));
        words.join(" ")
    }
}

/// POSIX shell quoting: words made only of safe characters stay as they are;
/// anything else is wrapped in single quotes.
pub fn quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// How a tunnel counts as ready (FR-16, FR-44).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Every address must have a listening socket owned by the tunnel's processes.
    Listening(Vec<SocketAddr>),
    /// The process must stay up this long.
    AliveFor(Duration),
}

/// Output that means retrying cannot help (FR-24, FR-45).
#[derive(Clone, Debug)]
pub struct OutputPattern {
    pub regex: Regex,
    pub reason: FailureReason,
    /// The next step for the user. May contain `{tunnel}` and type fields
    /// such as `{host}`; see [`TunnelConfig::render_hint`].
    pub hint: String,
}

fn pattern(regex: &str, reason: FailureReason, hint: &str) -> OutputPattern {
    OutputPattern {
        // Built-in patterns are fixed in the source and covered by tests.
        regex: Regex::new(&format!("(?i){regex}")).expect("built-in pattern"),
        reason,
        hint: hint.to_owned(),
    }
}

/// A user pattern, matched ignoring case. Validated when the config loads.
pub fn user_pattern(regex: &str, reason: FailureReason, hint: String) -> Option<OutputPattern> {
    Regex::new(&format!("(?i){regex}"))
        .ok()
        .map(|regex| OutputPattern {
            regex,
            reason,
            hint,
        })
}

/// The patterns of a login: its `expired` texts mean "log in again".
pub fn login_patterns(login: &Login) -> Vec<OutputPattern> {
    let hint = format!(
        "the {} login has expired; run `tunwarden login {}`",
        login.name, login.name
    );
    login
        .expired
        .iter()
        .filter_map(|p| user_pattern(p, FailureReason::AuthExpired, hint.clone()))
        .collect()
}

/// The first pattern that matches `line`.
pub fn classify<'a>(patterns: &'a [OutputPattern], line: &str) -> Option<&'a OutputPattern> {
    patterns.iter().find(|p| p.regex.is_match(line))
}

/// What every type provides.
pub trait TypeDef {
    fn program(&self) -> Program;
    fn args(&self, forwards: &[Forward]) -> Vec<String>;
    fn readiness(&self, forwards: &[Forward]) -> Readiness;
    fn output_patterns(&self) -> Vec<OutputPattern>;
    /// Values for the `{...}` placeholders in hints.
    fn hint_vars(&self) -> Vec<(&'static str, String)>;
}

/// The validated settings of one tunnel's type.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeSettings {
    SshLocal(SshLocal),
    SshRemote(SshRemote),
    SshDynamic(SshDynamic),
    Command(CommandTunnel),
}

impl TypeSettings {
    pub fn kind(&self) -> TunnelKind {
        match self {
            TypeSettings::SshLocal(_) => TunnelKind::SshLocal,
            TypeSettings::SshRemote(_) => TunnelKind::SshRemote,
            TypeSettings::SshDynamic(_) => TunnelKind::SshDynamic,
            TypeSettings::Command(_) => TunnelKind::Command,
        }
    }

    pub fn def(&self) -> &dyn TypeDef {
        match self {
            TypeSettings::SshLocal(t) => t,
            TypeSettings::SshRemote(t) => t,
            TypeSettings::SshDynamic(t) => t,
            TypeSettings::Command(t) => t,
        }
    }

    /// The ssh connection, for the ssh types (used by `trust`).
    pub fn ssh_target(&self) -> Option<&SshTarget> {
        match self {
            TypeSettings::SshLocal(s) => Some(&s.target),
            TypeSettings::SshRemote(s) => Some(&s.target),
            TypeSettings::SshDynamic(s) => Some(&s.target),
            TypeSettings::Command(_) => None,
        }
    }
}

/// Validates a type's own fields. Returns `None` if any error was reported.
pub fn parse_settings(
    kind: TunnelKind,
    table: &toml::Table,
    forwards: &[Forward],
    path: &FieldPath,
    report: &mut dyn Report,
    env: &dyn Env,
    common_keys: &[&str],
) -> Option<TypeSettings> {
    let mut f = Fields::new(table, path, report);
    let settings = match kind {
        TunnelKind::SshLocal => SshLocal::parse(&mut f, forwards).map(TypeSettings::SshLocal),
        TunnelKind::SshRemote => SshRemote::parse(&mut f, forwards).map(TypeSettings::SshRemote),
        TunnelKind::SshDynamic => SshDynamic::parse(&mut f, forwards).map(TypeSettings::SshDynamic),
        TunnelKind::Command => {
            CommandTunnel::parse(&mut f, forwards, env).map(TypeSettings::Command)
        }
    };
    let failed = f.finish(kind.id(), common_keys);
    if failed { None } else { settings }
}

/// How a command line in the config is split into words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// POSIX shell rules: quotes group words, `\` escapes the next character.
    Posix,
    /// The same, except that `\` is an ordinary character, so Windows paths
    /// such as `C:\tools\gcloud.cmd` need no quoting. `'...'` and `"..."` still
    /// group, with nothing escaped inside them.
    Windows,
}

impl Syntax {
    /// The syntax of the OS this binary was built for.
    pub const fn current() -> Syntax {
        if cfg!(windows) {
            Syntax::Windows
        } else {
            Syntax::Posix
        }
    }
}

/// Splits `command` into words; `None` for an unclosed quote.
pub fn split_words(command: &str, syntax: Syntax) -> Option<Vec<String>> {
    match syntax {
        Syntax::Posix => shlex::split(command),
        Syntax::Windows => split_windows(command),
    }
}

fn split_windows(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut open: Option<char> = None;
    for c in command.chars() {
        match open {
            Some(q) if c == q => open = None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                open = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if open.is_some() {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// Quotes `word` so that [`split_words`] with the same syntax gives it back.
/// For text written into the config.
pub fn quote_with(word: &str, syntax: Syntax) -> String {
    match syntax {
        Syntax::Posix => quote(word),
        Syntax::Windows => {
            let safe = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-\\".contains(c));
            if safe {
                word.to_owned()
            } else if !word.contains('\'') {
                format!("'{word}'")
            } else if !word.contains('"') {
                format!("\"{word}\"")
            } else {
                // Both quote characters: single-quoted runs joined by "'".
                word.split('\'')
                    .map(|part| format!("'{part}'"))
                    .collect::<Vec<_>>()
                    .join("\"'\"")
            }
        }
    }
}

/// The listen addresses of a tunnel's forwards.
fn listen_addrs(forwards: &[Forward]) -> Vec<SocketAddr> {
    forwards
        .iter()
        .map(|f| SocketAddr::new(f.bind, f.local_port))
        .collect()
}

/// Splits a command line without a shell (FR-8): shell-style quoting and word
/// splitting in this OS's [`Syntax`], `~` expanded at the start of a word, no
/// other shell features.
pub fn split_command_line(command: &str, env: &dyn Env) -> Result<Vec<String>, String> {
    let syntax = Syntax::current();
    if let Some((c, what)) = command::shell_feature(command, syntax) {
        return Err(format!(
            "uses {what} (`{c}`), which needs a shell, but Tunwarden runs commands without one; put the command in a script and run the script instead"
        ));
    }
    let words = split_words(command, syntax).ok_or_else(|| "has an unclosed quote".to_owned())?;
    if words.is_empty() {
        return Err("is empty".to_owned());
    }
    Ok(words
        .into_iter()
        .map(|w| {
            if w == "~" || w.starts_with("~/") {
                expand_tilde(Path::new(&w), &command::EnvRef(env))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(w)
            } else {
                w
            }
        })
        .collect())
}

impl TunnelConfig {
    /// The command to spawn for this tunnel (FR-7). Its environment is the
    /// login's, so the tunnel runs as the account the check verifies, with
    /// the profile's values added on top; the profile wins on a clash.
    pub fn launch_plan(&self, profile: Option<&Profile>, login: Option<&Login>) -> LaunchPlan {
        let def = self.type_settings.def();
        let mut env = std::collections::BTreeMap::new();
        for (k, v) in login.into_iter().flat_map(|l| &l.env) {
            env.insert(k.clone(), v.clone());
        }
        for (k, v) in profile.into_iter().flat_map(|p| &p.env) {
            env.insert(k.clone(), v.clone());
        }
        LaunchPlan {
            program: def.program(),
            args: def.args(&self.forwards),
            env: env.into_iter().collect(),
        }
    }

    pub fn readiness(&self) -> Readiness {
        self.type_settings.def().readiness(&self.forwards)
    }

    /// The type's patterns, then the "login expired" patterns of the login
    /// its profile uses (FR-24, FR-45).
    pub fn output_patterns(&self, login: Option<&Login>) -> Vec<OutputPattern> {
        let mut patterns = self.type_settings.def().output_patterns();
        if let Some(login) = login {
            patterns.extend(login_patterns(login));
        }
        patterns
    }

    /// Fills the `{...}` placeholders of a hint for this tunnel.
    pub fn render_hint(&self, hint: &str) -> String {
        let mut text = hint.replace("{tunnel}", &self.name);
        for (key, value) in self.type_settings.def().hint_vars() {
            text = text.replace(&format!("{{{key}}}"), &value);
        }
        text
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.command())
    }
}

#[cfg(test)]
mod tests;
