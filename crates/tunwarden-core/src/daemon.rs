//! The daemon: the single owner of all tunnels (PRD FR-37, FR-38, design 5).
//!
//! Startup order:
//! 1. Take the exclusive lock `<state>/daemon.lock`; a second daemon exits.
//! 2. Load the config. An invalid or missing file is reported, not fatal.
//! 3. Read `state.json` and sweep processes a crashed daemon left (FR-52).
//! 4. Create one supervisor per tunnel, then restore and auto-start (FR-15).
//! 5. Serve the control endpoint until asked to shut down.

use std::collections::BTreeMap;
use std::fs::{OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::auth::{self, Outcome};
use crate::config::{self, Config, LoadError, TunnelConfig};
use crate::failure::FailureReason;
use crate::fsutil::{format_timestamp_now, private_dir};
use crate::ipc::{self, Listener, Stream};
use crate::logs::{Activity, ActivityEvent, DEFAULT_MAX_BYTES, Logs, format_line};
use crate::persist::Persister;
use crate::probe::Prober;
use crate::process::{self, DEFAULT_STOP_GRACE, SweepOutcome};
use crate::proto::{self, method, notice, *};
use crate::state::{DaemonInfo, Desired, StateError, StateStore, now};
use crate::supervisor::{Event, LogSource, Observer, Spec, State, Status, Supervisor};

pub struct Options {
    pub config_path: PathBuf,
    pub state_dir: PathBuf,
    /// Handle SIGTERM, SIGINT and SIGHUP (Unix). Off for in-process tests.
    pub handle_signals: bool,
    /// Overrides the config's auth check interval, for tests.
    pub auth_interval: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("a Tunwarden daemon is already running for {}", .0.display())]
    AlreadyRunning(PathBuf),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("cannot set up {what}: {source}")]
    Io {
        what: String,
        #[source]
        source: io::Error,
    },
}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> DaemonError {
    let what = what.into();
    move |source| DaemonError::Io { what, source }
}

/// What the daemon broadcasts to subscribers.
#[derive(Clone, Debug)]
enum Notice {
    State(Status),
    Log { tunnel: String, line: String },
    Activity(ActivityEvent),
}

/// The supervisors' observer: persists, then broadcasts.
struct Hub {
    persister: Persister,
    tx: broadcast::Sender<Notice>,
}

impl Observer for Hub {
    fn log(&self, tunnel: &str, source: LogSource, line: &str) {
        self.persister.record_log(tunnel, source, line);
        let line = format_line(SystemTime::now(), source, line)
            .trim_end()
            .to_owned();
        let _ = self.tx.send(Notice::Log {
            tunnel: tunnel.to_owned(),
            line,
        });
    }

    fn status(&self, status: &Status) {
        if let Some(event) = self.persister.record_status(status) {
            let _ = self.tx.send(Notice::Activity(event));
        }
        let _ = self.tx.send(Notice::State(status.clone()));
    }
}

struct Entry {
    config: TunnelConfig,
    production: bool,
    /// The login this tunnel's profile uses, if any.
    login: Option<String>,
    supervisor: Supervisor,
    note: Option<String>,
}

struct Ctx {
    config_path: PathBuf,
    state_dir: PathBuf,
    config: std::sync::RwLock<Arc<Config>>,
    config_errors: Mutex<Vec<String>>,
    prober: Arc<Prober>,
    /// Serializes config writes and reloads.
    edits: tokio::sync::Mutex<Loaded>,
    notes: Vec<String>,
    tunnels: Mutex<BTreeMap<String, Entry>>,
    hub: Arc<Hub>,
    shutdown: watch::Sender<Option<bool>>,
    auth: tokio::sync::Mutex<BTreeMap<String, AuthState>>,
}

/// What the daemon knows about one login.
struct AuthState {
    profiles: Vec<String>,
    last: Option<(Outcome, String, Instant)>,
}

