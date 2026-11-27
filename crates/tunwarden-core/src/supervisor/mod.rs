//! The supervisor: one task per tunnel that runs its command and carries out
//! the state machine's actions (design section 7).
//!
//! The task owns the process, the output readers and the timers. Everything it
//! observes becomes an [`Event`] for [`Machine::step`], and every [`Action`] the
//! machine returns is carried out here. Output and status go to an
//! [`Observer`]; logs and the state store (M1 step 6) are observers.

mod machine;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub use machine::{Action, Event, Machine, Policy, ProbeResult, ReadyMode, State, Timer};

use crate::config::{Config, ToolPaths, TunnelConfig};
use crate::failure::FailureReason;
use crate::paths::{Os, SystemEnv};
use crate::probe::{Prober, Ready};
use crate::process::{DEFAULT_STOP_GRACE, ProcessRecord, TunnelProcess};
use crate::tools;
use crate::types::{LaunchPlan, OutputPattern};

/// How often a running command is checked for exit.
const EXIT_POLL: Duration = Duration::from_millis(100);
/// How long to wait for buffered output after the command exits, so an error
/// printed just before exiting is classified before the exit is handled.
const OUTPUT_DRAIN: Duration = Duration::from_millis(300);
/// Output lines longer than this are cut (design 7.3).
const MAX_LINE: usize = 8 * 1024;

/// Everything needed to run one tunnel.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    pub plan: LaunchPlan,
    pub tools: ToolPaths,
    /// Local addresses checked for conflicts before a spawn (FR-13) and, in
    /// `Listening` mode, for readiness (FR-16).
    pub listen: Vec<SocketAddr>,
    /// Output patterns with their hints already filled in.
    pub patterns: Vec<(OutputPattern, String)>,
    pub policy: Policy,
    pub stop_grace: Duration,
}

impl Spec {
    /// The spec of `tunnel`, with its profile's and login's environment and
    /// its login's patterns.
    pub fn new(tunnel: &TunnelConfig, config: &Config) -> Spec {
        let (profile, login) = config.context_of(tunnel);
        let listen = if tunnel.kind.listens_locally() {
            tunnel
                .forwards
                .iter()
                .map(|f| SocketAddr::new(f.bind, f.local_port))
                .collect()
        } else {
            Vec::new()
        };
        let patterns = tunnel
            .output_patterns(login)
            .into_iter()
            .map(|p| {
                let hint = tunnel.render_hint(&p.hint);
                (p, hint)
            })
            .collect();
        Spec {
            name: tunnel.name.clone(),
            plan: tunnel.launch_plan(profile, login),
            tools: config.tools.clone(),
            listen,
            patterns,
            policy: Policy::for_tunnel(tunnel),
            stop_grace: DEFAULT_STOP_GRACE,
        }
    }
}

/// Where a log line came from (FR-27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Stdout,
    Stderr,
    /// Tunwarden's own events.
    Tunwarden,
}

/// A tunnel's current status, as shown by `status` and the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub name: String,
    pub state: State,
    pub reason: Option<FailureReason>,
    pub last_error: Option<String>,
    pub process: Option<ProcessRecord>,
    pub restarts: u32,
    pub retries: u32,
    /// When the current state began.
    pub since: SystemTime,
    pub desired_running: bool,
    /// The config changed while running; a restart applies it (FR-5).
    pub pending_restart: bool,
}

/// Receives a tunnel's output and status changes.
pub trait Observer: Send + Sync + 'static {
    fn log(&self, tunnel: &str, source: LogSource, line: &str);
    fn status(&self, status: &Status);
}

/// An observer that ignores everything.
pub struct NoObserver;

impl Observer for NoObserver {
    fn log(&self, _: &str, _: LogSource, _: &str) {}
    fn status(&self, _: &Status) {}
}

enum Command {
    Event(Event),
    Update(Box<Spec>, Option<oneshot::Sender<()>>),
    Shutdown(oneshot::Sender<()>),
}

