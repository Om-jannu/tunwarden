//! Per-OS locations for the config file and the state directory (PRD FR-1, FR-51).
//!
//! Every function takes an [`Env`] and an [`Os`], so the rules for all three
//! operating systems can be tested on any machine.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Environment variable that overrides the config file location.
pub const CONFIG_ENV: &str = "TUNWARDEN_CONFIG";
/// Environment variable that overrides the state directory (mainly for tests).
pub const STATE_DIR_ENV: &str = "TUNWARDEN_STATE_DIR";

const APP_DIR: &str = "tunwarden";
const CONFIG_FILE: &str = "config.toml";

/// The operating system whose conventions apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    /// The OS this binary was built for.
    pub const fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// Access to environment variables and the home directory.
pub trait Env {
    /// The value of `key`, or `None` if it is unset or empty.
    fn var(&self, key: &str) -> Option<OsString>;
    /// The current user's home directory.
    fn home(&self) -> Option<PathBuf>;
}

/// The real process environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemEnv;

impl Env for SystemEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key).filter(|v| !v.is_empty())
    }

    fn home(&self) -> Option<PathBuf> {
        std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("cannot find your home directory; set {0} to choose a location")]
    NoHome(&'static str),
}

/// The config file to use. Order: the `--config` flag, then `TUNWARDEN_CONFIG`,
/// then the per-OS default.
///
/// - macOS and Linux: `$XDG_CONFIG_HOME/tunwarden/config.toml`, default `~/.config/tunwarden/config.toml`
/// - Windows: `%APPDATA%\tunwarden\config.toml`
pub fn config_file(flag: Option<&Path>, env: &impl Env, os: Os) -> Result<PathBuf, PathError> {
    if let Some(path) = flag {
        return expand_tilde(path, env).ok_or(PathError::NoHome(CONFIG_ENV));
    }
    if let Some(path) = env.var(CONFIG_ENV) {
        return expand_tilde(Path::new(&path), env).ok_or(PathError::NoHome(CONFIG_ENV));
    }
    let base = match os {
        Os::Windows => windows_dir(env, "APPDATA", &["AppData", "Roaming"])?,
        Os::MacOs | Os::Linux => xdg_dir(env, "XDG_CONFIG_HOME", &[".config"])?,
    };
    Ok(base.join(APP_DIR).join(CONFIG_FILE))
}

/// The directory for state, logs and the control socket. `TUNWARDEN_STATE_DIR`
/// overrides the per-OS default.
///
/// - macOS: `~/Library/Application Support/tunwarden`
/// - Linux: `$XDG_STATE_HOME/tunwarden`, default `~/.local/state/tunwarden`
/// - Windows: `%LOCALAPPDATA%\tunwarden`
pub fn state_dir(env: &impl Env, os: Os) -> Result<PathBuf, PathError> {
    if let Some(path) = env.var(STATE_DIR_ENV) {
        return expand_tilde(Path::new(&path), env).ok_or(PathError::NoHome(STATE_DIR_ENV));
    }
    let base = match os {
        Os::MacOs => home(env, STATE_DIR_ENV)?
            .join("Library")
            .join("Application Support"),
        Os::Linux => xdg_dir(env, "XDG_STATE_HOME", &[".local", "state"])?,
        Os::Windows => windows_dir(env, "LOCALAPPDATA", &["AppData", "Local"])?,
    };
    Ok(base.join(APP_DIR))
}

/// Expands a leading `~` (alone, or followed by `/` or `\`) to the home
/// directory. Other paths are returned unchanged. `~user` is not supported.
/// Returns `None` only when the path needs the home directory and it is unknown.
pub fn expand_tilde(path: &Path, env: &impl Env) -> Option<PathBuf> {
    let Some(text) = path.to_str() else {
        return Some(path.to_path_buf());
    };
    if text == "~" {
        return env.home();
    }
    match text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        Some(rest) => env.home().map(|home| home.join(rest)),
        None => Some(path.to_path_buf()),
    }
}

fn home(env: &impl Env, override_var: &'static str) -> Result<PathBuf, PathError> {
    env.home().ok_or(PathError::NoHome(override_var))
}

/// An XDG base directory. Relative values are ignored, as the XDG spec requires.
fn xdg_dir(env: &impl Env, var: &str, fallback: &[&str]) -> Result<PathBuf, PathError> {
    if let Some(dir) = env.var(var).map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Ok(dir);
    }
    Ok(join_all(home(env, CONFIG_ENV)?, fallback))
}

fn windows_dir(env: &impl Env, var: &str, fallback: &[&str]) -> Result<PathBuf, PathError> {
    if let Some(dir) = env.var(var) {
        return Ok(PathBuf::from(dir));
    }
    Ok(join_all(home(env, CONFIG_ENV)?, fallback))
}