impl Ctx {
    fn config(&self) -> Arc<Config> {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn errors(&self) -> Vec<String> {
        self.config_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn errors_set(&self, errors: Vec<String>) {
        *self.config_errors.lock().unwrap_or_else(|e| e.into_inner()) = errors;
    }

    fn tunnels(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        self.tunnels.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Runs the daemon until it is asked to shut down.
pub async fn run(opts: Options) -> Result<(), DaemonError> {
    let state_dir = opts.state_dir.clone();
    private_dir(&state_dir).map_err(io_err(format!(
        "the state directory {}",
        state_dir.display()
    )))?;

    // 1. Single instance.
    let lock_path = state_dir.join("daemon.lock");
    let mut lock_options = OpenOptions::new();
    lock_options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let lock = lock_options
        .open(&lock_path)
        .map_err(io_err(format!("the lock file {}", lock_path.display())))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(DaemonError::AlreadyRunning(state_dir)),
        Err(TryLockError::Error(e)) => {
            return Err(io_err(format!("the lock file {}", lock_path.display()))(e));
        }
    }

    // 2. Config.
    let (config, config_errors, config_text) = load_config(&opts.config_path);

    // 3. State and crash sweep.
    let (store, previous) = StateStore::open(&state_dir)?;
    let mut notes: Vec<String> = previous.note.into_iter().collect();
    let logs = Logs::new(&state_dir, DEFAULT_MAX_BYTES).map_err(io_err("the log directory"))?;
    let activity =
        Activity::new(&state_dir, DEFAULT_MAX_BYTES).map_err(io_err("the activity log"))?;
    let persister = Persister::new(logs, activity, store);

    let crashed = previous.state.daemon.is_some();
    for (name, record) in &previous.state.tunnels {
        let Some(p) = record.process else { continue };
        let outcome = tokio::task::spawn_blocking(move || process::sweep(&p, DEFAULT_STOP_GRACE))
            .await
            .unwrap_or(SweepOutcome::NothingLeft);
        if outcome == SweepOutcome::Killed {
            persister.record_log(
                name,
                LogSource::Tunwarden,
                &format!(
                    "removed processes left by the previous daemon (PID {})",
                    p.pid
                ),
            );
        }
    }
    let _ = persister.state.mark_all_stopped(None);
    for name in previous.state.tunnels.keys() {
        if !config.tunnels.iter().any(|t| &t.name == name) {
            let _ = persister.state.remove(name);
        }
    }
    if crashed {
        notes.push(
            "the previous daemon stopped unexpectedly; leftover tunnel processes were removed"
                .to_owned(),
        );
    }

    // 4. Supervisors, restore and auto-start.
    let (tx, _) = broadcast::channel(1024);
    let hub = Arc::new(Hub { persister, tx });
    let prober = Arc::new(Prober::new());
    let mut tunnels = BTreeMap::new();
    for t in &config.tunnels {
        let mut entry = new_entry(t, &config, &prober, &hub);
        let was_running = previous
            .state
            .tunnels
            .get(&t.name)
            .is_some_and(|r| r.desired == Desired::Running);
        let restore = was_running && (!crashed || config.settings.restore_on_start);
        let note = match (crashed && was_running, restore) {
            (true, true) => Some("restored after the previous daemon stopped unexpectedly".to_owned()),
            (true, false) => Some(
                "was running when the previous daemon stopped unexpectedly; not restored because restore_on_start is off"
                    .to_owned(),
            ),
            _ => None,
        };
        if restore || t.auto_start {
            entry.supervisor.send(Event::Start);
        }
        entry.note = note;
        tunnels.insert(t.name.clone(), entry);
    }
    hub.persister
        .state
        .set_daemon(Some(DaemonInfo {
            pid: std::process::id(),
            started_at: now(),
            protocol: PROTOCOL,
        }))
        .map_err(io_err("the state file"))?;

    // 5. Serve.
    let endpoint = ipc::endpoint(&state_dir);
    let mut listener =
        Listener::bind(&endpoint).map_err(io_err(format!("the control endpoint {endpoint}")))?;
    let (shutdown, mut shutdown_rx) = watch::channel(None);
    let auth_interval = opts
        .auth_interval
        .unwrap_or(config.settings.login_check_interval);
    let auth_states = login_states(&config, &mut BTreeMap::new());
    let ctx = Arc::new(Ctx {
        config_path: opts.config_path.clone(),
        state_dir: state_dir.clone(),
        config: std::sync::RwLock::new(Arc::new(config)),
        config_errors: Mutex::new(config_errors),
        prober: prober.clone(),
        edits: tokio::sync::Mutex::new(Loaded::new(config_text, &opts.config_path)),
        notes,
        tunnels: Mutex::new(tunnels),
        hub,
        shutdown,
        auth: tokio::sync::Mutex::new(auth_states),
    });
    let watcher = tokio::spawn(auth_watcher(ctx.clone(), auth_interval));
    let config_watch = tokio::spawn(config_watcher(ctx.clone()));
    diag(&format!(
        "daemon {} (PID {}) listening on {endpoint}",
        crate::VERSION,
        std::process::id()
    ));

    let mut signals = Signals::new(opts.handle_signals);
    let mut connections = JoinSet::new();
    let keep = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    connections.spawn(serve(stream, ctx.clone()));
                }
                Err(e) => diag(&format!("accept failed: {e}")),
            },
            _ = shutdown_rx.changed() => {
                if let Some(keep) = *shutdown_rx.borrow() {
                    break keep;
                }
            }
            signal = signals.next() => match signal {
                Sig::Terminate => {
                    diag("received a stop signal; stopping tunnels and keeping them for the next start");
                    break true;
                }
                #[cfg(unix)]
                Sig::Ignore => {}
            },
        }
    };

    // Shutdown: stop every tunnel, then record how the daemon stopped.
    diag(&format!("shutting down (keep = {keep})"));
    watcher.abort();
    config_watch.abort();
    let entries = std::mem::take(&mut *ctx.tunnels());
    let mut stopping = JoinSet::new();
    for (_, entry) in entries {
        stopping.spawn(entry.supervisor.shutdown());
    }
    while stopping.join_next().await.is_some() {}
    let desired = if keep { None } else { Some(Desired::Stopped) };
    let state = &ctx.hub.persister.state;
    let _ = state.mark_all_stopped(desired);
    let _ = state.set_daemon(None);
    listener.cleanup();
    connections.abort_all();
    drop(lock);
    Ok(())
}