/// A handle to one tunnel's supervisor task.
pub struct Supervisor {
    tx: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<Status>,
    task: JoinHandle<()>,
}

impl Supervisor {
    pub fn spawn(spec: Spec, prober: Arc<Prober>, observer: Arc<dyn Observer>) -> Supervisor {
        let machine = Machine::new(spec.policy.clone());
        let initial = Status {
            name: spec.name.clone(),
            state: State::Stopped,
            reason: None,
            last_error: None,
            process: None,
            restarts: 0,
            retries: 0,
            since: SystemTime::now(),
            desired_running: false,
            pending_restart: false,
        };
        let (status_tx, status) = watch::channel(initial);
        let (tx, rx) = mpsc::unbounded_channel();
        let runtime = Runtime {
            spec,
            pending: None,
            machine,
            prober,
            observer,
            status: status_tx,
            process: None,
            generation: 0,
            readers: Vec::new(),
            timers: HashMap::new(),
            since: SystemTime::now(),
        };
        let task = tokio::spawn(runtime.run(rx));
        Supervisor { tx, status, task }
    }

    /// Sends an event such as [`Event::Start`] or [`Event::AuthRecovered`].
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(Command::Event(event));
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// New settings from a config reload. Applied at once if nothing is
    /// running, otherwise on the next start ("restart to apply", FR-5).
    pub fn update(&self, spec: Spec) {
        let _ = self.tx.send(Command::Update(Box::new(spec), None));
    }

    /// Like [`Supervisor::update`]; the returned receiver completes once the
    /// status shows the change, so a following `status` is consistent.
    pub fn update_confirmed(&self, spec: Spec) -> oneshot::Receiver<()> {
        let (done, wait) = oneshot::channel();
        let _ = self.tx.send(Command::Update(Box::new(spec), Some(done)));
        wait
    }

    /// Stops the tunnel's process (if any) and ends the task.
    pub async fn shutdown(self) {
        let (done, wait) = oneshot::channel();
        if self.tx.send(Command::Shutdown(done)).is_ok() {
            let _ = wait.await;
        }
        let _ = self.task.await;
    }
}

/// An event from a reader task, tagged with the process generation it came from.
struct Tagged {
    generation: u64,
    event: Event,
}

struct Runtime {
    spec: Spec,
    pending: Option<Spec>,
    machine: Machine,
    prober: Arc<Prober>,
    observer: Arc<dyn Observer>,
    status: watch::Sender<Status>,
    process: Option<TunnelProcess>,
    generation: u64,
    readers: Vec<JoinHandle<()>>,
    timers: HashMap<Timer, Instant>,
    since: SystemTime,
}

