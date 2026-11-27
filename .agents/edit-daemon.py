import sys

root = "/Users/omjannu/Projects/tunwarden/crates/tunwarden-core/src/"

def edit(p, pairs):
    s = open(root + p).read()
    for a, b in pairs:
        if a not in s:
            sys.exit(f"not found in {p}: {a[:90]!r}")
        s = s.replace(a, b, 1)
    open(root + p, "w").write(s)

def cut(p, start, end, new):
    s = open(root + p).read()
    i = s.index(start)
    j = s.index(end, i)
    open(root + p, "w").write(s[:i] + new + s[j:])

# ---------------------------------------------------------------- proto.rs
edit("proto.rs", [
("""    pub const CONFIG_ADD_PROJECT: &str = "config.add_project";""",
 """    pub const CONFIG_ADD_ENTRY: &str = "config.add_entry";"""),
("""    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub all: bool,""", """    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub all: bool,"""),
])
s = open(root + "proto.rs").read()
s = s.replace("""    pub kind: String,
    pub project: Option<String>,
    pub production: bool,""", """    pub kind: String,
    pub profile: Option<String>,
    pub production: bool,""")
if s.count("pub profile: Option<String>,\n    pub production: bool,") != 2:
    sys.exit("views not both renamed")
i = s.index("#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]\npub struct AuthParams")
j = s.index("/// Adds a tunnel.")
s = s[:i] + '''#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthParams {
    /// A login name, or a profile name meaning that profile's login. All
    /// logins if empty.
    #[serde(default)]
    pub name: Option<String>,
}

/// One login and the profiles that use it (FR-22, FR-23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginView {
    pub name: String,
    pub profiles: Vec<String>,
    /// Whether the login has a `check` command.
    pub checkable: bool,
    /// `ok`, `expired`, `unknown`, or `not checked`.
    pub result: String,
    pub detail: Option<String>,
    pub checked_at: Option<String>,
    /// Tunnels using this login that are active now.
    pub active_tunnels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResult {
    pub logins: Vec<LoginView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredParams {
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredResult {
    /// Tunnels that failed because of the expired login and are starting again.
    pub restarted: Vec<String>,
}

/// Adds a `[[logins]]` or `[[profiles]]` entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigAddEntryParams {
    /// `logins` or `profiles`.
    pub table: String,
    pub name: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
}

''' + s[j:]
open(root + "proto.rs", "w").write(s)

