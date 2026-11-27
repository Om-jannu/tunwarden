//! Loading and validating the config file (PRD 7.1, design section 3).
//!
//! The file is parsed in two passes. The first deserializes the fields Tunwarden
//! itself owns, and stops at the first type error with its exact location. The
//! second runs every semantic check (names, references, ports) and reports all
//! problems at once, each with a line, column and field path.
//!
//! Fields that belong to a tunnel's type (for example `command` or `host`) are
//! validated by the type itself (see [`crate::types`]).
//!
//! Nothing here knows about a particular cloud or tool: `[[logins]]` and
//! `[[profiles]]` hold the user's own commands and environment variables.

pub mod edit;
mod locate;

use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use locate::Locator;
pub use locate::{FieldPath, Position};

use crate::paths::{Env, SystemEnv, expand_tilde};
use crate::types::{self, Report, TunnelKind, TypeSettings, split_command_line};

/// The newest config schema version this build understands (FR-50).
pub const CURRENT_VERSION: u32 = 1;

const MAX_NAME_LEN: usize = 64;

/// Tunnel fields handled by core. Everything else in a tunnel table belongs to its type.
const COMMON_TUNNEL_KEYS: [&str; 9] = [
    "name",
    "type",
    "profile",
    "groups",
    "forwards",
    "auto_start",
    "auto_restart",
    "max_retries",
    "health_interval_secs",
];

// ---------------------------------------------------------------------------
// Validated model

/// A loaded, validated config.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub version: u32,
    pub settings: Settings,
    pub tools: ToolPaths,
    pub logins: Vec<Login>,
    pub profiles: Vec<Profile>,
    pub tunnels: Vec<TunnelConfig>,
}

impl Config {
    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn login(&self, name: &str) -> Option<&Login> {
        self.logins.iter().find(|l| l.name == name)
    }

    /// The profile and login a tunnel uses.
    pub fn context_of(&self, t: &TunnelConfig) -> (Option<&Profile>, Option<&Login>) {
        let profile = t.profile.as_deref().and_then(|n| self.profile(n));
        let login = profile
            .and_then(|p| p.login.as_deref())
            .and_then(|n| self.login(n));
        (profile, login)
    }

    /// An empty config, used when the file is missing or invalid.
    pub fn empty() -> Config {
        Config {
            version: CURRENT_VERSION,
            settings: Settings::default(),
            tools: ToolPaths::default(),
            logins: Vec::new(),
            profiles: Vec::new(),
            tunnels: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Restore running tunnels after a crash or reboot (FR-15).
    pub restore_on_start: bool,
    /// How often logins with a `check` command are checked (FR-22).
    pub login_check_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            restore_on_start: true,
            login_check_interval: Duration::from_secs(5 * 60),
        }
    }
}

/// Where programs are found (design 6.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolPaths {
    /// The ssh binary used by the ssh types and `trust`.
    pub ssh: Option<PathBuf>,
    /// Extra directories searched for commands, after PATH. Needed when the
    /// daemon does not get the shell's PATH (macOS GUI and login items).
    pub path: Vec<PathBuf>,
}

/// A login the user has to renew now and then, for any tool (FR-22 to FR-25).
#[derive(Clone, Debug, PartialEq)]
pub struct Login {
    pub name: String,
    /// A command that succeeds while the login is valid, split into words.
    pub check: Option<Vec<String>>,
    /// The interactive command that renews the login, split into words.
    pub login: Option<Vec<String>>,
    /// Regular expressions (ignoring case) for output that means "log in again".
    pub expired: Vec<String>,
    /// Environment for `check` and `login`.
    pub env: Vec<(String, String)>,
}

/// Settings shared by tunnels: environment, production flag, login (FR-2, FR-7).
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub env: Vec<(String, String)>,
    pub production: bool,
    pub login: Option<String>,
}

