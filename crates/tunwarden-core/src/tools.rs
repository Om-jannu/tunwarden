//! Locating programs (PRD FR-46, FR-36, design 6.4).
//!
//! The daemon is often started by the desktop app or at login, and on macOS
//! those processes do not get the shell's PATH. So a program is looked up in
//! this order: `tools.ssh` for ssh, the daemon's PATH, the directories in
//! `tools.path`, then the usual install directories for the OS. `doctor`
//! shows which one was used.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::config::{Config, ToolPaths};
use crate::paths::{Env, Os, expand_tilde};
use crate::types::{Program, Tool};

/// Where a program was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `tools.ssh` in the config.
    Config,
    /// The daemon's PATH.
    Path,
    /// A directory in `tools.path`.
    ToolsPath,
    /// A usual install directory for the OS.
    KnownLocation,
    /// The command was given as a path.
    Explicit,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Config => "set in [tools]",
            Source::Path => "found on PATH",
            Source::ToolsPath => "found in tools.path",
            Source::KnownLocation => "found in a usual install location",
            Source::Explicit => "path in the command",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("ssh is set to {0} in [tools], but that file does not exist")]
    ConfiguredMissing(PathBuf),
    #[error(
        "{0} was not found on PATH, in tools.path or in the usual install locations; install it, use its full path, or add its directory to `tools.path` in the config"
    )]
    NotFound(String),
    #[error("{0} does not exist")]
    PathMissing(PathBuf),
}

/// Finds the program a tunnel runs.
pub fn resolve(
    program: &Program,
    tools: &ToolPaths,
    env: &impl Env,
    os: Os,
) -> Result<Resolved, ResolveError> {
    match program {
        Program::Tool(Tool::Ssh) => {
            if let Some(path) = &tools.ssh {
                return if path.is_file() {
                    Ok(Resolved {
                        path: path.clone(),
                        source: Source::Config,
                    })
                } else {
                    Err(ResolveError::ConfiguredMissing(path.clone()))
                };
            }
            search("ssh", tools, env, os).ok_or_else(|| ResolveError::NotFound("ssh".into()))
        }
        Program::Command(command) => {
            if command.contains('/') || command.contains('\\') {
                let path =
                    expand_tilde(Path::new(command), env).unwrap_or_else(|| PathBuf::from(command));
                return if path.is_file() {
                    Ok(Resolved {
                        path,
                        source: Source::Explicit,
                    })
                } else {
                    Err(ResolveError::PathMissing(path))
                };
            }
            search(command, tools, env, os).ok_or_else(|| ResolveError::NotFound(command.clone()))
        }
    }
}

fn search(name: &str, tools: &ToolPaths, env: &impl Env, os: Os) -> Option<Resolved> {
    let cwd = env.home().unwrap_or_else(|| PathBuf::from("/"));
    let find = |dirs: Option<OsString>, source: Source| {
        which::which_in(name, dirs, &cwd)
            .ok()
            .map(|path| Resolved { path, source })
    };
    if let Some(path_var) = env.var("PATH")
        && let Some(found) = find(Some(path_var), Source::Path)
    {
        return Some(found);
    }
    if !tools.path.is_empty()
        && let Some(found) = find(std::env::join_paths(&tools.path).ok(), Source::ToolsPath)
    {
        return Some(found);
    }
    find(
        std::env::join_paths(known_dirs(env, os)).ok(),
        Source::KnownLocation,
    )
}

/// Usual install directories per OS. No vendor-specific directories: those
/// belong in `tools.path`.
pub fn known_dirs(env: &impl Env, os: Os) -> Vec<PathBuf> {
    let home = env.home();
    let in_home = |rel: &str| home.as_ref().map(|h| h.join(rel));
    let dirs: Vec<Option<PathBuf>> = match os {
        Os::MacOs => vec![
            Some("/opt/homebrew/bin".into()),
            Some("/usr/local/bin".into()),
            Some("/usr/bin".into()),
            in_home(".local/bin"),
        ],
        Os::Linux => vec![
            Some("/usr/local/bin".into()),
            Some("/usr/bin".into()),
            Some("/snap/bin".into()),
            in_home(".local/bin"),
        ],
        Os::Windows => vec![
            env.var("SystemRoot")
                .map(|d| PathBuf::from(d).join("System32\\OpenSSH")),
        ],
    };
    dirs.into_iter().flatten().collect()
}

/// The programs a config needs: every tunnel's program and every login's
/// `check` and `login` program. In config order, without repeats.
pub fn needed(config: &Config) -> Vec<Program> {
    let mut out: Vec<Program> = Vec::new();
    let mut add = |p: Program| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for t in &config.tunnels {
        add(t.launch_plan(None, None).program);
    }
    for l in &config.logins {
        for words in [&l.check, &l.login].into_iter().flatten() {
            add(Program::Command(words[0].clone()));
        }
    }
    out
}