impl Runtime {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Command>) {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Tagged>();
        let mut exit_poll = tokio::time::interval(EXIT_POLL);
        exit_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let next_timer = self
                .timers
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(t, at)| (*t, *at));
            let watching_exit = self.process.is_some();
            tokio::select! {
                command = rx.recv() => match command {
                    Some(Command::Event(event)) => self.handle(event, &out_tx).await,
                    Some(Command::Update(spec, done)) => {
                        self.update(*spec);
                        if let Some(done) = done {
                            let _ = done.send(());
                        }
                    }
                    Some(Command::Shutdown(done)) => {
                        self.stop_now().await;
                        let _ = done.send(());
                        return;
                    }
                    // Every handle is gone: stop the tunnel and end.
                    None => {
                        self.stop_now().await;
                        return;
                    }
                },
                Some(tagged) = out_rx.recv() => {
                    if tagged.generation == self.generation {
                        self.handle(tagged.event, &out_tx).await;
                    }
                }
                _ = sleep_until(next_timer.map(|(_, at)| at)) => {
                    if let Some((timer, _)) = next_timer {
                        self.timers.remove(&timer);
                        self.handle(Event::Timer(timer), &out_tx).await;
                    }
                }
                _ = exit_poll.tick(), if watching_exit => {
                    if let Some(status) = self.exited() {
                        // Classify output printed just before the exit first.
                        self.drain_readers(&mut out_rx, &out_tx).await;
                        self.handle(Event::Exited(describe(status)), &out_tx).await;
                    }
                }
            }
        }
    }

    fn exited(&mut self) -> Option<ExitStatus> {
        let p = self.process.as_mut()?;
        p.try_wait().unwrap_or_default()
    }

    async fn drain_readers(
        &mut self,
        out_rx: &mut mpsc::UnboundedReceiver<Tagged>,
        out_tx: &mpsc::UnboundedSender<Tagged>,
    ) {
        let readers = std::mem::take(&mut self.readers);
        let _ = tokio::time::timeout(OUTPUT_DRAIN, async {
            for r in readers {
                let _ = r.await;
            }
        })
        .await;
        while let Ok(tagged) = out_rx.try_recv() {
            if tagged.generation == self.generation {
                self.handle(tagged.event, out_tx).await;
            }
        }
    }

    /// Feeds one event to the machine and carries out the resulting actions,
    /// including events the actions produce.
    async fn handle(&mut self, event: Event, out_tx: &mpsc::UnboundedSender<Tagged>) {
        let mut queue = vec![event];
        while let Some(event) = queue.pop() {
            let before = self.machine.state;
            let actions = self.machine.step(event);
            if self.machine.state != before {
                self.since = SystemTime::now();
                self.log(&format!(
                    "{} -> {}",
                    before.as_str(),
                    self.machine.state.as_str()
                ));
            }
            for action in actions {
                if let Some(next) = self.act(action, out_tx).await {
                    queue.insert(0, next);
                }
            }
            if !self.machine.has_process() {
                self.apply_pending();
            }
            self.publish();
        }
    }

    async fn act(
        &mut self,
        action: Action,
        out_tx: &mpsc::UnboundedSender<Tagged>,
    ) -> Option<Event> {
        match action {
            Action::Spawn => Some(self.spawn(out_tx).await),
            Action::Terminate => {
                self.terminate().await;
                Some(Event::Terminated)
            }
            Action::Probe => Some(Event::Probe(self.probe().await)),
            Action::SetTimer(timer, after) => {
                self.timers.insert(timer, Instant::now() + after);
                None
            }
            Action::CancelTimer(timer) => {
                self.timers.remove(&timer);
                None
            }
            Action::Log(line) => {
                self.log(&line);
                None
            }
        }
    }

    async fn spawn(&mut self, out_tx: &mpsc::UnboundedSender<Tagged>) -> Event {
        self.apply_pending();
        let spec = &self.spec;
        let program =
            match tools::resolve(&spec.plan.program, &spec.tools, &SystemEnv, Os::current()) {
                Ok(resolved) => resolved.path,
                Err(e) => {
                    return Event::SpawnFailed {
                        reason: FailureReason::MissingTool,
                        message: e.to_string(),
                    };
                }
            };
        if let Some(conflict) = self.port_conflict().await {
            return Event::SpawnFailed {
                reason: FailureReason::PortConflict,
                message: conflict,
            };
        }
        let spec = &self.spec;
        match TunnelProcess::spawn(program.as_os_str(), &spec.plan.args, &spec.plan.env) {
            Ok(mut process) => {
                self.generation += 1;
                let stdout = process.take_stdout();
                let stderr = process.take_stderr();
                if let Some(out) = stdout {
                    self.readers
                        .push(self.reader(out, LogSource::Stdout, out_tx));
                }
                if let Some(err) = stderr {
                    self.readers
                        .push(self.reader(err, LogSource::Stderr, out_tx));
                }
                self.log(&format!(
                    "started PID {}: {}",
                    process.record().pid,
                    spec.plan.display()
                ));
                self.process = Some(process);
                Event::Spawned
            }
            Err(e) => Event::SpawnFailed {
                reason: FailureReason::SpawnError,
                message: format!("cannot start {}: {e}", program.display()),
            },
        }
    }

    async fn port_conflict(&self) -> Option<String> {
        if self.spec.listen.is_empty() {
            return None;
        }
        let snapshot = self.prober.fresh_snapshot().await.ok()?;
        self.spec.listen.iter().find_map(|addr| {
            snapshot.holder(*addr).map(|h| {
                format!(
                    "local address {addr} is already in use by {} (PID {}); stop it or change local_port for {}",
                    h.name, h.pid, self.spec.name
                )
            })
        })
    }

    fn reader(
        &self,
        stream: impl AsyncRead + Unpin + Send + 'static,
        source: LogSource,
        out_tx: &mpsc::UnboundedSender<Tagged>,
    ) -> JoinHandle<()> {
        let observer = self.observer.clone();
        let name = self.spec.name.clone();
        let patterns: Vec<(OutputPattern, String)> = self.spec.patterns.clone();
        let tx = out_tx.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let mut line = String::from_utf8_lossy(&buf)
                    .trim_end_matches(['\r', '\n'])
                    .to_owned();
                if line.len() > MAX_LINE {
                    let mut cut = MAX_LINE;
                    while !line.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    line.truncate(cut);
                }
                observer.log(&name, source, &line);
                if let Some((hit, hint)) = patterns.iter().find(|(p, _)| p.regex.is_match(&line)) {
                    let _ = tx.send(Tagged {
                        generation,
                        event: Event::Output {
                            reason: hit.reason,
                            message: hint.clone(),
                        },
                    });
                }
            }
        })
    }

    async fn terminate(&mut self) {
        if let Some(mut process) = self.process.take() {
            let pid = process.record().pid;
            match process.stop(self.spec.stop_grace).await {
                Ok(outcome) => self.log(&format!("stopped PID {pid} ({outcome:?})")),
                Err(e) => self.log(&format!("could not stop PID {pid}: {e}")),
            }
        }
        for r in self.readers.drain(..) {
            r.abort();
        }
        // Output from the old process must not reach the next one.
        self.generation += 1;
    }

    async fn probe(&self) -> ProbeResult {
        let Some(process) = &self.process else {
            return ProbeResult::Waiting;
        };
        let pid = process.record().pid;
        match self.prober.snapshot().await {
            Ok(snapshot) => match snapshot.ready(pid, &self.spec.listen) {
                Ready::Yes => ProbeResult::Ready,
                Ready::Waiting(_) => ProbeResult::Waiting,
                Ready::HeldByOther(addr, h) => ProbeResult::HeldByOther(format!(
                    "local address {addr} is held by {} (PID {}), not by the tunnel; stop it or change local_port for {}",
                    h.name, h.pid, self.spec.name
                )),
            },
            Err(e) => ProbeResult::Error(e.to_string()),
        }
    }

    async fn stop_now(&mut self) {
        self.timers.clear();
        if self.process.is_some() {
            self.terminate().await;
        }
    }

    fn update(&mut self, spec: Spec) {
        self.pending = Some(spec);
        if !self.machine.has_process() {
            self.apply_pending();
        }
        self.publish();
    }

    fn apply_pending(&mut self) {
        if let Some(spec) = self.pending.take() {
            self.machine.set_policy(spec.policy.clone());
            self.spec = spec;
        }
    }

    fn log(&self, line: &str) {
        self.observer
            .log(&self.spec.name, LogSource::Tunwarden, line);
    }

    fn publish(&self) {
        let m = &self.machine;
        let status = Status {
            name: self.spec.name.clone(),
            state: m.state,
            reason: m.reason,
            last_error: m.last_error.clone(),
            process: self.process.as_ref().map(TunnelProcess::record),
            restarts: m.restarts,
            retries: m.retries,
            since: self.since,
            desired_running: m.desired_running,
            pending_restart: self.pending.is_some(),
        };
        if *self.status.borrow() != status {
            self.observer.status(&status);
            self.status.send_replace(status);
        }
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn describe(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exited with code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("was killed by signal {signal}");
        }
    }
    "exited".to_owned()
}

#[cfg(test)]
mod machine_tests;
