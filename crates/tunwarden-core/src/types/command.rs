//! `command`: any command that opens a local port (PRD 7.10, FR-8).

use std::time::Duration;

use super::fields::{Fields, forbid_forward_fields};
use super::{
    OutputPattern, Program, Readiness, Syntax, Tool, TypeDef, listen_addrs, split_command_line,
    user_pattern,
};
use crate::config::Forward;
use crate::failure::FailureReason;
use crate::paths::Env;

/// How long a "process alive" tunnel must stay up to count as ready.
pub const ALIVE_READY_AFTER: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub struct CommandTunnel {
    /// The command line split into words, with `~` expanded.
    pub words: Vec<String>,
    /// Readiness by "process alive" instead of listening ports.
    pub process_alive: bool,
    /// User regexes that mean a credential problem (FR-47).
    pub credential_patterns: Vec<String>,
}

impl CommandTunnel {
    pub(super) fn parse(f: &mut Fields<'_>, forwards: &[Forward], env: &dyn Env) -> Option<Self> {
        let kind = "command";
        let command = f.required_string("command", kind, "\"my-tool connect --port 5433\"");
        let readiness = f.string("readiness");
        let credential_patterns = f.string_list("credential_patterns");
        forbid_forward_fields(f, kind, forwards, &[]);

        let process_alive = match readiness.as_deref() {
            None => forwards.is_empty(),
            Some("process") => true,
            Some("port") if forwards.is_empty() => {
                f.error(
                    "readiness",
                    "`readiness = \"port\"` needs at least one forward with a local_port"
                        .to_owned(),
                );
                false
            }
            Some("port") => false,
            Some(other) => {
                f.error(
                    "readiness",
                    format!("unknown readiness \"{other}\"; use \"port\" or \"process\""),
                );
                false
            }
        };

        for (i, p) in credential_patterns.iter().enumerate() {
            if let Err(e) = regex::Regex::new(p) {
                let path = f.path("credential_patterns").index(i);
                f.error_at(
                    &path,
                    format!("invalid regular expression: {}", first_line(&e.to_string())),
                );
            }
        }

        let words = match split_command_line(&command?, env) {
            Ok(words) => words,
            Err(problem) => {
                f.error("command", problem);
                return None;
            }
        };
        Some(CommandTunnel {
            words,
            process_alive,
            credential_patterns,
        })
    }
}

fn first_line(s: &str) -> &str {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim()
}

/// `expand_tilde` takes `&impl Env`; this adapts a `&dyn Env`.
pub(super) struct EnvRef<'a>(pub(super) &'a dyn Env);

impl Env for EnvRef<'_> {
    fn var(&self, key: &str) -> Option<std::ffi::OsString> {
        self.0.var(key)
    }

    fn home(&self) -> Option<std::path::PathBuf> {
        self.0.home()
    }
}

/// The first character outside single quotes that a shell would treat
/// specially, with a description. Inside double quotes only `$` and `` ` `` count.
pub(super) fn shell_feature(command: &str, syntax: Syntax) -> Option<(char, &'static str)> {
    let mut single = false;
    let mut double = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        if single {
            single = c != '\'';
            continue;
        }
        match c {
            '\\' if syntax == Syntax::Posix => {
                chars.next();
            }
            '\'' if !double => single = true,
            '"' => double = !double,
            '$' => return Some((c, "variable or command expansion")),
            '`' => return Some((c, "command substitution")),
            '|' if !double => return Some((c, "a pipe")),
            '&' if !double => return Some((c, "`&` or `&&`")),
            ';' if !double => return Some((c, "a command separator")),
            '<' | '>' if !double => return Some((c, "redirection")),
            '(' | ')' if !double => return Some((c, "a subshell")),
            _ => {}
        }
    }
    None
}

impl TypeDef for CommandTunnel {
    /// `ssh` is located like the ssh types; anything else on PATH.
    fn program(&self) -> Program {
        match self.words[0].as_str() {
            "ssh" => Program::Tool(Tool::Ssh),
            other => Program::Command(other.to_owned()),
        }
    }

    fn args(&self, _forwards: &[Forward]) -> Vec<String> {
        self.words[1..].to_vec()
    }

    fn readiness(&self, forwards: &[Forward]) -> Readiness {
        if self.process_alive {
            Readiness::AliveFor(ALIVE_READY_AFTER)
        } else {
            Readiness::Listening(listen_addrs(forwards))
        }
    }

    /// ssh's patterns if the command is ssh, plus the user's own patterns.
    fn output_patterns(&self) -> Vec<OutputPattern> {
        let mut patterns = match self.program() {
            Program::Tool(tool) => tool.patterns(),
            Program::Command(_) => Vec::new(),
        };
        let hint = "{tunnel} printed a credential error; fix the credentials, then start it again"
            .to_owned();
        patterns.extend(
            self.credential_patterns
                .iter()
                .filter_map(|p| user_pattern(p, FailureReason::Credentials, hint.clone())),
        );
        patterns
    }

    fn hint_vars(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::shell_feature;
    use crate::types::Syntax;

    fn posix(c: &str) -> Option<(char, &'static str)> {
        shell_feature(c, Syntax::Posix)
    }

    #[test]
    fn shell_features() {
        assert_eq!(posix("a b 'c d'"), None);
        assert_eq!(posix("a | b").map(|x| x.0), Some('|'));
        assert_eq!(posix("a && b").map(|x| x.0), Some('&'));
        assert_eq!(posix("a; b").map(|x| x.0), Some(';'));
        assert_eq!(posix("a > log").map(|x| x.0), Some('>'));
        assert_eq!(posix("echo $HOME").map(|x| x.0), Some('$'));
        assert_eq!(posix("echo \"$HOME\"").map(|x| x.0), Some('$'));
        assert_eq!(posix("echo '$HOME | x'"), None);
        assert_eq!(posix("echo \"a|b\""), None);
        assert_eq!(posix("echo a\\|b"), None);
    }

    #[test]
    fn a_backslash_escapes_nothing_on_windows() {
        let windows = |c: &str| shell_feature(c, Syntax::Windows).map(|x| x.0);
        assert_eq!(
            windows("C:\\tools\\gcloud.cmd compute start-iap-tunnel db 5432"),
            None
        );
        // `\|` is not an escaped pipe on Windows; the pipe still needs a shell.
        assert_eq!(windows("echo a\\|b"), Some('|'));
        assert_eq!(windows("'C:\\dir\\' | x"), Some('|'));
    }
}
