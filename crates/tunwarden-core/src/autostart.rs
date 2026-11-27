//! Starting the daemon at login (PRD FR-15, `tunwarden autostart`).
//!
//! - macOS: a launchd agent, `~/Library/LaunchAgents/<LABEL>.plist`.
//! - Linux: a systemd user unit, `$XDG_CONFIG_HOME/systemd/user/tunwarden.service`,
//!   enabled with `systemctl --user enable`.
//! - Windows: a value in `HKCU\...\CurrentVersion\Run` that runs `daemon start`.
//!
//! Enabling only registers the entry: the daemon running now is left alone
//! and the entry takes effect at the next login. No entry restarts the daemon
//! when it exits, so `tunwarden daemon stop` keeps it stopped and a second
//! daemon refused by the lock does not loop.
//!
//! On macOS and Linux the entry records the `PATH` of the shell that ran
//! `enable`, because login items do not get the shell's PATH (FR-36).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::paths::{Env, Os};

/// The launchd label and the base name of the plist.
pub const LABEL: &str = "io.github.om-jannu.tunwarden";
/// The systemd unit name.
pub const UNIT: &str = "tunwarden.service";
/// The value name under the Windows `Run` key.
pub const RUN_VALUE: &str = "Tunwarden";
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// What the login entry starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The `tunwarden` binary.
    pub exe: PathBuf,
    /// Passed as `--config`, so the daemon uses the same file.
    pub config: PathBuf,
    /// Set as `TUNWARDEN_STATE_DIR` when the user overrode it.
    pub state_dir_override: Option<PathBuf>,
    /// The daemon's log; launchd and systemd append the daemon's output there.
    pub log: PathBuf,
    /// The PATH to give the daemon (macOS and Linux).
    pub path: Option<OsString>,
    /// `exe` is the desktop app, started hidden in the tray (`--background`),
    /// which then starts the daemon. Used on Windows, where the CLI is a
    /// console program and would flash a window at login.
    pub app: bool,
}

impl Entry {
    /// The command the entry runs, as words.
    pub fn args(&self, os: Os) -> Vec<String> {
        if self.app {
            return vec![self.exe.display().to_string(), "--background".to_owned()];
        }
        let mut words = vec![
            self.exe.display().to_string(),
            "--config".to_owned(),
            self.config.display().to_string(),
            "daemon".to_owned(),
        ];
        // Windows has no service manager here: `start` detaches the daemon
        // and returns. launchd and systemd keep `run` as their process.
        words.push(if os == Os::Windows { "start" } else { "run" }.to_owned());
        words
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = Vec::new();
        if let Some(p) = &self.path {
            env.push(("PATH", p.to_string_lossy().into_owned()));
        }
        if let Some(s) = &self.state_dir_override {
            env.push((crate::paths::STATE_DIR_ENV, s.display().to_string()));
        }
        env
    }
}

/// Where the entry lives.
pub fn location(env: &impl Env, os: Os) -> Option<PathBuf> {
    match os {
        Os::MacOs => Some(
            env.home()?
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        ),
        Os::Linux => {
            let base = env
                .var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| env.home().map(|h| h.join(".config")))?;
            Some(base.join("systemd/user").join(UNIT))
        }
        Os::Windows => None,
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The launchd agent. `RunAtLoad` starts it at login; there is no
/// `KeepAlive`, so it is not restarted after it exits.
pub fn launchd_plist(e: &Entry) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <!-- Written by `tunwarden autostart enable`; remove with `tunwarden autostart disable`. -->\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    s.push_str(&format!("  <key>Label</key>\n  <string>{LABEL}</string>\n"));
    s.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in e.args(Os::MacOs) {
        s.push_str(&format!("    <string>{}</string>\n", xml(&a)));
    }
    s.push_str("  </array>\n");
    let env = e.env();
    if !env.is_empty() {
        s.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in env {
            s.push_str(&format!(
                "    <key>{k}</key>\n    <string>{}</string>\n",
                xml(&v)
            ));
        }
        s.push_str("  </dict>\n");
    }
    let log = xml(&e.log.display().to_string());
    s.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    s.push_str("  <key>ProcessType</key>\n  <string>Background</string>\n");
    s.push_str(&format!(
        "  <key>StandardOutPath</key>\n  <string>{log}</string>\n  <key>StandardErrorPath</key>\n  <string>{log}</string>\n"
    ));
    s.push_str("</dict>\n</plist>\n");
    s
}

