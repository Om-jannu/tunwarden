//! The per-tunnel state machine (PRD section 8, design 7.1).
//!
//! `Machine::step` is a pure function of the current state and one event. It
//! returns actions for the runtime to carry out (spawn, terminate, probe, set a
//! timer, log). Every rule from PRD sections 7.3, 7.4 and 8 can therefore be
//! tested with plain unit tests, without processes or real time.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::TunnelConfig;
use crate::failure::FailureReason;
use crate::types::Readiness;

/// The six visible states (FR-11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Unhealthy,
    Reconnecting,
    Failed,
}

impl State {
    pub const fn as_str(self) -> &'static str {
        match self {
            State::Stopped => "stopped",
            State::Starting => "starting",
            State::Running => "running",
            State::Unhealthy => "unhealthy",
            State::Reconnecting => "reconnecting",
            State::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Timer {
    /// Time for the next readiness check.
    Probe,
    /// The startup grace period ended (FR-17).
    StartupGrace,
    /// A "process alive" tunnel has stayed up long enough (FR-44).
    AliveReady,
    /// The tunnel stayed up long enough to reset its retry counter (FR-18).
    StableReset,
    /// The backoff delay before the next retry ended (FR-18).
    Backoff,
}

/// The outcome of a readiness check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    Ready,
    Waiting,
    /// A wanted port is held by another process; the message names it.
    HeldByOther(String),
    /// The socket table could not be read.
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user asked to start (FR-9).
    Start,
    /// The user asked to stop.
    Stop,
    /// The user asked to restart.
    Restart,
    /// The command was started.
    Spawned,
    /// The command could not be started.
    SpawnFailed {
        reason: FailureReason,
        message: String,
    },
    /// The command exited; the text describes how.
    Exited(String),
    Probe(ProbeResult),
    /// The command printed something retrying cannot fix (FR-24, FR-45).
    Output {
        reason: FailureReason,
        message: String,
    },
    Timer(Timer),
    /// The command and everything it started are gone.
    Terminated,
    /// The scheduled check found the tunnel's login expired (FR-22).
    AuthExpired(String),
    /// The tunnel's login succeeded again (FR-25).
    AuthRecovered,
    /// The system woke from sleep or the network changed (FR-19).
    Wake,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Check prerequisites and ports, then start the command. Reports back
    /// `Spawned` or `SpawnFailed`.
    Spawn,
    /// Stop the command and everything it started. Reports back `Terminated`.
    Terminate,
    /// Check readiness. Reports back `Probe`.
    Probe,
    SetTimer(Timer, Duration),
    CancelTimer(Timer),
    /// A line for the tunnel's log (FR-27).
    Log(String),
}

/// How readiness is decided for this tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadyMode {
    Listening,
    AliveFor(Duration),
}

/// Retry and health settings for one tunnel.
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub auto_restart: bool,
    pub max_retries: u32,
    pub health_interval: Duration,
    pub ready: ReadyMode,
    pub startup_grace: Duration,
    pub starting_probe: Duration,
    pub failed_check_limit: u32,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub stable_after: Duration,
}

impl Policy {
    /// The PRD defaults (FR-16 to FR-18) with the tunnel's own settings.
    pub fn for_tunnel(t: &TunnelConfig) -> Policy {
        Policy {
            auto_restart: t.auto_restart,
            max_retries: t.max_retries,
            health_interval: t.health_interval,
            ready: match t.readiness() {
                Readiness::Listening(_) => ReadyMode::Listening,
                Readiness::AliveFor(d) => ReadyMode::AliveFor(d),
            },
            ..Policy::default()
        }
    }