# ---------------------------------------------------------------- daemon.rs
d = "daemon.rs"
edit(d, [
("use crate::auth::{self, ContextKey, Outcome};", "use crate::auth::{self, Outcome};"),
("""    /// The login this tunnel's project uses, if any.
    context: Option<ContextKey>,""", """    /// The login this tunnel's profile uses, if any.
    login: Option<String>,"""),
("    auth: tokio::sync::Mutex<BTreeMap<ContextKey, AuthState>>,", "    auth: tokio::sync::Mutex<BTreeMap<String, AuthState>>,"),
("""struct AuthState {
    projects: Vec<String>,""", """struct AuthState {
    profiles: Vec<String>,"""),
])
cut(d, "    for t in &config.tunnels {\n        let project = t\n", "        let was_running = previous", """    for t in &config.tunnels {
        let mut entry = new_entry(t, &config, &prober, &hub);
""")
edit(d, [
("""        if restore || t.auto_start {
            supervisor.send(Event::Start);
        }
        tunnels.insert(
            t.name.clone(),
            Entry {
                config: t.clone(),
                production: project.is_some_and(|p| p.production),
                context: project.map(ContextKey::of),
                supervisor,
                note,
            },
        );""", """        if restore || t.auto_start {
            entry.supervisor.send(Event::Start);
        }
        entry.note = note;
        tunnels.insert(t.name.clone(), entry);"""),
])
cut(d, "    let auth_states = auth::contexts(&config)", "    let ctx = Arc::new(Ctx {", """    let auth_states = login_states(&config, &mut BTreeMap::new());
""")
edit(d, [
("""        .unwrap_or(config.settings.auth_check_interval);""", """        .unwrap_or(config.settings.login_check_interval);"""),
])
cut(d, "    let empty = || Config {", "    match config::load(path) {", """    let empty = Config::empty;
""")
# handle(): auth methods
cut(d, "        method::AUTH_STATUS => {", "        method::HELLO => {", """        method::AUTH_STATUS => {
            let p: AuthParams = params(req)?;
            let names = login_names(ctx, p.name.as_deref()).await?;
            to_value(AuthResult {
                logins: login_views(ctx, &names).await,
            })
        }
        method::AUTH_CHECK => {
            let p: AuthParams = params(req)?;
            let names = login_names(ctx, p.name.as_deref()).await?;
            for name in &names {
                check_login(ctx, name).await;
            }
            to_value(AuthResult {
                logins: login_views(ctx, &names).await,
            })
        }
        method::AUTH_RECOVERED => {
            let p: RecoveredParams = params(req)?;
            let names = login_names(ctx, Some(&p.login)).await?;
            let name = &names[0];
            let outcome = check_login(ctx, name).await;
            if !matches!(outcome, Outcome::Ok) {
                return Err(RpcError::app(
                    "still_expired",
                    format!(
                        "the {name} login still fails its check: {}",
                        outcome.detail().unwrap_or_default()
                    ),
                    Some(format!("run `tunwarden login {name}` again")),
                ));
            }
            let tunnels = ctx.tunnels();
            let mut restarted = Vec::new();
            for e in tunnels.values().filter(|e| e.login.as_ref() == Some(name)) {
                let s = e.supervisor.status();
                if s.state == State::Failed && s.reason == Some(FailureReason::AuthExpired) {
                    restarted.push(e.config.name.clone());
                }
                e.supervisor.send(Event::AuthRecovered);
            }
            to_value(RecoveredResult { restarted })
        }
""")
edit(d, [
("""        method::CONFIG_ADD_PROJECT => {
            let p: ConfigAddParams = params(req)?;
            edit_config(ctx, EditOp::AddProject(p.name, p.fields)).await
        }""", """        method::CONFIG_ADD_ENTRY => {
            let p: ConfigAddEntryParams = params(req)?;
            if !matches!(p.table.as_str(), "logins" | "profiles") {
                return Err(RpcError::app("invalid_edit", format!("cannot add to `{}`; use logins or profiles", p.table), None));
            }
            edit_config(ctx, EditOp::AddEntry(p.table, p.name, p.fields)).await
        }"""),
])
cut(d, "/// The entry for one tunnel of `config`, with a stopped supervisor.", "/// Switches the daemon to `new` (design 3.4).", """/// The entry for one tunnel of `config`, with a stopped supervisor.
fn new_entry(t: &TunnelConfig, config: &Config, prober: &Arc<Prober>, hub: &Arc<Hub>) -> Entry {
    let (spec, production, login) = new_entry_parts(t, config);
    Entry {
        config: t.clone(),
        production,
        login,
        supervisor: Supervisor::spawn(spec, prober.clone(), hub.clone()),
        note: None,
    }
}

/// Whether anything that affects how `t` runs differs between two configs:
/// its own fields, its profile, its profile's login, or the tool paths.
fn tunnel_changed(t: &TunnelConfig, old_cfg: &Config, old: &TunnelConfig, new_cfg: &Config) -> bool {
    let context = |c: &Config, t: &TunnelConfig| {
        let (p, l) = c.context_of(t);
        (p.cloned(), l.cloned())
    };
    old != t || context(old_cfg, old) != context(new_cfg, t) || old_cfg.tools != new_cfg.tools
}

/// The login states for `config`, keeping known results for logins that
/// still exist.
fn login_states(config: &Config, old: &mut BTreeMap<String, AuthState>) -> BTreeMap<String, AuthState> {
    config
        .logins
        .iter()
        .map(|l| {
            let profiles = config
                .profiles
                .iter()
                .filter(|p| p.login.as_ref() == Some(&l.name))
                .map(|p| p.name.clone())
                .collect();
            let last = old.remove(&l.name).and_then(|s| s.last);
            (l.name.clone(), AuthState { profiles, last })
        })
        .collect()
}

""")
edit(d, [
("""                    e.production = fresh.1;
                    e.context = fresh.2;""", """                    e.production = fresh.1;
                    e.login = fresh.2;"""),
])
cut(d, "    // Logins: keep what is known about logins that still exist.", "    for e in removed {", """    // Logins: keep what is known about logins that still exist.
    {
        let mut auth_states = ctx.auth.lock().await;
        let fresh = login_states(&new, &mut auth_states);
        *auth_states = fresh;
    }

""")
cut(d, "/// The spec, production flag and login of `t` in `config`.", "/// Reloads after a hand edit.", """/// The spec, production flag and login name of `t` in `config`.
fn new_entry_parts(t: &TunnelConfig, config: &Config) -> (Spec, bool, Option<String>) {
    let (profile, login) = config.context_of(t);
    (
        Spec::new(t, config),
        profile.is_some_and(|p| p.production),
        login.map(|l| l.name.clone()),
    )
}

""")
edit(d, [
("    AddProject(String, serde_json::Map<String, Value>),", "    AddEntry(String, String, serde_json::Map<String, Value>),"),
("""        EditOp::AddProject(name, fields) => {
            edit::add_project(&mut doc, name, fields).map_err(edit_error)?;""", """        EditOp::AddEntry(table, name, fields) => {
            edit::add_entry(&mut doc, table, name, fields).map_err(edit_error)?;"""),
("        EditError::Exists(_) => \"tunnel_exists\",", "        EditError::Exists(..) => \"already_exists\","),
])
cut(d, "// --- gcloud logins (FR-22 to FR-25)", "fn check_name(tunnels: &BTreeMap<String, Entry>, name: &str)", open("/Users/omjannu/Projects/tunwarden/.agents/daemon-logins.rs").read())
edit(d, [
("""    if s.names.is_empty() && s.group.is_none() && s.project.is_none() {
        return Err(RpcError::app(
            "no_selection",
            "no tunnels selected",
            Some("name one or more tunnels, or use --all, --group or --project".into()),""", """    if s.names.is_empty() && s.group.is_none() && s.profile.is_none() {
        return Err(RpcError::app(
            "no_selection",
            "no tunnels selected",
            Some("name one or more tunnels, or use --all, --group or --profile".into()),"""),
])
cut(d, "    if let Some(project) = &s.project {", "    let mut seen = std::collections::HashSet::new();", """    if let Some(profile) = &s.profile {
        if ctx.config().profile(profile).is_none() {
            return Err(RpcError::app(
                "unknown_profile",
                format!("no profile named \\"{profile}\\""),
                None,
            ));
        }
        names.extend(
            tunnels
                .values()
                .filter(|e| e.config.profile.as_ref() == Some(profile))
                .map(|e| e.config.name.clone()),
        );
    }
""")
cut(d, "pub fn tunnel_info(config: &Config, t: &TunnelConfig) -> TunnelInfo {", "fn local(t: &TunnelConfig)", """pub fn tunnel_info(config: &Config, t: &TunnelConfig) -> TunnelInfo {
    let (profile, _) = config.context_of(t);
    TunnelInfo {
        name: t.name.clone(),
        kind: t.kind.id().into(),
        profile: t.profile.clone(),
        production: profile.is_some_and(|p| p.production),
        groups: t.groups.clone(),
        local: local(t),
        command: t.launch_plan(profile).display(),
    }
}

""")
edit(d, [
("        project: e.config.project.clone(),\n        production: e.production,", "        profile: e.config.profile.clone(),\n        production: e.production,"),
])
print("daemon ok")