/// The version line of ssh. Other programs are not run just to ask.
pub async fn version(program: &Program, path: &Path) -> Option<String> {
    let Program::Tool(Tool::Ssh) = program else {
        return None;
    };
    let mut cmd = tokio::process::Command::new(path);
    cmd.arg("-V")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        cmd.spawn()?.wait_with_output().await
    })
    .await
    .ok()?
    .ok()?;
    // ssh prints its version on stderr.
    let text = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .into_owned();
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

/// Locates every program `config` needs, as this process sees them.
pub async fn check_all(config: &Config) -> Vec<crate::proto::ToolCheck> {
    let mut checks = Vec::new();
    for program in needed(config) {
        let check = match resolve(
            &program,
            &config.tools,
            &crate::paths::SystemEnv,
            Os::current(),
        ) {
            Ok(r) => crate::proto::ToolCheck {
                program: program.name().to_owned(),
                path: Some(r.path.display().to_string()),
                source: Some(r.source.describe().to_owned()),
                version: version(&program, &r.path).await,
                error: None,
            },
            Err(e) => crate::proto::ToolCheck {
                program: program.name().to_owned(),
                path: None,
                source: None,
                version: None,
                error: Some(e.to_string()),
            },
        };
        checks.push(check);
    }
    checks
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::paths::tests::FakeEnv;
    use std::os::unix::fs::PermissionsExt;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn cmd(name: &str) -> Program {
        Program::Command(name.into())
    }

    #[test]
    fn configured_ssh_wins_and_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = executable(dir.path(), "ssh");
        let tools = ToolPaths {
            ssh: Some(ssh.clone()),
            ..ToolPaths::default()
        };
        let r = resolve(
            &Program::Tool(Tool::Ssh),
            &tools,
            &FakeEnv::with_home("/nohome"),
            Os::Linux,
        )
        .unwrap();
        assert_eq!(
            r,
            Resolved {
                path: ssh,
                source: Source::Config
            }
        );
        let tools = ToolPaths {
            ssh: Some("/nonexistent/ssh".into()),
            ..ToolPaths::default()
        };
        let e = resolve(
            &Program::Tool(Tool::Ssh),
            &tools,
            &FakeEnv::with_home("/nohome"),
            Os::Linux,
        )
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "ssh is set to /nonexistent/ssh in [tools], but that file does not exist"
        );
    }

    #[test]
    fn path_then_tools_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let on_path = executable(a.path(), "mytool");
        let extra = executable(b.path(), "other");
        let env = FakeEnv::with_home("/nohome").set("PATH", a.path().to_str().unwrap());
        let tools = ToolPaths {
            ssh: None,
            path: vec![b.path().to_path_buf()],
        };
        assert_eq!(
            resolve(&cmd("mytool"), &tools, &env, Os::Linux).unwrap(),
            Resolved {
                path: on_path,
                source: Source::Path
            }
        );
        assert_eq!(
            resolve(&cmd("other"), &tools, &env, Os::Linux).unwrap(),
            Resolved {
                path: extra,
                source: Source::ToolsPath
            }
        );
    }

    #[test]
    fn found_in_a_known_location() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = executable(&bin, "mytool");
        let env = FakeEnv::with_home(home.path().to_str().unwrap()).set("PATH", "/nonexistent");
        let r = resolve(&cmd("mytool"), &ToolPaths::default(), &env, Os::Linux).unwrap();
        assert_eq!(
            r,
            Resolved {
                path: tool,
                source: Source::KnownLocation
            }
        );
    }

    #[test]
    fn not_found_message() {
        let env = FakeEnv::with_home("/nonexistent").set("PATH", "/nonexistent");
        let e = resolve(&cmd("mytool"), &ToolPaths::default(), &env, Os::Windows).unwrap_err();
        assert_eq!(
            e.to_string(),
            "mytool was not found on PATH, in tools.path or in the usual install locations; install it, use its full path, or add its directory to `tools.path` in the config"
        );
    }

    #[test]
    fn commands_given_as_paths() {
        let dir = tempfile::tempdir().unwrap();
        let tool = executable(dir.path(), "my-tunnel");
        let r = resolve(
            &cmd(tool.to_str().unwrap()),
            &ToolPaths::default(),
            &FakeEnv::with_home("/nohome"),
            Os::MacOs,
        )
        .unwrap();
        assert_eq!(r.source, Source::Explicit);
        assert!(matches!(
            resolve(
                &cmd("/nonexistent/my-tunnel"),
                &ToolPaths::default(),
                &FakeEnv::with_home("/nohome"),
                Os::MacOs
            ),
            Err(ResolveError::PathMissing(_))
        ));
    }
}