    /// The delay before retry number `retry` (0-based): 1 s, 2 s, 4 s ... capped.
    pub fn backoff(&self, retry: u32) -> Duration {
        let factor = 1u32.checked_shl(retry.min(31)).unwrap_or(u32::MAX);
        self.backoff_base
            .saturating_mul(factor)
            .min(self.backoff_max)
    }
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            auto_restart: true,
            max_retries: 10,
            health_interval: Duration::from_secs(15),
            ready: ReadyMode::Listening,
            startup_grace: Duration::from_secs(25),
            starting_probe: Duration::from_secs(1),
            failed_check_limit: 3,
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            stable_after: Duration::from_secs(60),
        }
    }
}

/// Whether a process exists, from the machine's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proc {
    None,
    Spawning,
    Alive,
    Terminating,
}

/// What to do once the process is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    /// The user stopped it.
    Stop,
    /// Start again with a fresh retry counter (restart, recovery).
    Fresh,
    /// Retry after the backoff delay.
    Backoff,
    /// Stay failed.
    Fail,
}

impl After {
    /// User requests (`Stop`, `Fresh`) win over automatic ones.
    fn merge(old: Option<After>, new: After) -> After {
        match (old, new) {
            (_, After::Stop | After::Fresh) => new,
            (Some(old @ (After::Stop | After::Fresh)), _) => old,
            _ => new,
        }
    }
}

/// One tunnel's state.
#[derive(Clone, Debug)]
pub struct Machine {
    policy: Policy,
    pub state: State,
    pub reason: Option<FailureReason>,
    /// The last error, as an actionable message (FR-48).
    pub last_error: Option<String>,
    /// Consecutive retries; reset after `stable_after` of running (FR-18).
    pub retries: u32,
    /// Automatic restarts since the user started the tunnel.
    pub restarts: u32,
    /// What the user asked for; used to restore tunnels (FR-15).
    pub desired_running: bool,
    failed_checks: u32,
    process: Proc,
    after: Option<After>,
}