fn load_config(path: &Path) -> (Config, Vec<String>, Option<String>) {
    let text = read_config(path).ok().flatten();
    let (config, errors) = load_config_errors(path);
    (config, errors, text)
}

fn load_config_errors(path: &Path) -> (Config, Vec<String>) {
    let empty = Config::empty;
    match config::load(path) {
        Ok(c) => (c, Vec::new()),
        Err(LoadError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => (
            empty(),
            vec![format!(
                "no config file at {}; run `tunwarden init` to create one",
                path.display()
            )],
        ),
        Err(LoadError::Io { path, source }) => (
            empty(),
            vec![format!("cannot read {}: {source}", path.display())],
        ),
        Err(LoadError::Invalid(errors)) => {
            (empty(), errors.0.iter().map(|e| e.to_string()).collect())
        }
    }
}

/// A line for the daemon's own log. Its stderr goes to `<state>/daemon.log`.
fn diag(message: &str) {
    eprintln!("{} {message}", format_timestamp_now());
}

// --- OS signals ----------------------------------------------------------------

enum Sig {
    /// Stop and keep tunnels for the next start (logout, shutdown, Ctrl-C).
    Terminate,
    /// SIGHUP: a closed terminal must not stop the daemon.
    #[cfg(unix)]
    Ignore,
}

#[cfg(unix)]
struct Signals(
    Option<(
        tokio::signal::unix::Signal,
        tokio::signal::unix::Signal,
        tokio::signal::unix::Signal,
    )>,
);

#[cfg(unix)]
impl Signals {
    fn new(enabled: bool) -> Signals {
        use tokio::signal::unix::{SignalKind, signal};
        if !enabled {
            return Signals(None);
        }
        let make = |kind| signal(kind).ok();
        match (
            make(SignalKind::terminate()),
            make(SignalKind::interrupt()),
            make(SignalKind::hangup()),
        ) {
            (Some(t), Some(i), Some(h)) => Signals(Some((t, i, h))),
            _ => Signals(None),
        }
    }

    async fn next(&mut self) -> Sig {
        match &mut self.0 {
            Some((term, int, hup)) => tokio::select! {
                _ = term.recv() => Sig::Terminate,
                _ = int.recv() => Sig::Terminate,
                _ = hup.recv() => Sig::Ignore,
            },
            None => std::future::pending().await,
        }
    }
}

#[cfg(windows)]
struct Signals(bool);

#[cfg(windows)]
impl Signals {
    fn new(enabled: bool) -> Signals {
        Signals(enabled)
    }

    async fn next(&mut self) -> Sig {
        if self.0 {
            let _ = tokio::signal::ctrl_c().await;
            Sig::Terminate
        } else {
            std::future::pending().await
        }
    }
}

// --- Connections -------------------------------------------------------------

async fn serve(stream: Box<dyn Stream>, ctx: Arc<Ctx>) {
    let (read, mut write) = tokio::io::split(stream);
    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err()
                || write.write_all(b"\n").await.is_err()
            {
                break;
            }
        }
        let _ = write.shutdown().await;
    });

    let mut reader = BufReader::new(read);
    let mut greeted = false;
    let mut subscriptions: Vec<JoinHandle<()>> = Vec::new();
    loop {
        let line = match read_line(&mut reader).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(message) => {
                send(
                    &out,
                    &Response::err(
                        None,
                        RpcError {
                            code: RPC_PARSE_ERROR,
                            message,
                            data: None,
                        },
                    ),
                );
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::err(
                None,
                RpcError {
                    code: RPC_PARSE_ERROR,
                    message: format!("not a valid request: {e}"),
                    data: None,
                },
            ),
            Ok(req) if !greeted && req.method != method::HELLO => Response::err(
                Some(req.id),
                RpcError::app(
                    "handshake_required",
                    "the first request must be `hello`",
                    None,
                ),
            ),
            Ok(req) => match handle(&ctx, &req, &out, &mut subscriptions).await {
                Ok(result) => {
                    greeted |= req.method == method::HELLO;
                    Response::ok(req.id, result)
                }
                Err(e) => Response::err(Some(req.id), e),
            },
        };
        send(&out, &response);
    }
    for s in subscriptions {
        s.abort();
    }
    drop(out);
    let _ = writer.await;
}

/// One line, at most [`proto::MAX_LINE`] bytes. `Ok(None)` at end of stream.
async fn read_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<String>, String> {
    let mut buf = Vec::new();
    let read = (&mut *reader)
        .take(proto::MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)
        .await
        .map_err(|e| e.to_string())?;
    if read == 0 {
        return Ok(None);
    }
    if buf.len() > proto::MAX_LINE {
        return Err(format!("message longer than {} bytes", proto::MAX_LINE));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| "message is not UTF-8".to_owned())
}