/// A tunnel entry (FR-3).
#[derive(Clone, Debug, PartialEq)]
pub struct TunnelConfig {
    pub name: String,
    pub kind: TunnelKind,
    pub profile: Option<String>,
    pub groups: Vec<String>,
    pub forwards: Vec<Forward>,
    pub auto_start: bool,
    pub auto_restart: bool,
    pub max_retries: u32,
    pub health_interval: Duration,
    /// The type's own validated fields.
    pub type_settings: TypeSettings,
}

/// One port mapping of a tunnel.
#[derive(Clone, Debug, PartialEq)]
pub struct Forward {
    /// Local bind address. Defaults to `127.0.0.1`.
    pub bind: IpAddr,
    pub local_port: u16,
    pub remote_host: Option<String>,
    pub remote_port: Option<u16>,
    /// Target host of an `ssh-remote` forward.
    pub local_host: Option<String>,
}

// ---------------------------------------------------------------------------
// Errors

/// One problem in the config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub position: Position,
    /// Field path such as `tunnels[2].forwards[0].local_port`; empty for file-level errors.
    pub field: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Position { line, column } = self.position;
        write!(f, "{}:{line}:{column}: ", self.file.display())?;
        if !self.field.is_empty() {
            write!(f, "{}: ", self.field)?;
        }
        f.write_str(&self.message)
    }
}

/// All problems found in a config file, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(ConfigErrors),
}

// ---------------------------------------------------------------------------
// Entry points

/// Reads and validates the config file at `path`.
pub fn load(path: &Path) -> Result<Config, LoadError> {
    let source = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&source, path, &SystemEnv).map_err(LoadError::Invalid)
}

/// Validates config text. `file` is only used in error messages.
pub fn parse(source: &str, file: &Path, env: &impl Env) -> Result<Config, ConfigErrors> {
    let mut ctx = Ctx {
        file,
        locator: Locator::new(source),
        errors: Vec::new(),
    };

    let raw: RawConfig = match toml::from_str(source) {
        Ok(raw) => raw,
        Err(e) => {
            let position = e
                .span()
                .map_or(Position::START, |s| Position::of_offset(source, s.start));
            return Err(ConfigErrors(vec![ConfigError {
                file: file.to_path_buf(),
                position,
                field: String::new(),
                message: e.message().trim().to_owned(),
            }]));
        }
    };
    // Cannot fail: the same text just deserialized as `RawConfig`.
    let table: toml::Table = toml::from_str(source).unwrap_or_default();

    let config = ctx.validate(raw, &table, env);
    if ctx.errors.is_empty() {
        Ok(config)
    } else {
        ctx.errors
            .sort_by_key(|e| (e.position.line, e.position.column));
        Err(ConfigErrors(ctx.errors))
    }
}