/// A value quoted for a systemd unit: double quotes, backslash escapes, and
/// `%` doubled because systemd expands `%` specifiers.
fn systemd_quote(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

/// The systemd user unit. No `Restart=`, for the same reason as launchd.
pub fn systemd_unit(e: &Entry) -> String {
    let mut s = String::from(
        "# Written by `tunwarden autostart enable`; remove with `tunwarden autostart disable`.\n\
         [Unit]\nDescription=Tunwarden tunnel daemon\nDocumentation=https://github.com/Om-jannu/tunwarden\n\n\
         [Service]\nType=simple\n",
    );
    let exec: Vec<String> = e.args(Os::Linux).iter().map(|a| systemd_quote(a)).collect();
    s.push_str(&format!("ExecStart={}\n", exec.join(" ")));
    for (k, v) in e.env() {
        s.push_str(&format!(
            "Environment={}\n",
            systemd_quote(&format!("{k}={v}"))
        ));
    }
    let log = e.log.display().to_string().replace('%', "%%");
    s.push_str(&format!(
        "StandardOutput=append:{log}\nStandardError=append:{log}\n"
    ));
    // The daemon stops its tunnels itself on SIGTERM (FR-12); systemd only
    // signals the daemon, then cleans up whatever is left after the timeout.
    s.push_str("KillMode=mixed\nTimeoutStopSec=30\n\n[Install]\nWantedBy=default.target\n");
    s
}

/// The command line stored under the Windows `Run` key.
pub fn windows_command(e: &Entry) -> String {
    e.args(Os::Windows)
        .iter()
        .map(|a| {
            if a.contains(' ') || a.is_empty() {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The binary a written entry starts, read back from its text.
pub fn exe_in(text: &str, os: Os) -> Option<PathBuf> {
    match os {
        Os::MacOs => {
            let after = text.split("<key>ProgramArguments</key>").nth(1)?;
            let start = after.find("<string>")? + "<string>".len();
            let end = after[start..].find("</string>")? + start;
            Some(PathBuf::from(
                after[start..end]
                    .replace("&quot;", "\"")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&"),
            ))
        }
        Os::Linux => {
            let line = text.lines().find(|l| l.starts_with("ExecStart=\""))?;
            let rest = &line["ExecStart=\"".len()..];
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    '"' => break,
                    _ => out.push(c),
                }
            }
            Some(PathBuf::from(out.replace("%%", "%")))
        }
        Os::Windows => {
            let t = text.trim();
            let exe = match t.strip_prefix('"') {
                Some(rest) => rest.split('"').next()?,
                None => t.split(' ').next()?,
            };
            Some(PathBuf::from(exe))
        }
    }
}

/// A reason to doubt the binary is a good one to start at every login.
pub fn exe_warning(exe: &Path) -> Option<String> {
    let text = exe.display().to_string().replace('\\', "/");
    let temp = std::env::temp_dir();
    if text.contains("/_npx/") {
        Some("this tunwarden runs from npx's cache, which can be cleared; install it with `npm i -g tunwarden` and enable autostart again".into())
    } else if text.contains("/target/debug/") || text.contains("/target/release/") {
        Some("this tunwarden is a build in a cargo target directory; the entry breaks when that build is cleaned or rebuilt elsewhere".into())
    } else if exe.starts_with(&temp) {
        Some("this tunwarden is in a temporary directory, which can be cleared".into())
    } else {
        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AutostartError {
    #[error("cannot find your home directory")]
    NoHome,
    #[error("cannot write {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("cannot remove {0}: {1}")]
    Remove(PathBuf, std::io::Error),
    #[error("{0}")]
    Command(String),
}

/// The result of `enable`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enabled {
    pub location: String,
    pub command: String,
    pub warning: Option<String>,
    pub notes: Vec<String>,
}

/// The result of `status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub enabled: bool,
    pub location: String,
    /// The binary the entry starts.
    pub exe: Option<PathBuf>,
    /// Why the entry will not work, for example a binary that is gone.
    pub problem: Option<String>,
}

fn display_command(e: &Entry, os: Os) -> String {
    match os {
        Os::Windows => windows_command(e),
        _ => e
            .args(os)
            .iter()
            .map(|w| crate::types::quote(w))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Registers the entry for the current OS, replacing an older one.
pub fn enable(e: &Entry, env: &impl Env) -> Result<Enabled, AutostartError> {
    let os = Os::current();
    let mut notes = Vec::new();
    let location = match os {
        Os::MacOs => {
            let path = location(env, os).ok_or(AutostartError::NoHome)?;
            write(&path, &launchd_plist(e))?;
            path.display().to_string()
        }
        Os::Linux => {
            let path = location(env, os).ok_or(AutostartError::NoHome)?;
            write(&path, &systemd_unit(e))?;
            let enabled = systemctl(&["daemon-reload"]).and_then(|_| systemctl(&["enable", UNIT]));
            if let Err(problem) = enabled {
                let _ = std::fs::remove_file(&path);
                return Err(AutostartError::Command(format!(
                    "autostart needs a systemd user session, and `systemctl --user` failed: {problem}"
                )));
            }
            notes.push(
                "to start it at boot, before you log in, also run `loginctl enable-linger`".into(),
            );
            path.display().to_string()
        }
        Os::Windows => {
            windows::set(&windows_command(e)).map_err(AutostartError::Command)?;
            format!(r"{RUN_KEY}\{RUN_VALUE}")
        }
    };
    notes.insert(
        0,
        "it takes effect at your next login; the daemon running now is not changed".into(),
    );
    Ok(Enabled {
        location,
        command: display_command(e, os),
        warning: exe_warning(&e.exe),
        notes,
    })
}

/// Removes the entry. Returns where it was, or `None` if there was none.
pub fn disable(env: &impl Env) -> Result<Option<String>, AutostartError> {
    let os = Os::current();
    match os {
        Os::MacOs | Os::Linux => {
            let path = location(env, os).ok_or(AutostartError::NoHome)?;
            if !path.exists() {
                return Ok(None);
            }
            if os == Os::Linux {
                // Removes the enablement link; the file goes next either way.
                let _ = systemctl(&["disable", UNIT]);
            }
            std::fs::remove_file(&path).map_err(|e| AutostartError::Remove(path.clone(), e))?;
            if os == Os::Linux {
                let _ = systemctl(&["daemon-reload"]);
            }
            Ok(Some(path.display().to_string()))
        }
        Os::Windows => match windows::get().map_err(AutostartError::Command)? {
            None => Ok(None),
            Some(_) => {
                windows::delete().map_err(AutostartError::Command)?;
                Ok(Some(format!(r"{RUN_KEY}\{RUN_VALUE}")))
            }
        },
    }
}

/// Whether an entry exists, and whether the binary it starts is still there.
pub fn status(env: &impl Env) -> Result<Status, AutostartError> {
    let os = Os::current();
    let (location, text) = match os {
        Os::MacOs | Os::Linux => {
            let path = location(env, os).ok_or(AutostartError::NoHome)?;
            let text = std::fs::read_to_string(&path).ok();
            (path.display().to_string(), text)
        }
        Os::Windows => (
            format!(r"{RUN_KEY}\{RUN_VALUE}"),
            windows::get().map_err(AutostartError::Command)?,
        ),
    };
    let Some(text) = text else {
        return Ok(Status {
            enabled: false,
            location,
            exe: None,
            problem: None,
        });
    };
    let exe = exe_in(&text, os);
    let mut problem = match &exe {
        Some(p) if !p.is_file() => Some(format!(
            "it starts {}, which no longer exists; run `tunwarden autostart enable` again",
            p.display()
        )),
        None => Some("the entry cannot be read; run `tunwarden autostart enable` again".into()),
        _ => None,
    };
    if os == Os::Linux
        && problem.is_none()
        && let Err(e) = systemctl(&["is-enabled", UNIT])
    {
        problem = Some(format!(
            "the unit file exists but systemd does not have it enabled ({e}); run `tunwarden autostart enable` again"
        ));
    }
    Ok(Status {
        enabled: true,
        location,
        exe,
        problem,
    })
}

fn write(path: &Path, text: &str) -> Result<(), AutostartError> {
    let dir = path.parent().expect("an entry path has a parent");
    std::fs::create_dir_all(dir).map_err(|e| AutostartError::Write(dir.to_owned(), e))?;
    crate::fsutil::write_atomic(path, text.as_bytes())
        .map_err(|e| AutostartError::Write(path.to_owned(), e))
}

/// Runs `systemctl --user ARGS`; the error is its message.
fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        Err(if text.is_empty() {
            format!("exit status {}", out.status)
        } else {
            text.lines().next().unwrap_or(text).to_owned()
        })
    }
}

/// The `Run` value, through `reg.exe` (no registry crate needed).
#[cfg(windows)]
mod windows {
    use super::{RUN_KEY, RUN_VALUE};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    fn reg(args: &str) -> Result<String, String> {
        // `raw_arg` passes the quotes in the value through unchanged.
        let out = Command::new("reg")
            .raw_arg(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run reg.exe: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if out.status.success() {
            Ok(stdout)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }

    pub fn set(command: &str) -> Result<(), String> {
        let value = command.replace('"', "\\\"");
        reg(&format!(
            "add \"{RUN_KEY}\" /v {RUN_VALUE} /t REG_SZ /d \"{value}\" /f"
        ))
        .map(|_| ())
    }

    pub fn get() -> Result<Option<String>, String> {
        match reg(&format!("query \"{RUN_KEY}\" /v {RUN_VALUE}")) {
            // Output: `    Tunwarden    REG_SZ    "C:\...\tunwarden.exe" --config ...`
            Ok(out) => Ok(out
                .lines()
                .find_map(|l| l.split_once("REG_SZ"))
                .map(|(_, v)| v.trim().to_owned())),
            Err(_) => Ok(None),
        }
    }

    pub fn delete() -> Result<(), String> {
        reg(&format!("delete \"{RUN_KEY}\" /v {RUN_VALUE} /f")).map(|_| ())
    }
}

#[cfg(not(windows))]
mod windows {
    pub fn set(_: &str) -> Result<(), String> {
        unreachable!("only called on Windows")
    }
    pub fn get() -> Result<Option<String>, String> {
        unreachable!("only called on Windows")
    }
    pub fn delete() -> Result<(), String> {
        unreachable!("only called on Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::FakeEnv;

    fn entry() -> Entry {
        Entry {
            exe: PathBuf::from("/Users/me/.cargo/bin/tunwarden"),
            config: PathBuf::from("/Users/me/My Config/tw & co.toml"),
            state_dir_override: None,
            log: PathBuf::from("/Users/me/Library/Application Support/tunwarden/daemon.log"),
            path: Some(OsString::from("/opt/homebrew/bin:/usr/bin:/bin")),
            app: false,
        }
    }

    #[test]
    fn locations() {
        let env = FakeEnv::with_home("/home/me");
        assert_eq!(
            location(&env, Os::MacOs).unwrap(),
            PathBuf::from("/home/me/Library/LaunchAgents/io.github.om-jannu.tunwarden.plist")
        );
        assert_eq!(
            location(&env, Os::Linux).unwrap(),
            PathBuf::from("/home/me/.config/systemd/user/tunwarden.service")
        );
        let env = env.set("XDG_CONFIG_HOME", "/xdg");
        assert_eq!(
            location(&env, Os::Linux).unwrap(),
            PathBuf::from("/xdg/systemd/user/tunwarden.service")
        );
        assert_eq!(location(&env, Os::Windows), None);
    }

    #[test]
    fn launchd_agent() {
        let plist = launchd_plist(&entry());
        assert!(plist.contains(
            "  <array>\n    <string>/Users/me/.cargo/bin/tunwarden</string>\n    <string>--config</string>\n    <string>/Users/me/My Config/tw &amp; co.toml</string>\n    <string>daemon</string>\n    <string>run</string>\n  </array>"
        ), "{plist}");
        assert!(
            plist.contains("<key>PATH</key>\n    <string>/opt/homebrew/bin:/usr/bin:/bin</string>")
        );
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(
            !plist.contains("KeepAlive"),
            "never restarted after it exits"
        );
        assert!(!plist.contains("TUNWARDEN_STATE_DIR"));
        assert_eq!(
            exe_in(&plist, Os::MacOs).unwrap(),
            PathBuf::from("/Users/me/.cargo/bin/tunwarden")
        );
    }

    #[test]
    fn systemd_unit_quotes_values() {
        let mut e = entry();
        e.state_dir_override = Some(PathBuf::from("/tmp/100% state"));
        let unit = systemd_unit(&e);
        assert!(unit.contains(
            "ExecStart=\"/Users/me/.cargo/bin/tunwarden\" \"--config\" \"/Users/me/My Config/tw & co.toml\" \"daemon\" \"run\"\n"
        ), "{unit}");
        assert!(unit.contains("Environment=\"PATH=/opt/homebrew/bin:/usr/bin:/bin\"\n"));
        assert!(unit.contains("Environment=\"TUNWARDEN_STATE_DIR=/tmp/100%% state\"\n"));
        assert!(unit.contains(
            "StandardError=append:/Users/me/Library/Application Support/tunwarden/daemon.log\n"
        ));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("Restart="));
        assert_eq!(
            exe_in(&unit, Os::Linux).unwrap(),
            PathBuf::from("/Users/me/.cargo/bin/tunwarden")
        );
    }

    #[test]
    fn windows_run_value() {
        let e = Entry {
            exe: PathBuf::from(r"C:\Users\me\.cargo\bin\tunwarden.exe"),
            config: PathBuf::from(r"C:\Users\me\AppData\Roaming\tunwarden\config.toml"),
            state_dir_override: None,
            log: PathBuf::from(r"C:\x\daemon.log"),
            path: None,
            app: false,
        };
        let cmd = windows_command(&e);
        assert_eq!(
            cmd,
            r"C:\Users\me\.cargo\bin\tunwarden.exe --config C:\Users\me\AppData\Roaming\tunwarden\config.toml daemon start"
        );
        assert_eq!(exe_in(&cmd, Os::Windows).unwrap(), e.exe);
        let app = Entry {
            exe: PathBuf::from(r"C:\Program Files\Tunwarden\Tunwarden.exe"),
            app: true,
            ..e.clone()
        };
        assert_eq!(
            windows_command(&app),
            r#""C:\Program Files\Tunwarden\Tunwarden.exe" --background"#
        );
        assert_eq!(
            exe_in(&windows_command(&app), Os::Windows).unwrap(),
            app.exe
        );
        let spaced = r#""C:\Program Files\tw\tunwarden.exe" --config x daemon start"#;
        assert_eq!(
            exe_in(spaced, Os::Windows).unwrap(),
            PathBuf::from(r"C:\Program Files\tw\tunwarden.exe")
        );
    }

    #[test]
    fn warnings_for_short_lived_binaries() {
        assert!(
            exe_warning(Path::new(
                "/Users/me/.npm/_npx/abc/node_modules/tunwarden/tunwarden"
            ))
            .unwrap()
            .contains("npx")
        );
        assert!(
            exe_warning(Path::new("/src/tunwarden/target/debug/tunwarden"))
                .unwrap()
                .contains("cargo target")
        );
        assert_eq!(
            exe_warning(Path::new("/Users/me/.cargo/bin/tunwarden")),
            None
        );
    }
}