fn join_all(mut base: PathBuf, parts: &[&str]) -> PathBuf {
    for part in parts {
        base.push(part);
    }
    base
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A fake environment for tests.
    #[derive(Default)]
    pub(crate) struct FakeEnv {
        pub vars: HashMap<String, String>,
        pub home: Option<PathBuf>,
    }

    impl FakeEnv {
        pub(crate) fn with_home(home: &str) -> Self {
            FakeEnv {
                vars: HashMap::new(),
                home: Some(PathBuf::from(home)),
            }
        }

        pub(crate) fn set(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_owned(), value.to_owned());
            self
        }
    }

    impl Env for FakeEnv {
        fn var(&self, key: &str) -> Option<OsString> {
            self.vars
                .get(key)
                .filter(|v| !v.is_empty())
                .map(OsString::from)
        }

        fn home(&self) -> Option<PathBuf> {
            self.home.clone()
        }
    }

    fn p(parts: &[&str]) -> PathBuf {
        join_all(PathBuf::from(parts[0]), &parts[1..])
    }

    #[test]
    fn config_flag_wins_over_env_and_default() {
        let env = FakeEnv::with_home("/home/me").set(CONFIG_ENV, "/env/config.toml");
        let path = config_file(Some(Path::new("~/flag.toml")), &env, Os::Linux).unwrap();
        assert_eq!(path, p(&["/home/me", "flag.toml"]));
    }

    #[test]
    fn config_env_wins_over_default() {
        let env = FakeEnv::with_home("/home/me").set(CONFIG_ENV, "/env/config.toml");
        assert_eq!(
            config_file(None, &env, Os::MacOs).unwrap(),
            PathBuf::from("/env/config.toml")
        );
    }

    #[test]
    fn config_default_per_os() {
        let env = FakeEnv::with_home("/home/me");
        let unix = p(&["/home/me", ".config", "tunwarden", "config.toml"]);
        assert_eq!(config_file(None, &env, Os::Linux).unwrap(), unix);
        assert_eq!(config_file(None, &env, Os::MacOs).unwrap(), unix);

        let env =
            FakeEnv::with_home("C:\\Users\\me").set("APPDATA", "C:\\Users\\me\\AppData\\Roaming");
        assert_eq!(
            config_file(None, &env, Os::Windows).unwrap(),
            p(&[
                "C:\\Users\\me\\AppData\\Roaming",
                "tunwarden",
                "config.toml"
            ])
        );
    }

    // Linux rules for XDG paths; `/xdg` is not absolute on a Windows host.
    #[cfg(unix)]
    #[test]
    fn config_honors_absolute_xdg_and_ignores_relative() {
        let env = FakeEnv::with_home("/home/me").set("XDG_CONFIG_HOME", "/xdg");
        assert_eq!(
            config_file(None, &env, Os::Linux).unwrap(),
            p(&["/xdg", "tunwarden", "config.toml"])
        );
        let env = FakeEnv::with_home("/home/me").set("XDG_CONFIG_HOME", "relative/dir");
        assert_eq!(
            config_file(None, &env, Os::Linux).unwrap(),
            p(&["/home/me", ".config", "tunwarden", "config.toml"])
        );
    }

    #[test]
    fn state_dir_per_os() {
        let env = FakeEnv::with_home("/Users/me");
        assert_eq!(
            state_dir(&env, Os::MacOs).unwrap(),
            p(&["/Users/me", "Library", "Application Support", "tunwarden"])
        );
        assert_eq!(
            state_dir(&env, Os::Linux).unwrap(),
            p(&["/Users/me", ".local", "state", "tunwarden"])
        );
        #[cfg(unix)]
        {
            let env = FakeEnv::with_home("/home/me").set("XDG_STATE_HOME", "/xdg/state");
            assert_eq!(
                state_dir(&env, Os::Linux).unwrap(),
                p(&["/xdg/state", "tunwarden"])
            );
        }
        let env = FakeEnv::with_home("C:\\Users\\me").set("LOCALAPPDATA", "C:\\Local");
        assert_eq!(
            state_dir(&env, Os::Windows).unwrap(),
            p(&["C:\\Local", "tunwarden"])
        );
    }

    #[test]
    fn state_dir_override() {
        let env = FakeEnv::with_home("/home/me").set(STATE_DIR_ENV, "~/tw-state");
        assert_eq!(
            state_dir(&env, Os::MacOs).unwrap(),
            p(&["/home/me", "tw-state"])
        );
    }

    #[test]
    fn missing_home_is_reported() {
        let env = FakeEnv::default();
        assert_eq!(
            config_file(None, &env, Os::Linux),
            Err(PathError::NoHome(CONFIG_ENV))
        );
        assert_eq!(
            state_dir(&env, Os::MacOs),
            Err(PathError::NoHome(STATE_DIR_ENV))
        );
    }

    #[test]
    fn tilde_expansion() {
        let env = FakeEnv::with_home("/home/me");
        assert_eq!(
            expand_tilde(Path::new("~"), &env),
            Some(PathBuf::from("/home/me"))
        );
        assert_eq!(
            expand_tilde(Path::new("~/a/b"), &env),
            Some(p(&["/home/me", "a/b"]))
        );
        assert_eq!(
            expand_tilde(Path::new("~other/x"), &env),
            Some(PathBuf::from("~other/x"))
        );
        assert_eq!(
            expand_tilde(Path::new("/abs"), &env),
            Some(PathBuf::from("/abs"))
        );
        assert_eq!(expand_tilde(Path::new("~/x"), &FakeEnv::default()), None);
    }
}