// ---------------------------------------------------------------------------
// Raw (deserialized) form

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: Option<i64>,
    #[serde(default)]
    settings: RawSettings,
    #[serde(default)]
    tools: RawTools,
    #[serde(default)]
    logins: Vec<RawLogin>,
    #[serde(default)]
    profiles: Vec<RawProfile>,
    #[serde(default)]
    tunnels: Vec<RawTunnel>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    restore_on_start: Option<bool>,
    login_check_interval_minutes: Option<i64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTools {
    ssh: Option<String>,
    #[serde(default)]
    path: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLogin {
    name: String,
    check: Option<String>,
    login: Option<String>,
    #[serde(default)]
    expired: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    name: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    production: Option<bool>,
    login: Option<String>,
}

/// Unknown fields are allowed here: they belong to the tunnel's type.
#[derive(Deserialize)]
struct RawTunnel {
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    profile: Option<String>,
    groups: Option<Vec<String>>,
    forwards: Option<Vec<RawForward>>,
    auto_start: Option<bool>,
    auto_restart: Option<bool>,
    max_retries: Option<i64>,
    health_interval_secs: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawForward {
    bind: Option<String>,
    local_port: Option<i64>,
    remote_host: Option<String>,
    remote_port: Option<i64>,
    local_host: Option<String>,
}

// ---------------------------------------------------------------------------
// Validation

struct Ctx<'a> {
    file: &'a Path,
    locator: Locator<'a>,
    errors: Vec<ConfigError>,
}

/// A local address a tunnel will listen on, remembered for conflict checks.
struct Listen {
    bind: IpAddr,
    port: u16,
    tunnel: String,
    path: FieldPath,
}

impl Ctx<'_> {
    fn err(&mut self, path: &FieldPath, message: impl Into<String>) {
        self.errors.push(ConfigError {
            file: self.file.to_path_buf(),
            position: self.locator.position(path),
            field: path.to_string(),
            message: message.into(),
        });
    }

    fn validate(&mut self, raw: RawConfig, table: &toml::Table, env: &impl Env) -> Config {
        let root = FieldPath::root();
        let version = self.version(raw.version, &root.key("version"));
        let settings = self.settings(raw.settings, &root.key("settings"));
        let tools = self.tools(raw.tools, &root.key("tools"), env);

        let logins_path = root.key("logins");
        let mut logins: Vec<Login> = Vec::new();
        for (i, raw) in raw.logins.into_iter().enumerate() {
            let path = logins_path.index(i);
            self.name("login", &raw.name, &path.key("name"));
            self.unique(
                "login",
                &raw.name,
                logins.iter().map(|l| &l.name),
                &logins_path,
                &path,
            );
            logins.push(self.login(raw, &path, env));
        }

        let profiles_path = root.key("profiles");
        let mut profiles: Vec<Profile> = Vec::new();
        for (i, raw) in raw.profiles.into_iter().enumerate() {
            let path = profiles_path.index(i);
            self.name("profile", &raw.name, &path.key("name"));
            self.unique(
                "profile",
                &raw.name,
                profiles.iter().map(|p| &p.name),
                &profiles_path,
                &path,
            );
            profiles.push(self.profile(raw, &path, &logins));
        }

        let raw_tunnel_tables = table
            .get("tunnels")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tunnels_path = root.key("tunnels");
        let mut tunnels: Vec<TunnelConfig> = Vec::new();
        let mut listens: Vec<Listen> = Vec::new();
        for (i, raw_tunnel) in raw.tunnels.into_iter().enumerate() {
            let path = tunnels_path.index(i);
            let name_path = path.key("name");
            self.name("tunnel", &raw_tunnel.name, &name_path);
            if let Some(other) = tunnels
                .iter()
                .position(|t| t.name.eq_ignore_ascii_case(&raw_tunnel.name))
            {
                self.err(
                    &name_path,
                    format!(
                        "tunnel name \"{}\" is already used by {} (names are compared ignoring case, because they become file names)",
                        raw_tunnel.name,
                        tunnels_path.index(other)
                    ),
                );
            }
            let settings = raw_tunnel_tables
                .get(i)
                .and_then(toml::Value::as_table)
                .map(|t| {
                    t.iter()
                        .filter(|(k, _)| !COMMON_TUNNEL_KEYS.contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(tunnel) =
                self.tunnel(raw_tunnel, settings, &path, &profiles, &mut listens, env)
            {
                tunnels.push(tunnel);
            }
        }

        Config {
            version,
            settings,
            tools,
            logins,
            profiles,
            tunnels,
        }
    }

    /// Reports a name already used by an earlier entry (ignoring case).
    fn unique<'n>(
        &mut self,
        what: &str,
        name: &str,
        earlier: impl Iterator<Item = &'n String>,
        list: &FieldPath,
        path: &FieldPath,
    ) {
        if let Some(other) = earlier
            .into_iter()
            .position(|n| n.eq_ignore_ascii_case(name))
        {
            self.err(
                &path.key("name"),
                format!(
                    "{what} name \"{name}\" is already used by {} (names are compared ignoring case)",
                    list.index(other)
                ),
            );
        }
    }

    fn version(&mut self, version: Option<i64>, path: &FieldPath) -> u32 {
        match version {
            None => {
                self.err(
                    path,
                    format!("missing `version = {CURRENT_VERSION}` at the top of the file"),
                );
                CURRENT_VERSION
            }
            Some(v) if v == i64::from(CURRENT_VERSION) => CURRENT_VERSION,
            Some(v) if v > i64::from(CURRENT_VERSION) => {
                self.err(
                    path,
                    format!(
                        "this config uses version {v}, but this Tunwarden only supports version {CURRENT_VERSION}; upgrade Tunwarden"
                    ),
                );
                CURRENT_VERSION
            }
            Some(v) => {
                self.err(
                    path,
                    format!("version {v} is not valid; use `version = {CURRENT_VERSION}`"),
                );
                CURRENT_VERSION
            }
        }
    }

    fn settings(&mut self, raw: RawSettings, path: &FieldPath) -> Settings {
        let mut settings = Settings::default();
        if let Some(restore) = raw.restore_on_start {
            settings.restore_on_start = restore;
        }
        if let Some(minutes) = raw.login_check_interval_minutes {
            let p = path.key("login_check_interval_minutes");
            if let Some(m) = self.range(minutes, 1, 1440, "minutes", &p) {
                settings.login_check_interval = Duration::from_secs(m * 60);
            }
        }
        settings
    }

    fn tools(&mut self, raw: RawTools, path: &FieldPath, env: &impl Env) -> ToolPaths {
        let ssh = raw
            .ssh
            .and_then(|v| self.abs_path(&v, &path.key("ssh"), env));
        let dirs = raw
            .path
            .iter()
            .enumerate()
            .filter_map(|(i, d)| self.abs_path(d, &path.key("path").index(i), env))
            .collect();
        ToolPaths { ssh, path: dirs }
    }

    fn login(&mut self, raw: RawLogin, path: &FieldPath, env: &impl Env) -> Login {
        if raw.check.is_none() && raw.login.is_none() {
            self.err(
                path,
                format!(
                    "login \"{}\" needs `check`, `login`, or both, for example `check = \"my-tool auth status\"`",
                    raw.name
                ),
            );
        }
        let mut command = |key: &str, value: Option<String>| -> Option<Vec<String>> {
            let value = value?;
            match split_command_line(&value, env) {
                Ok(words) => Some(words),
                Err(problem) => {
                    self.err(&path.key(key), problem);
                    None
                }
            }
        };
        let check = command("check", raw.check);
        let login = command("login", raw.login);
        for (i, pattern) in raw.expired.iter().enumerate() {
            if let Err(e) = regex::Regex::new(pattern) {
                let first = e
                    .to_string()
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                self.err(
                    &path.key("expired").index(i),
                    format!("invalid regular expression: {first}"),
                );
            }
        }
        let env_vars = self.env_vars(raw.env, &path.key("env"));
        Login {
            name: raw.name,
            check,
            login,
            expired: raw.expired,
            env: env_vars,
        }
    }

    fn profile(&mut self, raw: RawProfile, path: &FieldPath, logins: &[Login]) -> Profile {
        if let Some(name) = &raw.login
            && !logins.iter().any(|l| &l.name == name)
        {
            let known: Vec<&str> = logins.iter().map(|l| l.name.as_str()).collect();
            let list = if known.is_empty() {
                "no logins are defined; add a [[logins]] entry".to_owned()
            } else {
                format!("defined logins: {}", known.join(", "))
            };
            self.err(
                &path.key("login"),
                format!("no login named \"{name}\"; {list}"),
            );
        }
        Profile {
            env: self.env_vars(raw.env, &path.key("env")),
            name: raw.name,
            production: raw.production.unwrap_or(false),
            login: raw.login,
        }
    }

    /// Environment variable names must be usable on every OS.
    fn env_vars(
        &mut self,
        env: BTreeMap<String, String>,
        path: &FieldPath,
    ) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (k, v) in env {
            let ok_name = !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit());
            if !ok_name {
                self.err(&path.key(&k), format!("\"{k}\" is not a valid environment variable name; use letters, digits and '_'"));
            } else if v.contains('\0') {
                self.err(&path.key(&k), "contains a NUL character".to_owned());
            } else {
                out.push((k, v));
            }
        }
        out
    }

    fn tunnel(
        &mut self,
        raw: RawTunnel,
        settings: toml::Table,
        path: &FieldPath,
        profiles: &[Profile],
        listens: &mut Vec<Listen>,
        env: &dyn Env,
    ) -> Option<TunnelConfig> {
        let type_id = raw.kind.as_deref().unwrap_or("command");
        let kind = match TunnelKind::from_id(type_id) {
            Some(kind) => kind,
            None => {
                let ids: Vec<&str> = TunnelKind::ALL.iter().map(|k| k.id()).collect();
                let hint = format!(
                    "unknown type \"{type_id}\"; use one of: {}. Any other tool runs as type = \"command\"",
                    ids.join(", ")
                );
                self.err(&path.key("type"), hint);
                return None;
            }
        };

        let profile = self.profile_ref(raw.profile, path, profiles);

        let mut groups = Vec::new();
        for (i, group) in raw.groups.unwrap_or_default().into_iter().enumerate() {
            let p = path.key("groups").index(i);
            self.name("group", &group, &p);
            if groups.contains(&group) {
                self.err(&p, format!("group \"{group}\" is listed twice"));
            } else {
                groups.push(group);
            }
        }

        let forwards_path = path.key("forwards");
        let raw_forwards = raw.forwards.unwrap_or_default();
        if raw_forwards.is_empty() && kind.requires_forward() {
            self.err(
                &forwards_path,
                format!(
                    "a {kind} tunnel needs at least one forward, for example `forwards = [{{ local_port = 5433, remote_port = 5432 }}]`"
                ),
            );
        }
        let mut forwards = Vec::new();
        for (i, raw_forward) in raw_forwards.into_iter().enumerate() {
            let p = forwards_path.index(i);
            if let Some(forward) = self.forward(raw_forward, &p) {
                if kind.listens_locally() {
                    self.listen(&raw.name, forward.bind, forward.local_port, &p, listens);
                }
                forwards.push(forward);
            }
        }

        let max_retries = match raw.max_retries {
            Some(v) => self
                .range(v, 0, 1000, "retries", &path.key("max_retries"))
                .map_or(10, |v| v as u32),
            None => 10,
        };
        let health_interval = match raw.health_interval_secs {
            Some(v) => self
                .range(v, 1, 3600, "seconds", &path.key("health_interval_secs"))
                .map_or(Duration::from_secs(15), Duration::from_secs),
            None => Duration::from_secs(15),
        };

        let type_settings = types::parse_settings(
            kind,
            &settings,
            &forwards,
            path,
            self,
            env,
            &COMMON_TUNNEL_KEYS,
        )?;

        Some(TunnelConfig {
            name: raw.name,
            kind,
            profile,
            groups,
            forwards,
            auto_start: raw.auto_start.unwrap_or(false),
            auto_restart: raw.auto_restart.unwrap_or(true),
            max_retries,
            health_interval,
            type_settings,
        })
    }

    fn profile_ref(
        &mut self,
        profile: Option<String>,
        path: &FieldPath,
        profiles: &[Profile],
    ) -> Option<String> {
        let name = profile?;
        if profiles.iter().any(|p| p.name == name) {
            return Some(name);
        }
        let known: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        let list = if known.is_empty() {
            "no profiles are defined; add a [[profiles]] entry".to_owned()
        } else {
            format!("defined profiles: {}", known.join(", "))
        };
        self.err(
            &path.key("profile"),
            format!("no profile named \"{name}\"; {list}"),
        );
        None
    }

    fn forward(&mut self, raw: RawForward, path: &FieldPath) -> Option<Forward> {
        let bind = match raw.bind {
            None => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some(text) => match text.parse::<IpAddr>() {
                Ok(ip) => ip,
                Err(_) => {
                    self.err(
                        &path.key("bind"),
                        format!("\"{text}\" is not an IP address; use one such as 127.0.0.1 (the default) or ::1"),
                    );
                    return None;
                }
            },
        };
        let local_port = match raw.local_port {
            Some(v) => self.port(v, &path.key("local_port"))?,
            None => {
                self.err(&path.key("local_port"), "missing `local_port`");
                return None;
            }
        };
        let remote_port = match raw.remote_port {
            Some(v) => Some(self.port(v, &path.key("remote_port"))?),
            None => None,
        };
        Some(Forward {
            bind,
            local_port,
            remote_host: raw.remote_host,
            remote_port,
            local_host: raw.local_host,
        })
    }

    /// Records a listen address and reports a conflict with an earlier one (FR-4).
    fn listen(
        &mut self,
        tunnel: &str,
        bind: IpAddr,
        port: u16,
        path: &FieldPath,
        listens: &mut Vec<Listen>,
    ) {
        let clash = listens.iter().find(|l| {
            l.port == port && (l.bind == bind || l.bind.is_unspecified() || bind.is_unspecified())
        });
        if let Some(other) = clash {
            let message = format!(
                "local address {} is already used by tunnel \"{}\" ({}); choose another local_port",
                addr(bind, port),
                other.tunnel,
                other.path
            );
            self.err(&path.key("local_port"), message);
            return;
        }
        listens.push(Listen {
            bind,
            port,
            tunnel: tunnel.to_owned(),
            path: path.clone(),
        });
    }

    fn name(&mut self, what: &str, name: &str, path: &FieldPath) {
        if let Err(problem) = check_name(name) {
            self.err(path, format!("{what} name \"{name}\" {problem}"));
        }
    }

    fn port(&mut self, value: i64, path: &FieldPath) -> Option<u16> {
        self.range(value, 1, 65535, "", path).map(|v| v as u16)
    }

    fn range(
        &mut self,
        value: i64,
        min: u64,
        max: u64,
        unit: &str,
        path: &FieldPath,
    ) -> Option<u64> {
        match u64::try_from(value) {
            Ok(v) if (min..=max).contains(&v) => Some(v),
            _ => {
                let unit = if unit.is_empty() {
                    String::new()
                } else {
                    format!(" {unit}")
                };
                self.err(
                    path,
                    format!("{value} is out of range; use {min}–{max}{unit}"),
                );
                None
            }
        }
    }

    fn abs_path(&mut self, value: &str, path: &FieldPath, env: &impl Env) -> Option<PathBuf> {
        match expand_tilde(Path::new(value), env) {
            Some(p) if p.is_absolute() => Some(p),
            Some(_) => {
                self.err(
                    path,
                    format!("\"{value}\" must be an absolute path (starting with / or ~)"),
                );
                None
            }
            None => {
                self.err(
                    path,
                    format!("cannot expand \"{value}\": home directory unknown"),
                );
                None
            }
        }
    }
}

impl Report for Ctx<'_> {
    fn error(&mut self, path: &FieldPath, message: String) {
        self.err(path, message);
    }
}

fn addr(bind: IpAddr, port: u16) -> String {
    match bind {
        IpAddr::V6(ip) => format!("[{ip}]:{port}"),
        IpAddr::V4(ip) => format!("{ip}:{port}"),
    }
}

/// Names become file names, so they are restricted (FR-4).
fn check_name(name: &str) -> Result<(), String> {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    if name.is_empty() {
        return Err("is empty".to_owned());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("is longer than {MAX_NAME_LEN} characters"));
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(format!(
            "contains '{c}'; use only letters, digits, '-', '_' and '.'"
        ));
    }
    if !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err("must start with a letter or digit".to_owned());
    }
    let base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let is_port_device = (base.starts_with("COM") || base.starts_with("LPT"))
        && base.len() == 4
        && base.as_bytes()[3].is_ascii_digit()
        && base.as_bytes()[3] != b'0';
    if RESERVED.contains(&base.as_str()) || is_port_device {
        return Err(format!(
            "is reserved on Windows (\"{base}\"); choose another name"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
