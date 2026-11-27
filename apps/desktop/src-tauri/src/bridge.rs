//! Commands the web view calls, and the watcher that relays daemon events
//! (docs/desktop.md D2, D3). Every command opens its own short connection; the
//! watcher keeps one subscription open and reconnects when it drops.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tunwarden_core::client::{Client, ClientError, Launcher};
use tunwarden_core::config::{self, LoadError};
use tunwarden_core::paths::{self, Os, SystemEnv};
use tunwarden_core::proto::*;

use crate::tray;

/// The example config `init` writes, shared with the CLI.
const EXAMPLE_CONFIG: &str = include_str!("../../../../crates/tunwarden-cli/example-config.toml");

pub struct Bridge {
    pub config_path: PathBuf,
    pub state_dir: PathBuf,
    /// The `tunwarden` CLI shipped next to the app, which runs the daemon.
    pub cli: PathBuf,
    /// The last status, for the tray.
    pub last: Mutex<Option<StatusResult>>,
    /// Whether the system uses a dark appearance, for the tray icon's ink.
    pub dark: std::sync::atomic::AtomicBool,
}

/// An error for the web view: what happened and, if known, what to do.
#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
    /// `not_running`, `protocol_mismatch`, `start_failed` or `other`.
    pub kind: &'static str,
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Failure {
        let kind = match &e {
            ClientError::NotRunning => "not_running",
            ClientError::Start(_) => "start_failed",
            e if e.is_protocol_mismatch() => "protocol_mismatch",
            _ => "other",
        };
        Failure {
            hint: e.hint().map(str::to_owned),
            message: e.to_string(),
            kind,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Failure {
        Failure {
            message,
            hint: None,
            kind: "other",
        }
    }
}

type Res<T> = Result<T, Failure>;

impl Bridge {
    pub fn new() -> Result<Bridge, String> {
        let os = Os::current();
        let config_path = paths::config_file(None, &SystemEnv, os).map_err(|e| e.to_string())?;
        let state_dir = paths::state_dir(&SystemEnv, os).map_err(|e| e.to_string())?;
        Ok(Bridge {
            config_path,
            state_dir,
            cli: find_cli(),
            last: Mutex::new(None),
            dark: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn launcher(&self) -> Launcher {
        Launcher {
            program: self.cli.clone(),
            args: vec![
                "--config".into(),
                self.config_path.clone().into_os_string(),
                "daemon".into(),
                "run".into(),
            ],
        }
    }

    /// Connects, starting the daemon through the CLI if none runs.
    async fn client(&self) -> Res<Client> {
        Ok(Client::connect_or_start(&self.state_dir, "desktop", &self.launcher()).await?)
    }
}

/// `tunwarden` next to this executable (where Tauri puts bundled binaries and
/// where cargo builds it in development), else on PATH.
fn find_cli() -> PathBuf {
    let name = if cfg!(windows) {
        "tunwarden.exe"
    } else {
        "tunwarden"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .or_else(|| which::which("tunwarden").ok())
        .unwrap_or_else(|| PathBuf::from(name))
}

// --- Commands -------------------------------------------------------------------

#[derive(Serialize)]
pub struct ConfigInfo {
    pub path: String,
    pub exists: bool,
    /// Problems in the file, one line each.
    pub errors: Vec<String>,
    pub tunnels: usize,
}

#[tauri::command]
pub fn config_info(bridge: tauri::State<'_, Bridge>) -> ConfigInfo {
    let path = &bridge.config_path;
    let (exists, errors, tunnels) = match config::load(path) {
        Ok(c) => (true, Vec::new(), c.tunnels.len()),
        Err(LoadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            (false, Vec::new(), 0)
        }
        Err(LoadError::Invalid(e)) => (true, e.0.iter().map(|x| x.to_string()).collect(), 0),
        Err(e) => (true, vec![e.to_string()], 0),
    };
    ConfigInfo {
        path: path.display().to_string(),
        exists,
        errors,
        tunnels,
    }
}

/// Writes the example config, like `tunwarden init`. Never overwrites.
#[tauri::command]
pub fn init_example(bridge: tauri::State<'_, Bridge>) -> Res<String> {
    let path = &bridge.config_path;
    if path.exists() {
        return Err(format!("{} already exists", path.display()).into());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    tunwarden_core::fsutil::write_atomic(path, EXAMPLE_CONFIG.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// Opens the config file in the default editor.
#[tauri::command]
pub fn open_config(bridge: tauri::State<'_, Bridge>) -> Res<()> {
    let path = bridge.config_path.as_os_str();
    let result = match Os::current() {
        Os::MacOs => std::process::Command::new("open")
            .arg("-t")
            .arg(path)
            .spawn(),
        Os::Linux => std::process::Command::new("xdg-open").arg(path).spawn(),
        Os::Windows => std::process::Command::new("notepad").arg(path).spawn(),
    };
    result
        .map(|_| ())
        .map_err(|e| format!("cannot open the config file: {e}").into())
}

#[derive(Serialize)]
pub struct ImportEntry {
    pub name: String,
    pub kind: String,
    pub profile: Option<String>,
    /// The command or SSH host, for the list.
    pub summary: String,
    pub local: Vec<u16>,
}

#[derive(Serialize)]
pub struct ImportPreview {
    pub tunnels: Vec<ImportEntry>,
    pub profiles: Vec<String>,
    pub warnings: Vec<String>,
}

fn current_config(bridge: &Bridge) -> Res<config::Config> {
    match config::load(&bridge.config_path) {
        Ok(c) => Ok(c),
        Err(LoadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            config::parse(config::edit::NEW_FILE, &bridge.config_path, &SystemEnv)
                .map_err(|e| e.to_string().into())
        }
        Err(e) => Err(e.to_string().into()),
    }
}

/// Turns pasted commands or shell aliases into entries, like `tunwarden import`.
#[tauri::command]
pub fn import_preview(bridge: tauri::State<'_, Bridge>, text: String) -> Res<ImportPreview> {
    let config = current_config(&bridge)?;
    let imported = tunwarden_core::import::import(&text, &config);
    let str_of =
        |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(ImportPreview {
        tunnels: imported
            .tunnels
            .iter()
            .map(|t| ImportEntry {
                name: t.name.clone(),
                kind: str_of(&t.fields, "type").unwrap_or_else(|| "command".into()),
                profile: str_of(&t.fields, "profile"),
                summary: str_of(&t.fields, "command")
                    .or_else(|| str_of(&t.fields, "host"))
                    .unwrap_or_default(),
                local: t
                    .fields
                    .get("forwards")
                    .and_then(Value::as_array)
                    .map(|fs| {
                        fs.iter()
                            .filter_map(|f| f.get("local_port").and_then(Value::as_u64))
                            .filter_map(|p| u16::try_from(p).ok())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        profiles: imported.profiles.iter().map(|p| p.name.clone()).collect(),
        warnings: imported.warnings,
    })
}

/// Adds the chosen entries of an import, with the profiles they use.
#[tauri::command]
pub async fn import_apply(
    bridge: tauri::State<'_, Bridge>,
    text: String,
    names: Vec<String>,
) -> Res<Vec<String>> {
    let config = current_config(&bridge)?;
    if !bridge.config_path.exists() {
        tunwarden_core::fsutil::write_atomic(
            &bridge.config_path,
            config::edit::NEW_FILE.as_bytes(),
        )
        .map_err(|e| format!("cannot create {}: {e}", bridge.config_path.display()))?;
    }
    let imported = tunwarden_core::import::import(&text, &config);
    let tunnels: Vec<_> = imported
        .tunnels
        .iter()
        .filter(|t| names.contains(&t.name))
        .collect();
    let used: Vec<&str> = tunnels
        .iter()
        .filter_map(|t| t.fields.get("profile").and_then(Value::as_str))
        .collect();
    let mut c = bridge.client().await?;
    for p in imported
        .profiles
        .iter()
        .filter(|p| used.contains(&p.name.as_str()))
    {
        let _: ConfigEditResult = c
            .call(
                method::CONFIG_ADD_ENTRY,
                ConfigAddEntryParams {
                    table: "profiles".into(),
                    name: p.name.clone(),
                    fields: p.fields.clone(),
                },
            )
            .await?;
    }
    let mut added = Vec::new();
    for t in tunnels {
        let _: ConfigEditResult = c
            .call(
                method::CONFIG_ADD,
                ConfigAddParams {
                    name: t.name.clone(),
                    fields: t.fields.clone(),
                },
            )
            .await?;
        added.push(t.name.clone());
    }
    Ok(added)
}

#[tauri::command]
pub async fn status(bridge: tauri::State<'_, Bridge>) -> Res<StatusResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::STATUS, StatusParams { names: Vec::new() })
        .await?)
}

#[tauri::command]
pub async fn list(bridge: tauri::State<'_, Bridge>) -> Res<ListResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::LIST, json!({})).await?)
}

/// `start`, `stop` or `restart`.
#[tauri::command]
pub async fn act(
    bridge: tauri::State<'_, Bridge>,
    action: String,
    selector: Selector,
) -> Res<ActionResult> {
    let rpc = match action.as_str() {
        "start" => method::START,
        "stop" => method::STOP,
        "restart" => method::RESTART,
        other => return Err(format!("unknown action {other}").into()),
    };
    let mut c = bridge.client().await?;
    Ok(c.call(rpc, selector).await?)
}

#[tauri::command]
pub async fn logs_tail(
    bridge: tauri::State<'_, Bridge>,
    name: String,
    lines: usize,
) -> Res<LogsResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::LOGS_TAIL, LogsParams { name, lines })
        .await?)
}

#[tauri::command]
pub async fn auth_status(bridge: tauri::State<'_, Bridge>) -> Res<AuthResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::AUTH_STATUS, AuthParams::default()).await?)
}

/// Runs every login's check now; the daemon acts on the results (FR-22).
#[tauri::command]
pub async fn auth_check(bridge: tauri::State<'_, Bridge>) -> Res<AuthResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::AUTH_CHECK, AuthParams::default()).await?)
}

/// Stops the daemon keeping its tunnels for restore, then starts it again
/// (after a version mismatch, FR-53).
#[tauri::command]
pub async fn restart_daemon(bridge: tauri::State<'_, Bridge>) -> Res<()> {
    if let Ok(mut c) = Client::connect(&bridge.state_dir, "desktop").await {
        let _: Result<Value, _> = c
            .call(method::SHUTDOWN, ShutdownParams { keep: true })
            .await;
    }
    for _ in 0..100 {
        if matches!(
            Client::connect(&bridge.state_dir, "desktop").await,
            Err(ClientError::NotRunning)
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bridge.client().await.map(|_| ())
}

/// Quits the app. With `stop_tunnels`, stops every tunnel first (FR-10).
#[tauri::command]
pub async fn quit<R: Runtime>(
    app: AppHandle<R>,
    bridge: tauri::State<'_, Bridge>,
    stop_tunnels: bool,
) -> Res<()> {
    if stop_tunnels && let Ok(mut c) = Client::connect(&bridge.state_dir, "desktop").await {
        let _: ActionResult = c
            .call(
                method::STOP,
                Selector {
                    all: true,
                    ..Selector::default()
                },
            )
            .await?;
    }
    app.exit(0);
    Ok(())
}

// --- Watcher --------------------------------------------------------------------

/// What the web view hears, on the `daemon` event.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonEvent {
    Connected {
        status: StatusResult,
        version: String,
    },
    Disconnected {
        failure: Failure,
    },
    Tunnel {
        view: TunnelView,
    },
    Activity {
        event: Value,
    },
}

/// Keeps a subscription open: sends the full status on connect, then every
/// change; on a lost connection, says so and retries every 2 seconds.
pub async fn watch<R: Runtime>(app: AppHandle<R>) {
    loop {
        let bridge = app.state::<Bridge>();
        match subscribe(&app, &bridge).await {
            Ok(()) => {}
            Err(failure) => {
                *bridge.last.lock().unwrap_or_else(|e| e.into_inner()) = None;
                update_tray(&app, None);
                let _ = app.emit("daemon", DaemonEvent::Disconnected { failure });
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn subscribe<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge) -> Res<()> {
    let mut c = bridge.client().await?;
    let version = c.hello().daemon_version.clone();
    let _: Value = c
        .call(
            method::SUBSCRIBE,
            SubscribeParams {
                topics: vec!["tunnels".into(), "activity".into()],
            },
        )
        .await?;
    let status: StatusResult = c
        .call(method::STATUS, StatusParams { names: Vec::new() })
        .await?;
    remember(app, bridge, status.clone());
    let _ = app.emit("daemon", DaemonEvent::Connected { status, version });
    loop {
        let n = c.next_notification().await?;
        match n.method.as_str() {
            notice::TUNNEL_STATE => {
                let Ok(view) = serde_json::from_value::<TunnelView>(n.params) else {
                    continue;
                };
                let next = {
                    let last = bridge.last.lock().unwrap_or_else(|e| e.into_inner());
                    last.clone().map(|mut s| {
                        match s.tunnels.iter_mut().find(|t| t.name == view.name) {
                            Some(t) => *t = view.clone(),
                            None => s.tunnels.push(view.clone()),
                        }
                        s
                    })
                };
                if let Some(s) = next {
                    remember(app, bridge, s);
                }
                let _ = app.emit("daemon", DaemonEvent::Tunnel { view });
            }
            notice::ACTIVITY => {
                let _ = app.emit("daemon", DaemonEvent::Activity { event: n.params });
            }
            _ => {}
        }
    }
}

fn remember<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, status: StatusResult) {
    update_tray(app, Some(&status));
    *bridge.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
}

pub fn update_tray<R: Runtime>(app: &AppHandle<R>, status: Option<&StatusResult>) {
    let Some(t) = app.tray_by_id(tray::TRAY_ID) else {
        return;
    };
    let dark = app
        .state::<Bridge>()
        .dark
        .load(std::sync::atomic::Ordering::Relaxed);
    let _ = t.set_icon(Some(tray::icon(tray::health(status), dark)));
    let _ = t.set_tooltip(Some(format!("Tunwarden: {}", tray::summary(status))));
    if let Ok(menu) = tray::menu(app, status) {
        let _ = t.set_menu(Some(menu));
    }
}

// --- M3b: editing, doctor, activity, autostart, login ---------------------------

/// The config's entries as written in the file, for the edit forms.
#[derive(Serialize, Default)]
pub struct ConfigEntries {
    pub tunnels: Vec<Map<String, Value>>,
    pub profiles: Vec<Map<String, Value>>,
    pub logins: Vec<Map<String, Value>>,
}

#[tauri::command]
pub fn config_entries(bridge: tauri::State<'_, Bridge>) -> Res<ConfigEntries> {
    let text = match std::fs::read_to_string(&bridge.config_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigEntries::default()),
        Err(e) => return Err(format!("cannot read {}: {e}", bridge.config_path.display()).into()),
    };
    let doc: toml::Table = toml::from_str(&text).map_err(|e| e.message().to_owned())?;
    let entries = |key: &str| -> Vec<Map<String, Value>> {
        doc.get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| serde_json::to_value(i).ok())
                    .filter_map(|v| v.as_object().cloned())
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ConfigEntries {
        tunnels: entries("tunnels"),
        profiles: entries("profiles"),
        logins: entries("logins"),
    })
}

/// The `set` and `unset` that turn `old` into `new`, ignoring `name`.
fn diff(old: &Map<String, Value>, new: &Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    let set = new
        .iter()
        .filter(|(k, v)| k.as_str() != "name" && old.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let unset = old
        .keys()
        .filter(|k| k.as_str() != "name" && !new.contains_key(*k))
        .cloned()
        .collect();
    (set, unset)
}

/// Adds or changes an entry. `table` is `tunnels`, `profiles` or `logins`;
/// `old` is the entry as it was, or null for a new one.
#[tauri::command]
pub async fn save_entry(
    bridge: tauri::State<'_, Bridge>,
    table: String,
    name: String,
    old: Option<Map<String, Value>>,
    fields: Map<String, Value>,
) -> Res<ConfigEditResult> {
    if !bridge.config_path.exists() {
        tunwarden_core::fsutil::write_atomic(
            &bridge.config_path,
            config::edit::NEW_FILE.as_bytes(),
        )
        .map_err(|e| format!("cannot create {}: {e}", bridge.config_path.display()))?;
    }
    let mut c = bridge.client().await?;
    let mut fields = fields;
    fields.remove("name");
    Ok(match (table.as_str(), old) {
        ("tunnels", None) => {
            c.call(method::CONFIG_ADD, ConfigAddParams { name, fields })
                .await?
        }
        ("tunnels", Some(old)) => {
            let (set, unset) = diff(&old, &fields);
            c.call(method::CONFIG_EDIT, ConfigEditParams { name, set, unset })
                .await?
        }
        (_, None) => {
            c.call(
                method::CONFIG_ADD_ENTRY,
                ConfigAddEntryParams {
                    table,
                    name,
                    fields,
                },
            )
            .await?
        }
        (_, Some(old)) => {
            let (set, unset) = diff(&old, &fields);
            c.call(
                method::CONFIG_EDIT_ENTRY,
                ConfigEditEntryParams {
                    table,
                    name,
                    set,
                    unset,
                },
            )
            .await?
        }
    })
}

#[tauri::command]
pub async fn remove_entry(
    bridge: tauri::State<'_, Bridge>,
    table: String,
    name: String,
) -> Res<ConfigEditResult> {
    let mut c = bridge.client().await?;
    Ok(if table == "tunnels" {
        c.call(method::CONFIG_REMOVE, ConfigRemoveParams { name })
            .await?
    } else {
        c.call(
            method::CONFIG_REMOVE_ENTRY,
            ConfigRemoveEntryParams { table, name },
        )
        .await?
    })
}

#[tauri::command]
pub async fn doctor(bridge: tauri::State<'_, Bridge>) -> Res<tunwarden_core::doctor::Report> {
    Ok(tunwarden_core::doctor::run(
        &bridge.config_path,
        &bridge.state_dir,
        "desktop",
        Some(&bridge.cli),
    )
    .await)
}

#[tauri::command]
pub async fn activity(bridge: tauri::State<'_, Bridge>, limit: usize) -> Res<ActivityResult> {
    let mut c = bridge.client().await?;
    Ok(c.call(method::ACTIVITY, ActivityParams { limit }).await?)
}

#[derive(Serialize)]
pub struct AutostartInfo {
    pub enabled: bool,
    pub location: String,
    pub problem: Option<String>,
    /// The entry starts a different tunwarden than the one this app ships.
    pub other_binary: Option<String>,
}

#[tauri::command]
pub fn autostart_status(bridge: tauri::State<'_, Bridge>) -> Res<AutostartInfo> {
    let s = tunwarden_core::autostart::status(&SystemEnv).map_err(|e| e.to_string())?;
    let app_exe = std::env::current_exe().ok();
    let other_binary = s
        .exe
        .as_ref()
        .filter(|exe| **exe != bridge.cli && Some(*exe) != app_exe.as_ref())
        .map(|exe| exe.display().to_string());
    Ok(AutostartInfo {
        enabled: s.enabled,
        location: s.location,
        problem: s.problem,
        other_binary,
    })
}

/// Turns starting the daemon at login on or off, with the CLI next to the app.
#[tauri::command]
pub fn autostart_set(bridge: tauri::State<'_, Bridge>, enabled: bool) -> Res<AutostartInfo> {
    use tunwarden_core::autostart;
    if enabled {
        // On Windows the entry starts this app hidden in the tray (D12); on
        // macOS and Linux, launchd or systemd runs the CLI's daemon directly.
        let windows = Os::current() == Os::Windows;
        let app_exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let entry = autostart::Entry {
            exe: if windows { app_exe } else { bridge.cli.clone() },
            config: bridge.config_path.clone(),
            state_dir_override: std::env::var_os(paths::STATE_DIR_ENV)
                .filter(|v| !v.is_empty())
                .map(|_| bridge.state_dir.clone()),
            log: bridge.state_dir.join("daemon.log"),
            // The app has no shell PATH of its own; the CLI's tool lookup
            // covers the usual folders and `tools.path` (FR-36).
            path: std::env::var_os("PATH").filter(|_| !windows),
            app: windows,
        };
        autostart::enable(&entry, &SystemEnv).map_err(|e| e.to_string())?;
    } else {
        autostart::disable(&SystemEnv).map_err(|e| e.to_string())?;
    }
    autostart_status(bridge)
}

#[derive(Clone, Serialize)]
pub struct LoginLine {
    pub login: String,
    pub line: String,
}

#[derive(Serialize)]
pub struct LoginDone {
    pub login: String,
    pub restarted: Vec<String>,
}

/// Runs a login's `login` command as a child of the app, sending its output
/// as `login-output` events, then lets the daemon check it and restart the
/// tunnels waiting for it (FR-25). Tools that open a browser work like this;
/// tools that ask questions need `login_in_terminal`.
#[tauri::command]
pub async fn login_run<R: Runtime>(
    app: AppHandle<R>,
    bridge: tauri::State<'_, Bridge>,
    name: String,
) -> Res<LoginDone> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let config = config::load(&bridge.config_path).map_err(|e| e.to_string())?;
    let login = tunwarden_core::auth::find(&config, &name).map_err(|e| Failure {
        message: e.message,
        hint: e.hint,
        kind: "other",
    })?;
    let Some(words) = &login.login else {
        return Err(Failure {
            message: format!("the {} login has no `login` command", login.name),
            hint: Some("log in with your tool, then check it again in Logins".to_owned()),
            kind: "other",
        });
    };
    let program = tunwarden_core::tools::resolve(
        &tunwarden_core::types::Program::Command(words[0].clone()),
        &config.tools,
        &SystemEnv,
        Os::current(),
    )
    .map_err(|e| e.to_string())?
    .path;
    let std_cmd = tunwarden_core::auth::login_command(login, &program).expect("checked above");
    let mut cmd = tokio::process::Command::from(std_cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    let pipe = |r: Box<dyn tokio::io::AsyncRead + Unpin + Send>| {
        let app = app.clone();
        let login = login.name.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "login-output",
                    LoginLine {
                        login: login.clone(),
                        line,
                    },
                );
            }
        })
    };
    let out = pipe(Box::new(child.stdout.take().expect("piped")));
    let err = pipe(Box::new(child.stderr.take().expect("piped")));
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let _ = (out.await, err.await);
    if !status.success() {
        return Err(Failure {
            message: format!("the login command did not finish ({status})"),
            hint: Some("if it asks questions, use Run in Terminal".into()),
            kind: "other",
        });
    }
    let mut c = bridge.client().await?;
    let r: RecoveredResult = c
        .call(
            method::AUTH_RECOVERED,
            RecoveredParams {
                login: login.name.clone(),
            },
        )
        .await?;
    Ok(LoginDone {
        login: login.name.clone(),
        restarted: r.restarted,
    })
}

/// Opens a terminal window running `tunwarden login NAME`, for login tools
/// that ask questions.
#[tauri::command]
pub fn login_in_terminal(bridge: tauri::State<'_, Bridge>, name: String) -> Res<()> {
    let q = |s: &str| tunwarden_core::types::quote(s);
    let line = format!(
        "{} --config {} login {}",
        q(&bridge.cli.display().to_string()),
        q(&bridge.config_path.display().to_string()),
        q(&name)
    );
    let result = match Os::current() {
        Os::MacOs => {
            let script = format!(
                "tell application \"Terminal\" to do script \"{}\"\ntell application \"Terminal\" to activate",
                line.replace('\\', "\\\\").replace('"', "\\\"")
            );
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .spawn()
        }
        Os::Linux => std::process::Command::new("x-terminal-emulator")
            .args(["-e", "sh", "-c"])
            .arg(format!("{line}; echo; read -r -p 'Press Enter to close' _"))
            .spawn(),
        Os::Windows => std::process::Command::new("cmd")
            .args(["/c", "start", "Tunwarden login", "cmd", "/k"])
            .arg(&bridge.cli)
            .arg("--config")
            .arg(&bridge.config_path)
            .args(["login", &name])
            .spawn(),
    };
    result.map(|_| ()).map_err(|e| {
        format!("cannot open a terminal: {e}; run `tunwarden login {name}` yourself").into()
    })
}