fn send(out: &mpsc::UnboundedSender<String>, value: &impl serde::Serialize) {
    if let Ok(line) = serde_json::to_string(value) {
        let _ = out.send(line);
    }
}

fn params<T: DeserializeOwned>(req: &Request) -> Result<T, RpcError> {
    let value = if req.params.is_null() {
        json!({})
    } else {
        req.params.clone()
    };
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RPC_INVALID_PARAMS,
        message: format!("invalid parameters for `{}`: {e}", req.method),
        data: None,
    })
}

fn to_value(v: impl serde::Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::app("internal", e.to_string(), None))
}

async fn handle(
    ctx: &Arc<Ctx>,
    req: &Request,
    out: &mpsc::UnboundedSender<String>,
    subscriptions: &mut Vec<JoinHandle<()>>,
) -> Result<Value, RpcError> {
    match req.method.as_str() {
        method::AUTH_STATUS => {
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
        method::HELLO => {
            let hello: Hello = params(req)?;
            if hello.protocol != PROTOCOL {
                return Err(RpcError {
                    code: RPC_PROTOCOL_MISMATCH,
                    message: format!(
                        "the running daemon (Tunwarden {}) speaks protocol {PROTOCOL}, but this {} (Tunwarden {}) speaks protocol {}",
                        crate::VERSION,
                        hello.client,
                        hello.version,
                        hello.protocol
                    ),
                    data: Some(ErrorData {
                        code: "protocol_mismatch".into(),
                        hint: Some(
                            "run `tunwarden daemon restart` to start the installed version".into(),
                        ),
                    }),
                });
            }
            to_value(HelloResult {
                protocol: PROTOCOL,
                daemon_version: crate::VERSION.into(),
                pid: std::process::id(),
                config_path: ctx.config_path.display().to_string(),
                state_dir: ctx.state_dir.display().to_string(),
            })
        }
        method::STATUS => {
            let p: StatusParams = params(req)?;
            let tunnels = ctx.tunnels();
            for name in &p.names {
                check_name(&tunnels, name)?;
            }
            let views = tunnels
                .values()
                .filter(|e| p.names.is_empty() || p.names.contains(&e.config.name))
                .map(|e| view(e, &e.supervisor.status()))
                .collect();
            to_value(StatusResult {
                tunnels: views,
                config_errors: ctx.errors(),
                notes: ctx.notes.clone(),
            })
        }
        method::LIST => {
            let config = ctx.config();
            let list = ctx
                .tunnels()
                .values()
                .map(|e| tunnel_info(&config, &e.config))
                .collect();
            to_value(ListResult { tunnels: list })
        }
        method::START | method::STOP | method::RESTART => {
            let selector: Selector = params(req)?;
            let names = select(ctx, &selector)?;
            let event = match req.method.as_str() {
                method::START => Event::Start,
                method::STOP => Event::Stop,
                _ => Event::Restart,
            };
            let tunnels = ctx.tunnels();
            if !selector.dry_run {
                for name in &names {
                    if let Some(e) = tunnels.get(name) {
                        e.supervisor.send(event.clone());
                    }
                }
            }
            to_value(ActionResult { tunnels: names })
        }
        method::LOGS_TAIL => {
            let p: LogsParams = params(req)?;
            check_name(&ctx.tunnels(), &p.name)?;
            let lines = ctx
                .hub
                .persister
                .logs
                .tail(&p.name, p.lines.min(100_000))
                .map_err(|e| {
                    RpcError::app(
                        "io",
                        format!("cannot read the log of {}: {e}", p.name),
                        None,
                    )
                })?;
            to_value(LogsResult { lines })
        }
        method::ACTIVITY => {
            let p: ActivityParams = params(req)?;
            to_value(ActivityResult {
                events: ctx.hub.persister.activity.recent(p.limit),
            })
        }
        method::SUBSCRIBE => {
            let p: SubscribeParams = params(req)?;
            let mut logs = Vec::new();
            let (mut tunnels, mut activity) = (false, false);
            for topic in &p.topics {
                match topic.as_str() {
                    "tunnels" => tunnels = true,
                    "activity" => activity = true,
                    t => match t.strip_prefix("log:") {
                        Some(name) => {
                            check_name(&ctx.tunnels(), name)?;
                            logs.push(name.to_owned());
                        }
                        None => {
                            return Err(RpcError::app(
                                "unknown_topic",
                                format!("unknown topic `{t}`"),
                                Some("use `tunnels`, `activity` or `log:<tunnel>`".into()),
                            ));
                        }
                    },
                }
            }
            subscriptions.push(tokio::spawn(forward(
                ctx.clone(),
                out.clone(),
                tunnels,
                activity,
                logs,
            )));
            Ok(json!({}))
        }
        method::CONFIG_ADD => {
            let p: ConfigAddParams = params(req)?;
            edit_config(ctx, EditOp::Add(p.name, p.fields)).await
        }
        method::CONFIG_EDIT => {
            let p: ConfigEditParams = params(req)?;
            edit_config(ctx, EditOp::Edit(p.name, p.set, p.unset)).await
        }
        method::CONFIG_ADD_ENTRY => {
            let p: ConfigAddEntryParams = params(req)?;
            if !matches!(p.table.as_str(), "logins" | "profiles") {
                return Err(RpcError::app(
                    "invalid_edit",
                    format!("cannot add to `{}`; use logins or profiles", p.table),
                    None,
                ));
            }
            edit_config(ctx, EditOp::AddEntry(p.table, p.name, p.fields)).await
        }
        method::DOCTOR => {
            let config = ctx.config();
            to_value(DoctorResult {
                tools: crate::tools::check_all(&config).await,
                write_error: ctx.hub.persister.last_error(),
            })
        }
        method::CONFIG_REMOVE => {
            let p: ConfigRemoveParams = params(req)?;
            edit_config(ctx, EditOp::Remove(p.name)).await
        }
        method::SHUTDOWN => {
            let p: ShutdownParams = params(req)?;
            let _ = ctx.shutdown.send(Some(p.keep));
            Ok(json!({}))
        }
        other => Err(RpcError {
            code: RPC_UNKNOWN_METHOD,
            message: format!("unknown method `{other}`"),
            data: None,
        }),
    }
}

/// Sends matching broadcasts to one connection until it closes.
async fn forward(
    ctx: Arc<Ctx>,
    out: mpsc::UnboundedSender<String>,
    tunnels: bool,
    activity: bool,
    logs: Vec<String>,
) {
    let mut rx = ctx.hub.tx.subscribe();
    loop {
        let notice = match rx.recv().await {
            Ok(n) => n,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let message = match notice {
            Notice::State(status) if tunnels => {
                let view = ctx.tunnels().get(&status.name).map(|e| view(e, &status));
                match view {
                    Some(v) => Notification::new(notice::TUNNEL_STATE, json!(v)),
                    None => continue,
                }
            }
            Notice::Activity(event) if activity => {
                Notification::new(notice::ACTIVITY, json!(event))
            }
            Notice::Log { tunnel, line } if logs.contains(&tunnel) => {
                Notification::new(notice::LOG, json!(LogNotice { tunnel, line }))
            }
            _ => continue,
        };
        if out.is_closed() {
            return;
        }
        send(&out, &message);
    }
}

// --- Config changes (FR-5, design 3.3 and 3.4) -------------------------------

/// How often the config file is checked for hand edits.
const CONFIG_POLL: Duration = Duration::from_millis(500);

/// The config text the daemon last loaded, and the file's size and
/// modification time when it was read.
struct Loaded {
    text: Option<String>,
    stamp: Option<(SystemTime, u64)>,
}

impl Loaded {
    fn new(text: Option<String>, path: &Path) -> Loaded {
        Loaded {
            text,
            stamp: stamp(path),
        }
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn read_config(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The entry for one tunnel of `config`, with a stopped supervisor.
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
fn tunnel_changed(
    t: &TunnelConfig,
    old_cfg: &Config,
    old: &TunnelConfig,
    new_cfg: &Config,
) -> bool {
    let context = |c: &Config, t: &TunnelConfig| {
        let (p, l) = c.context_of(t);
        (p.cloned(), l.cloned())
    };
    old != t || context(old_cfg, old) != context(new_cfg, t) || old_cfg.tools != new_cfg.tools
}

/// The login states for `config`, keeping known results for logins that
/// still exist.
fn login_states(
    config: &Config,
    old: &mut BTreeMap<String, AuthState>,
) -> BTreeMap<String, AuthState> {
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

/// Switches the daemon to `new` (design 3.4). Added tunnels start stopped,
/// removed ones are stopped, and changed ones get their new settings at once
/// when stopped or on the next restart when running ("restart to apply").
async fn apply_config(ctx: &Ctx, new: Config) {
    let old = ctx.config();
    let mut removed = Vec::new();
    let mut summary = Vec::new();
    let mut confirmations = Vec::new();
    {
        let mut tunnels = ctx.tunnels();
        let gone: Vec<String> = tunnels
            .keys()
            .filter(|n| !new.tunnels.iter().any(|t| &t.name == *n))
            .cloned()
            .collect();
        for name in gone {
            if let Some(e) = tunnels.remove(&name) {
                summary.push(format!("removed {name}"));
                removed.push(e);
            }
        }
        for t in &new.tunnels {
            match tunnels.get_mut(&t.name) {
                None => {
                    summary.push(format!("added {}", t.name));
                    tunnels.insert(t.name.clone(), new_entry(t, &new, &ctx.prober, &ctx.hub));
                }
                Some(e) if tunnel_changed(t, &old, &e.config, &new) => {
                    summary.push(format!("changed {}", t.name));
                    let fresh = new_entry_parts(t, &new);
                    confirmations.push(e.supervisor.update_confirmed(fresh.0));
                    e.config = t.clone();
                    e.production = fresh.1;
                    e.login = fresh.2;
                }
                Some(_) => {}
            }
        }
    }
    for c in confirmations {
        let _ = c.await;
    }
    *ctx.config.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(new.clone());
    ctx.errors_set(Vec::new());

    // Logins: keep what is known about logins that still exist.
    {
        let mut auth_states = ctx.auth.lock().await;
        let fresh = login_states(&new, &mut auth_states);
        *auth_states = fresh;
    }

    for e in removed {
        let name = e.config.name.clone();
        ctx.hub.persister.record_log(
            &name,
            LogSource::Tunwarden,
            "removed from the config; stopping",
        );
        e.supervisor.shutdown().await;
        let _ = ctx.hub.persister.state.remove(&name);
        ctx.hub.persister.logs.close(&name);
    }
    if !summary.is_empty() {
        diag(&format!("config reloaded: {}", summary.join(", ")));
    }
}

/// The spec, production flag and login name of `t` in `config`.
fn new_entry_parts(t: &TunnelConfig, config: &Config) -> (Spec, bool, Option<String>) {
    let (profile, login) = config.context_of(t);
    (
        Spec::new(t, config),
        profile.is_some_and(|p| p.production),
        login.map(|l| l.name.clone()),
    )
}

/// Reloads after a hand edit. Called with the edit lock held. An invalid file
/// keeps the current config and is reported in `status` (FR-5).
async fn reload(ctx: &Ctx, loaded: &mut Loaded, text: Option<String>) {
    loaded.stamp = stamp(&ctx.config_path);
    if text == loaded.text {
        return;
    }
    loaded.text = text.clone();
    let Some(text) = text else {
        ctx.errors_set(vec![format!(
            "the config file {} was removed; the daemon keeps the last valid config",
            ctx.config_path.display()
        )]);
        return;
    };
    match config::parse(&text, &ctx.config_path, &crate::paths::SystemEnv) {
        Ok(new) => apply_config(ctx, new).await,
        Err(errors) => {
            diag("the edited config file is invalid; keeping the last valid config");
            ctx.errors_set(errors.0.iter().map(|e| e.to_string()).collect());
        }
    }
}

/// Watches the config file for hand edits.
async fn config_watcher(ctx: Arc<Ctx>) {
    let mut tick = tokio::time::interval(CONFIG_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let now = stamp(&ctx.config_path);
        let mut loaded = ctx.edits.lock().await;
        if now == loaded.stamp {
            continue;
        }
        match read_config(&ctx.config_path) {
            Ok(text) => reload(&ctx, &mut loaded, text).await,
            Err(e) => diag(&format!("cannot read {}: {e}", ctx.config_path.display())),
        }
    }
}

enum EditOp {
    Add(String, serde_json::Map<String, Value>),
    Edit(String, serde_json::Map<String, Value>, Vec<String>),
    Remove(String),
    AddEntry(String, String, serde_json::Map<String, Value>),
}

/// Applies a tool edit (design 3.3): reload hand edits first, edit the text,
/// validate it, write it atomically, then switch to it.
async fn edit_config(ctx: &Ctx, op: EditOp) -> Result<Value, RpcError> {
    use crate::config::edit;
    let mut loaded = ctx.edits.lock().await;
    let current = read_config(&ctx.config_path).map_err(|e| {
        RpcError::app(
            "io",
            format!("cannot read {}: {e}", ctx.config_path.display()),
            None,
        )
    })?;
    if current != loaded.text {
        reload(ctx, &mut loaded, current.clone()).await;
    }
    let errors = ctx.errors();
    if !errors.is_empty() && current.is_some() {
        return Err(RpcError::app(
            "invalid_config",
            format!(
                "the config file has problems, so it cannot be edited:\n{}",
                errors.join("\n")
            ),
            Some(format!("fix {} first", ctx.config_path.display())),
        ));
    }
    let text = current.clone().unwrap_or_else(|| edit::NEW_FILE.to_owned());
    let mut doc =
        edit::parse(&text).map_err(|e| RpcError::app("invalid_config", e.to_string(), None))?;
    let (name, verb) = match &op {
        EditOp::Add(name, fields) => {
            edit::add_tunnel(&mut doc, name, fields).map_err(edit_error)?;
            (name.clone(), "added")
        }
        EditOp::Edit(name, set, unset) => {
            edit::edit_tunnel(&mut doc, name, set, unset).map_err(edit_error)?;
            (name.clone(), "updated")
        }
        EditOp::Remove(name) => {
            edit::remove_tunnel(&mut doc, name).map_err(edit_error)?;
            (name.clone(), "removed")
        }
        EditOp::AddEntry(table, name, fields) => {
            edit::add_entry(&mut doc, table, name, fields).map_err(edit_error)?;
            (name.clone(), "added")
        }
    };
    let new_text = doc.to_string();
    let new =
        config::parse(&new_text, &ctx.config_path, &crate::paths::SystemEnv).map_err(|errors| {
            RpcError::app(
                "invalid_config",
                format!(
                    "the change would make the config invalid, so nothing was written:\n{}",
                    errors
                        .0
                        .iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
                None,
            )
        })?;
    if let Some(dir) = ctx.config_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            RpcError::app("io", format!("cannot create {}: {e}", dir.display()), None)
        })?;
    }
    crate::fsutil::write_atomic(&ctx.config_path, new_text.as_bytes()).map_err(|e| {
        RpcError::app(
            "io",
            format!("cannot write {}: {e}", ctx.config_path.display()),
            None,
        )
    })?;
    loaded.text = Some(new_text);
    loaded.stamp = stamp(&ctx.config_path);
    apply_config(ctx, new).await;

    // The supervisor gets the new settings asynchronously, so decide from
    // the state: an updated tunnel that is running needs a restart.
    let running = ctx
        .tunnels()
        .get(&name)
        .is_some_and(|e| is_active(e.supervisor.status().state));
    to_value(ConfigEditResult {
        tunnel: name,
        change: verb.to_owned(),
        config_path: ctx.config_path.display().to_string(),
        restart_to_apply: verb == "updated" && running,
    })
}

fn edit_error(e: crate::config::edit::EditError) -> RpcError {
    use crate::config::edit::EditError;
    let code = match e {
        EditError::UnknownTunnel(_) => "unknown_tunnel",
        EditError::Exists(..) => "already_exists",
        _ => "invalid_edit",
    };
    RpcError::app(code, e.to_string(), None)
}

// --- Logins (FR-22 to FR-25) ---------------------------------------------------

/// The login named `name` (or used by the profile named `name`), or all logins.
async fn login_names(ctx: &Ctx, name: Option<&str>) -> Result<Vec<String>, RpcError> {
    let config = ctx.config();
    let Some(name) = name else {
        return Ok(config.logins.iter().map(|l| l.name.clone()).collect());
    };
    if config.login(name).is_some() {
        return Ok(vec![name.to_owned()]);
    }
    if let Some(profile) = config.profile(name) {
        return match &profile.login {
            Some(login) => Ok(vec![login.clone()]),
            None => Err(RpcError::app(
                "no_login",
                format!("profile \"{name}\" has no login"),
                Some("set `login = \"NAME\"` in the profile to use one".into()),
            )),
        };
    }
    let known: Vec<&str> = config.logins.iter().map(|l| l.name.as_str()).collect();
    Err(RpcError::app(
        "unknown_login",
        format!("no login or profile named \"{name}\""),
        Some(if known.is_empty() {
            "no logins are configured; add a [[logins]] entry".into()
        } else {
            format!("configured logins: {}", known.join(", "))
        }),
    ))
}

fn is_active(state: State) -> bool {
    matches!(
        state,
        State::Starting | State::Running | State::Unhealthy | State::Reconnecting
    )
}

/// Active tunnels that use the login `name`.
fn active_in(ctx: &Ctx, name: &str) -> Vec<String> {
    ctx.tunnels()
        .values()
        .filter(|e| e.login.as_deref() == Some(name) && is_active(e.supervisor.status().state))
        .map(|e| e.config.name.clone())
        .collect()
}

/// Checks one login now. If it has expired, its active tunnels fail with
/// `auth expired`. An unknown result changes nothing (FR-22).
async fn check_login(ctx: &Ctx, name: &str) -> Outcome {
    let config = ctx.config();
    let Some(login) = config.login(name) else {
        return Outcome::Unknown {
            detail: format!("no login named {name}"),
        };
    };
    let outcome = auth::check(login, &config.tools).await;
    if login.check.is_some()
        && let Some(state) = ctx.auth.lock().await.get_mut(name)
    {
        state.last = Some((outcome.clone(), now(), Instant::now()));
    }
    match &outcome {
        Outcome::Expired { .. } => {
            let tunnels = ctx.tunnels();
            for e in tunnels
                .values()
                .filter(|e| e.login.as_deref() == Some(name))
            {
                e.supervisor.send(Event::AuthExpired(format!(
                    "the {name} login has expired; run `tunwarden login {name}`"
                )));
            }
            diag(&format!("the {name} login has expired"));
        }
        Outcome::Unknown { detail } => {
            diag(&format!(
                "the check of the {name} login was inconclusive: {detail}"
            ));
        }
        Outcome::Ok => {}
    }
    outcome
}

async fn login_views(ctx: &Ctx, names: &[String]) -> Vec<LoginView> {
    let config = ctx.config();
    let states = ctx.auth.lock().await;
    names
        .iter()
        .filter_map(|name| {
            let s = states.get(name)?;
            Some(LoginView {
                name: name.clone(),
                profiles: s.profiles.clone(),
                checkable: config.login(name).is_some_and(|l| l.check.is_some()),
                result: s
                    .last
                    .as_ref()
                    .map_or("not checked", |(o, _, _)| o.name())
                    .to_owned(),
                detail: s.last.as_ref().and_then(|(o, _, _)| o.detail()),
                checked_at: s.last.as_ref().map(|(_, at, _)| at.clone()),
                active_tunnels: active_in(ctx, name),
            })
        })
        .collect()
}

/// Checks logins that have a `check` command and active tunnels on a
/// schedule, and right away when a tunnel fails because its login expired,
/// so the other tunnels sharing that login are caught too.
async fn auth_watcher(ctx: Arc<Ctx>, interval: Duration) {
    const MIN_GAP: Duration = Duration::from_secs(10);
    let mut notices = ctx.hub.tx.subscribe();
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // the first tick is immediate; nothing is running yet
    loop {
        let due: Vec<String> = tokio::select! {
            _ = tick.tick() => {
                let config = ctx.config();
                config
                    .logins
                    .iter()
                    .filter(|l| l.check.is_some() && !active_in(&ctx, &l.name).is_empty())
                    .map(|l| l.name.clone())
                    .collect()
            }
            notice = notices.recv() => match notice {
                Ok(Notice::State(s)) if s.state == State::Failed && s.reason == Some(FailureReason::AuthExpired) => {
                    let login = ctx.tunnels().get(&s.name).and_then(|e| e.login.clone());
                    let Some(login) = login else { continue };
                    let recent = ctx
                        .auth
                        .lock()
                        .await
                        .get(&login)
                        .and_then(|st| st.last.as_ref())
                        .is_some_and(|(_, _, at)| at.elapsed() < MIN_GAP);
                    let checkable = ctx.config().login(&login).is_some_and(|l| l.check.is_some());
                    if recent || !checkable { continue }
                    vec![login]
                }
                Err(broadcast::error::RecvError::Closed) => return,
                _ => continue,
            },
        };
        for name in due {
            check_login(&ctx, &name).await;
        }
    }
}

fn check_name(tunnels: &BTreeMap<String, Entry>, name: &str) -> Result<(), RpcError> {
    if tunnels.contains_key(name) {
        return Ok(());
    }
    let known: Vec<&str> = tunnels.keys().map(String::as_str).collect();
    let hint = if known.is_empty() {
        "no tunnels are configured".to_owned()
    } else {
        format!("configured tunnels: {}", known.join(", "))
    };
    Err(RpcError::app(
        "unknown_tunnel",
        format!("no tunnel named \"{name}\""),
        Some(hint),
    ))
}

/// Resolves a selector to tunnel names in config order (FR-9).
fn select(ctx: &Ctx, s: &Selector) -> Result<Vec<String>, RpcError> {
    let tunnels = ctx.tunnels();
    if s.all {
        return Ok(tunnels.keys().cloned().collect());
    }
    if s.names.is_empty() && s.group.is_none() && s.profile.is_none() {
        return Err(RpcError::app(
            "no_selection",
            "no tunnels selected",
            Some("name one or more tunnels, or use --all, --group or --profile".into()),
        ));
    }
    let mut names: Vec<String> = Vec::new();
    for name in &s.names {
        check_name(&tunnels, name)?;
        names.push(name.clone());
    }
    if let Some(group) = &s.group {
        let members: Vec<String> = tunnels
            .values()
            .filter(|e| e.config.groups.contains(group))
            .map(|e| e.config.name.clone())
            .collect();
        if members.is_empty() {
            return Err(RpcError::app(
                "unknown_group",
                format!("no tunnel is in group \"{group}\""),
                None,
            ));
        }
        names.extend(members);
    }
    if let Some(profile) = &s.profile {
        if ctx.config().profile(profile).is_none() {
            return Err(RpcError::app(
                "unknown_profile",
                format!("no profile named \"{profile}\""),
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
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    Ok(names)
}

/// A tunnel's configuration as `list` shows it. Also used by the CLI, which
/// reads the config file directly.
pub fn tunnel_info(config: &Config, t: &TunnelConfig) -> TunnelInfo {
    let (profile, login) = config.context_of(t);
    TunnelInfo {
        name: t.name.clone(),
        kind: t.kind.id().into(),
        profile: t.profile.clone(),
        production: profile.is_some_and(|p| p.production),
        groups: t.groups.clone(),
        local: local(t),
        command: t.launch_plan(profile, login).display(),
    }
}

fn local(t: &TunnelConfig) -> Vec<String> {
    t.forwards
        .iter()
        .map(|f| std::net::SocketAddr::new(f.bind, f.local_port).to_string())
        .collect()
}

fn view(e: &Entry, s: &Status) -> TunnelView {
    TunnelView {
        name: e.config.name.clone(),
        kind: e.config.kind.id().into(),
        profile: e.config.profile.clone(),
        production: e.production,
        groups: e.config.groups.clone(),
        local: local(&e.config),
        state: s.state,
        reason: s.reason,
        last_error: s.last_error.clone(),
        pid: s.process.map(|p| p.pid),
        restarts: s.restarts,
        retries: s.retries,
        since: crate::fsutil::timestamp(s.since),
        desired: if s.desired_running {
            Desired::Running
        } else {
            Desired::Stopped
        },
        pending_restart: s.pending_restart,
        note: e
            .note
            .clone()
            .filter(|_| s.state != State::Failed || s.reason.is_none()),
    }
}