impl Machine {
    pub fn new(policy: Policy) -> Machine {
        Machine {
            policy,
            state: State::Stopped,
            reason: None,
            last_error: None,
            retries: 0,
            restarts: 0,
            desired_running: false,
            failed_checks: 0,
            process: Proc::None,
            after: None,
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Replaces the policy, for "restart to apply" (FR-5). Takes effect on the next spawn.
    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
    }

    /// Whether a process may be running.
    pub fn has_process(&self) -> bool {
        self.process != Proc::None
    }

    pub fn step(&mut self, event: Event) -> Vec<Action> {
        let mut acts = Vec::new();
        match event {
            Event::Start => self.on_start(&mut acts),
            Event::Stop => self.on_stop(&mut acts),
            Event::Restart => {
                self.desired_running = true;
                acts.push(Action::CancelTimer(Timer::Backoff));
                acts.push(Action::Log("restart requested".into()));
                self.terminate(After::Fresh, &mut acts);
            }
            Event::Spawned => self.on_spawned(&mut acts),
            Event::SpawnFailed { reason, message } => {
                self.on_spawn_failed(reason, message, &mut acts)
            }
            Event::Exited(how) => self.on_exited(how, &mut acts),
            Event::Probe(result) => self.on_probe(result, &mut acts),
            Event::Output { reason, message } => self.on_output(reason, message, &mut acts),
            Event::Timer(timer) => self.on_timer(timer, &mut acts),
            Event::Terminated => {
                self.process = Proc::None;
                if let Some(after) = self.after.take() {
                    self.after_gone(after, &mut acts);
                }
            }
            Event::AuthExpired(message) => {
                if matches!(
                    self.state,
                    State::Starting | State::Running | State::Unhealthy | State::Reconnecting
                ) {
                    self.fail(FailureReason::AuthExpired, message, &mut acts);
                }
            }
            Event::AuthRecovered => {
                if self.state == State::Failed && self.reason == Some(FailureReason::AuthExpired) {
                    acts.push(Action::Log("login recovered; starting again".into()));
                    self.restart_fresh(&mut acts);
                }
            }
            Event::Wake => self.on_wake(&mut acts),
        }
        acts
    }

    fn on_start(&mut self, acts: &mut Vec<Action>) {
        self.desired_running = true;
        match self.state {
            State::Stopped | State::Failed => self.restart_fresh(acts),
            State::Reconnecting if self.process == Proc::None && self.after.is_none() => {
                // Waiting for a retry: start now instead (FR-14 says no-op for a
                // running tunnel; a waiting one is not running yet).
                acts.push(Action::CancelTimer(Timer::Backoff));
                self.begin_start(acts);
            }
            _ => acts.push(Action::Log(format!(
                "already {}; nothing to do",
                self.state.as_str()
            ))),
        }
    }

    fn on_stop(&mut self, acts: &mut Vec<Action>) {
        self.desired_running = false;
        if self.state == State::Stopped && self.process == Proc::None {
            acts.push(Action::Log("already stopped; nothing to do".into()));
            return;
        }
        acts.push(Action::CancelTimer(Timer::Backoff));
        acts.push(Action::Log("stop requested".into()));
        self.terminate(After::Stop, acts);
    }

    fn on_spawned(&mut self, acts: &mut Vec<Action>) {
        if self.process != Proc::Spawning {
            return;
        }
        self.process = Proc::Alive;
        if self.after.is_some() {
            // A stop or restart arrived while spawning.
            self.process = Proc::Terminating;
            acts.push(Action::Terminate);
            return;
        }
        acts.push(Action::SetTimer(
            Timer::StartupGrace,
            self.policy.startup_grace,
        ));
        match self.policy.ready {
            ReadyMode::Listening => {
                acts.push(Action::SetTimer(Timer::Probe, self.policy.starting_probe))
            }
            ReadyMode::AliveFor(d) => acts.push(Action::SetTimer(Timer::AliveReady, d)),
        }
    }

    fn on_spawn_failed(&mut self, reason: FailureReason, message: String, acts: &mut Vec<Action>) {
        if self.process != Proc::Spawning {
            return;
        }
        self.process = Proc::None;
        acts.push(Action::Log(format!("could not start: {message}")));
        if let Some(after) = self.after.take() {
            self.after_gone(after, acts);
        } else if reason.retryable() {
            self.last_error = Some(message);
            self.state = State::Reconnecting;
            self.schedule_retry(acts);
        } else {
            self.fail(reason, message, acts);
        }
    }

    fn on_exited(&mut self, how: String, acts: &mut Vec<Action>) {
        if self.process != Proc::Alive {
            return;
        }
        acts.push(Action::Log(format!("command {how}")));
        if self.after.is_some() {
            self.process = Proc::Terminating;
            acts.push(Action::Terminate);
            return;
        }
        self.last_error = Some(format!("the command {how}"));
        self.state = State::Reconnecting;
        // Terminate also removes anything the command left behind (design 6.2).
        self.terminate(After::Backoff, acts);
    }

    fn on_probe(&mut self, result: ProbeResult, acts: &mut Vec<Action>) {
        if self.process != Proc::Alive || self.after.is_some() {
            return;
        }
        let interval = self.policy.health_interval;
        match (self.state, result) {
            (State::Starting, ProbeResult::Ready) => {
                self.state = State::Running;
                self.failed_checks = 0;
                self.last_error = None;
                acts.push(Action::CancelTimer(Timer::StartupGrace));
                acts.push(Action::SetTimer(
                    Timer::StableReset,
                    self.policy.stable_after,
                ));
                acts.push(Action::SetTimer(Timer::Probe, interval));
                acts.push(Action::Log("ready".into()));
            }
            (State::Starting, ProbeResult::HeldByOther(message)) => {
                self.fail(FailureReason::PortConflict, message, acts);
            }
            (State::Starting, ProbeResult::Waiting) => {
                acts.push(Action::SetTimer(Timer::Probe, self.policy.starting_probe));
            }
            (State::Starting, ProbeResult::Error(message)) => {
                self.last_error = Some(message);
                acts.push(Action::SetTimer(Timer::Probe, self.policy.starting_probe));
            }
            (State::Running, ProbeResult::Ready) => {
                acts.push(Action::SetTimer(Timer::Probe, interval))
            }
            (State::Running, failed) => {
                self.state = State::Unhealthy;
                self.failed_checks = 1;
                self.last_error = Some(check_failure(&failed));
                acts.push(Action::CancelTimer(Timer::StableReset));
                acts.push(Action::SetTimer(Timer::Probe, interval));
                acts.push(Action::Log(format!(
                    "unhealthy: {}",
                    check_failure(&failed)
                )));
            }
            (State::Unhealthy, ProbeResult::Ready) => {
                self.state = State::Running;
                self.failed_checks = 0;
                self.last_error = None;
                acts.push(Action::SetTimer(
                    Timer::StableReset,
                    self.policy.stable_after,
                ));
                acts.push(Action::SetTimer(Timer::Probe, interval));
                acts.push(Action::Log("healthy again".into()));
            }
            (State::Unhealthy, failed) => {
                self.failed_checks += 1;
                if self.failed_checks >= self.policy.failed_check_limit {
                    self.last_error = Some(format!(
                        "{} ({} checks in a row)",
                        check_failure(&failed),
                        self.failed_checks
                    ));
                    self.state = State::Reconnecting;
                    acts.push(Action::Log(format!(
                        "restarting after {} failed checks",
                        self.failed_checks
                    )));
                    self.terminate(After::Backoff, acts);
                } else {
                    acts.push(Action::SetTimer(Timer::Probe, interval));
                }
            }
            _ => {}
        }
    }

    fn on_output(&mut self, reason: FailureReason, message: String, acts: &mut Vec<Action>) {
        if reason.retryable() || self.after == Some(After::Stop) {
            return;
        }
        let active = matches!(
            self.state,
            State::Starting | State::Running | State::Unhealthy | State::Reconnecting
        );
        if active {
            self.fail(reason, message, acts);
        }
    }

    fn on_timer(&mut self, timer: Timer, acts: &mut Vec<Action>) {
        let alive = self.process == Proc::Alive && self.after.is_none();
        match timer {
            Timer::Probe => {
                let probing = matches!(
                    self.state,
                    State::Starting | State::Running | State::Unhealthy
                );
                if alive && probing && self.policy.ready == ReadyMode::Listening {
                    acts.push(Action::Probe);
                }
            }
            Timer::StartupGrace => {
                if alive && self.state == State::Starting {
                    let secs = self.policy.startup_grace.as_secs();
                    self.last_error = Some(format!("not ready within {secs} seconds"));
                    self.state = State::Reconnecting;
                    acts.push(Action::Log(format!("not ready within {secs} seconds")));
                    self.terminate(After::Backoff, acts);
                }
            }
            Timer::AliveReady => {
                if alive && self.state == State::Starting {
                    self.state = State::Running;
                    self.last_error = None;
                    acts.push(Action::CancelTimer(Timer::StartupGrace));
                    acts.push(Action::SetTimer(
                        Timer::StableReset,
                        self.policy.stable_after,
                    ));
                    acts.push(Action::Log("ready (process stayed up)".into()));
                }
            }
            Timer::StableReset => {
                if self.state == State::Running {
                    self.retries = 0;
                }
            }
            Timer::Backoff => {
                if self.state == State::Reconnecting
                    && self.process == Proc::None
                    && self.after.is_none()
                {
                    self.restarts += 1;
                    self.begin_start(acts);
                }
            }
        }
    }

    fn on_wake(&mut self, acts: &mut Vec<Action>) {
        match self.state {
            State::Failed
                if self.desired_running
                    && self.process == Proc::None
                    && matches!(
                        self.reason,
                        Some(FailureReason::RetriesExhausted | FailureReason::SpawnError)
                    ) =>
            {
                acts.push(Action::Log("woke up; trying again".into()));
                self.restart_fresh(acts);
            }
            State::Reconnecting if self.process == Proc::None && self.after.is_none() => {
                acts.push(Action::CancelTimer(Timer::Backoff));
                acts.push(Action::Log("woke up; reconnecting now".into()));
                self.restarts += 1;
                self.begin_start(acts);
            }
            State::Running | State::Unhealthy if self.process == Proc::Alive => {
                acts.push(Action::Probe)
            }
            _ => {}
        }
    }

    /// Start with a fresh retry counter, once any old process is gone.
    fn restart_fresh(&mut self, acts: &mut Vec<Action>) {
        if self.process == Proc::None && self.after.is_none() {
            self.retries = 0;
            self.reason = None;
            self.last_error = None;
            self.begin_start(acts);
        } else {
            self.terminate(After::Fresh, acts);
        }
    }

    fn begin_start(&mut self, acts: &mut Vec<Action>) {
        self.state = State::Starting;
        self.failed_checks = 0;
        self.process = Proc::Spawning;
        acts.push(Action::Spawn);
    }

    /// Ends the current process (if any), then does `after`.
    fn terminate(&mut self, after: After, acts: &mut Vec<Action>) {
        for timer in [
            Timer::Probe,
            Timer::StartupGrace,
            Timer::AliveReady,
            Timer::StableReset,
        ] {
            acts.push(Action::CancelTimer(timer));
        }
        let after = After::merge(self.after, after);
        match self.process {
            Proc::None => {
                self.after = None;
                self.after_gone(after, acts);
            }
            Proc::Spawning | Proc::Terminating => self.after = Some(after),
            Proc::Alive => {
                self.after = Some(after);
                self.process = Proc::Terminating;
                acts.push(Action::Terminate);
            }
        }
    }

    fn after_gone(&mut self, after: After, acts: &mut Vec<Action>) {
        match after {
            After::Stop => {
                self.state = State::Stopped;
                self.reason = None;
                self.last_error = None;
                self.retries = 0;
                self.restarts = 0;
                acts.push(Action::Log("stopped".into()));
            }
            After::Fresh => {
                self.retries = 0;
                self.reason = None;
                self.last_error = None;
                self.begin_start(acts);
            }
            After::Backoff => self.schedule_retry(acts),
            After::Fail => self.state = State::Failed,
        }
    }

    fn schedule_retry(&mut self, acts: &mut Vec<Action>) {
        if !self.policy.auto_restart {
            self.state = State::Failed;
            self.reason = Some(FailureReason::RetriesExhausted);
            acts.push(Action::Log("not restarting: auto_restart is off".into()));
            return;
        }
        if self.retries >= self.policy.max_retries {
            self.state = State::Failed;
            self.reason = Some(FailureReason::RetriesExhausted);
            acts.push(Action::Log(format!(
                "giving up after {} retries",
                self.policy.max_retries
            )));
            return;
        }
        let delay = self.policy.backoff(self.retries);
        self.retries += 1;
        self.state = State::Reconnecting;
        acts.push(Action::SetTimer(Timer::Backoff, delay));
        acts.push(Action::Log(format!(
            "retry {}/{} in {}",
            self.retries,
            self.policy.max_retries,
            human(delay)
        )));
    }

    fn fail(&mut self, reason: FailureReason, message: String, acts: &mut Vec<Action>) {
        self.state = State::Failed;
        self.reason = Some(reason);
        acts.push(Action::Log(format!("failed ({reason}): {message}")));
        self.last_error = Some(message);
        acts.push(Action::CancelTimer(Timer::Backoff));
        self.terminate(After::Fail, acts);
    }
}

fn check_failure(result: &ProbeResult) -> String {
    match result {
        ProbeResult::Ready => "ready".into(),
        ProbeResult::Waiting => "a local port is no longer listening".into(),
        ProbeResult::HeldByOther(message) | ProbeResult::Error(message) => message.clone(),
    }
}

fn human(d: Duration) -> String {
    if d.as_millis().is_multiple_of(1000) {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}
