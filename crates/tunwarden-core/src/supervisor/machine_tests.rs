use std::time::Duration;

use super::machine::*;
use crate::failure::FailureReason;

fn machine() -> Machine {
    Machine::new(Policy::default())
}

fn has(acts: &[Action], wanted: &Action) -> bool {
    acts.contains(wanted)
}

fn timer(acts: &[Action], t: Timer) -> Option<Duration> {
    acts.iter().rev().find_map(|a| match a {
        Action::SetTimer(x, d) if *x == t => Some(*d),
        _ => None,
    })
}

/// Start, spawn and pass the first probe.
fn running() -> Machine {
    let mut m = machine();
    m.step(Event::Start);
    m.step(Event::Spawned);
    m.step(Event::Probe(ProbeResult::Ready));
    assert_eq!(m.state, State::Running);
    m
}

/// Finishes a requested terminate.
fn terminated(m: &mut Machine) -> Vec<Action> {
    m.step(Event::Terminated)
}

#[test]
fn start_spawns_and_probes_every_second() {
    let mut m = machine();
    let acts = m.step(Event::Start);
    assert_eq!(m.state, State::Starting);
    assert!(has(&acts, &Action::Spawn));
    assert!(m.desired_running);

    let acts = m.step(Event::Spawned);
    assert_eq!(timer(&acts, Timer::Probe), Some(Duration::from_secs(1)));
    assert_eq!(
        timer(&acts, Timer::StartupGrace),
        Some(Duration::from_secs(25))
    );

    let acts = m.step(Event::Timer(Timer::Probe));
    assert!(has(&acts, &Action::Probe));
    let acts = m.step(Event::Probe(ProbeResult::Waiting));
    assert_eq!(m.state, State::Starting);
    assert_eq!(timer(&acts, Timer::Probe), Some(Duration::from_secs(1)));
}

#[test]
fn ready_moves_to_running_and_probes_at_the_interval() {
    let mut m = machine();
    m.step(Event::Start);
    m.step(Event::Spawned);
    let acts = m.step(Event::Probe(ProbeResult::Ready));
    assert_eq!(m.state, State::Running);
    assert!(has(&acts, &Action::CancelTimer(Timer::StartupGrace)));
    assert_eq!(timer(&acts, Timer::Probe), Some(Duration::from_secs(15)));
    assert_eq!(
        timer(&acts, Timer::StableReset),
        Some(Duration::from_secs(60))
    );
}

#[test]
fn starting_twice_is_a_no_op() {
    // FR-14
    let mut m = running();
    let acts = m.step(Event::Start);
    assert_eq!(m.state, State::Running);
    assert!(!has(&acts, &Action::Spawn));
    assert!(matches!(&acts[..], [Action::Log(l)] if l.contains("already running")));
}

#[test]
fn stop_terminates_then_reports_stopped() {
    let mut m = running();
    let acts = m.step(Event::Stop);
    assert!(has(&acts, &Action::Terminate));
    assert!(!m.desired_running);
    assert_eq!(
        m.state,
        State::Running,
        "stays running until the process is gone"
    );
    terminated(&mut m);
    assert_eq!(m.state, State::Stopped);
    assert!(!m.has_process());
}

#[test]
fn stop_while_spawning_terminates_after_spawn() {
    let mut m = machine();
    m.step(Event::Start);
    let acts = m.step(Event::Stop);
    assert!(!has(&acts, &Action::Terminate), "nothing to terminate yet");
    let acts = m.step(Event::Spawned);
    assert!(has(&acts, &Action::Terminate));
    terminated(&mut m);
    assert_eq!(m.state, State::Stopped);
}

#[test]
fn stop_while_waiting_to_retry_cancels_the_retry() {
    let mut m = running();
    m.step(Event::Exited("exited with code 1".into()));
    terminated(&mut m);
    assert_eq!(m.state, State::Reconnecting);
    let acts = m.step(Event::Stop);
    assert!(has(&acts, &Action::CancelTimer(Timer::Backoff)));
    assert_eq!(m.state, State::Stopped);
    let acts = m.step(Event::Timer(Timer::Backoff));
    assert!(acts.is_empty(), "a stale backoff timer does nothing");
}

#[test]
fn exit_retries_with_doubling_backoff() {
    // FR-18: 1 s, 2 s, 4 s ...
    let mut m = running();
    for (i, expected) in [1, 2, 4, 8].into_iter().enumerate() {
        let acts = m.step(Event::Exited("exited with code 1".into()));
        assert_eq!(m.state, State::Reconnecting);
        assert!(
            has(&acts, &Action::Terminate),
            "leftover children are removed"
        );
        let acts = terminated(&mut m);
        assert_eq!(
            timer(&acts, Timer::Backoff),
            Some(Duration::from_secs(expected))
        );
        assert_eq!(m.retries, i as u32 + 1);
        let acts = m.step(Event::Timer(Timer::Backoff));
        assert!(has(&acts, &Action::Spawn));
        assert_eq!(m.state, State::Starting);
        assert_eq!(m.restarts, i as u32 + 1);
        m.step(Event::Spawned);
    }
    assert_eq!(
        m.last_error.as_deref(),
        Some("the command exited with code 1")
    );
}

#[test]
fn backoff_is_capped_at_60_seconds() {
    let p = Policy::default();
    assert_eq!(p.backoff(5), Duration::from_secs(32));
    assert_eq!(p.backoff(6), Duration::from_secs(60));
    assert_eq!(p.backoff(40), Duration::from_secs(60));
}

#[test]
fn retries_exhausted_fails() {
    let mut m = Machine::new(Policy {
        max_retries: 2,
        ..Policy::default()
    });
    m.step(Event::Start);
    for _ in 0..2 {
        m.step(Event::Spawned);
        m.step(Event::Exited("exited with code 1".into()));
        terminated(&mut m);
        m.step(Event::Timer(Timer::Backoff));
    }
    m.step(Event::Spawned);
    m.step(Event::Exited("exited with code 1".into()));
    let acts = terminated(&mut m);
    assert_eq!(m.state, State::Failed);
    assert_eq!(m.reason, Some(FailureReason::RetriesExhausted));
    assert_eq!(timer(&acts, Timer::Backoff), None);
    assert_eq!(
        m.last_error.as_deref(),
        Some("the command exited with code 1")
    );
}

#[test]
fn staying_up_resets_the_retry_counter() {
    let mut m = running();
    m.step(Event::Exited("exited".into()));
    terminated(&mut m);
    m.step(Event::Timer(Timer::Backoff));
    m.step(Event::Spawned);
    m.step(Event::Probe(ProbeResult::Ready));
    assert_eq!(m.retries, 1);
    m.step(Event::Timer(Timer::StableReset));
    assert_eq!(m.retries, 0);
    assert_eq!(m.restarts, 1, "the restart count is kept for status");
}

#[test]
fn auto_restart_off_fails_on_exit() {
    let mut m = Machine::new(Policy {
        auto_restart: false,
        ..Policy::default()
    });
    m.step(Event::Start);
    m.step(Event::Spawned);
    m.step(Event::Exited("exited with code 0".into()));
    terminated(&mut m);
    assert_eq!(m.state, State::Failed);
}

#[test]
fn startup_grace_expiry_reconnects() {
    let mut m = machine();
    m.step(Event::Start);
    m.step(Event::Spawned);
    m.step(Event::Probe(ProbeResult::Waiting));
    let acts = m.step(Event::Timer(Timer::StartupGrace));
    assert_eq!(m.state, State::Reconnecting);
    assert!(has(&acts, &Action::Terminate));
    assert_eq!(m.last_error.as_deref(), Some("not ready within 25 seconds"));
}

#[test]
fn three_failed_checks_restart() {
    // FR-17
    let mut m = running();
    m.step(Event::Probe(ProbeResult::Waiting));
    assert_eq!(m.state, State::Unhealthy);
    m.step(Event::Probe(ProbeResult::Waiting));
    assert_eq!(m.state, State::Unhealthy);
    let acts = m.step(Event::Probe(ProbeResult::Waiting));
    assert_eq!(m.state, State::Reconnecting);
    assert!(has(&acts, &Action::Terminate));
}

#[test]
fn unhealthy_recovers() {
    let mut m = running();
    m.step(Event::Probe(ProbeResult::Waiting));
    m.step(Event::Probe(ProbeResult::Waiting));
    m.step(Event::Probe(ProbeResult::Ready));
    assert_eq!(m.state, State::Running);
    // The count starts over.
    m.step(Event::Probe(ProbeResult::Waiting));
    m.step(Event::Probe(ProbeResult::Waiting));
    assert_eq!(m.state, State::Unhealthy);
}

#[test]
fn port_held_by_another_process_fails_without_retry() {
    let mut m = machine();
    m.step(Event::Start);
    m.step(Event::Spawned);
    let acts = m.step(Event::Probe(ProbeResult::HeldByOther(
        "port 5433 is in use by postgres (PID 812)".into(),
    )));
    assert_eq!(m.state, State::Failed);
    assert_eq!(m.reason, Some(FailureReason::PortConflict));
    assert!(has(&acts, &Action::Terminate));
    let acts = terminated(&mut m);
    assert_eq!(timer(&acts, Timer::Backoff), None);
    assert_eq!(m.state, State::Failed);
}

#[test]
fn credential_output_fails_immediately_and_stops_retrying() {
    // FR-24, AC-3
    let mut m = running();
    let acts = m.step(Event::Output {
        reason: FailureReason::AuthExpired,
        message: "the google login has expired; run `tunwarden login google`".into(),
    });
    assert_eq!(m.state, State::Failed);
    assert_eq!(m.reason, Some(FailureReason::AuthExpired));
    assert!(has(&acts, &Action::Terminate));
    // The process then exits; that must not schedule a retry.
    m.step(Event::Exited("exited with code 1".into()));
    let acts = terminated(&mut m);
    assert_eq!(m.state, State::Failed);
    assert_eq!(timer(&acts, Timer::Backoff), None);
    assert_eq!(
        m.last_error.as_deref(),
        Some("the google login has expired; run `tunwarden login google`")
    );
}

#[test]
fn credential_output_after_exit_cancels_the_pending_retry() {
    let mut m = running();
    m.step(Event::Exited("exited with code 1".into()));
    terminated(&mut m);
    assert_eq!(m.state, State::Reconnecting);
    let acts = m.step(Event::Output {
        reason: FailureReason::HostKey,
        message: "run `tunwarden trust bastion`".into(),
    });
    assert!(has(&acts, &Action::CancelTimer(Timer::Backoff)));
    assert_eq!(m.state, State::Failed);
    assert_eq!(m.reason, Some(FailureReason::HostKey));
}

#[test]
fn output_while_stopping_is_ignored() {
    let mut m = running();
    m.step(Event::Stop);
    m.step(Event::Output {
        reason: FailureReason::Credentials,
        message: "x".into(),
    });
    terminated(&mut m);
    assert_eq!(m.state, State::Stopped);
    assert_eq!(m.reason, None);
}

#[test]
fn spawn_failures() {
    let mut m = machine();
    m.step(Event::Start);
    m.step(Event::SpawnFailed {
        reason: FailureReason::MissingTool,
        message: "ssh was not found".into(),
    });
    assert_eq!(m.state, State::Failed);
    assert_eq!(m.reason, Some(FailureReason::MissingTool));

    let mut m = machine();
    m.step(Event::Start);
    let acts = m.step(Event::SpawnFailed {
        reason: FailureReason::SpawnError,
        message: "permission denied".into(),
    });
    assert_eq!(
        m.state,
        State::Reconnecting,
        "a spawn error may be temporary"
    );
    assert_eq!(timer(&acts, Timer::Backoff), Some(Duration::from_secs(1)));
}

#[test]
fn auth_expiry_and_recovery() {
    // FR-22, FR-25
    let mut m = running();
    m.step(Event::AuthExpired("the google login has expired".into()));
    assert_eq!(m.state, State::Failed);
    terminated(&mut m);
    assert_eq!(m.reason, Some(FailureReason::AuthExpired));
    let acts = m.step(Event::AuthRecovered);
    assert!(has(&acts, &Action::Spawn));
    assert_eq!(m.state, State::Starting);
    assert_eq!(m.reason, None);
    assert_eq!(m.retries, 0);
}

#[test]
fn auth_recovery_ignores_other_failures_and_stopped_tunnels() {
    let mut m = machine();
    assert!(m.step(Event::AuthRecovered).is_empty());
    m.step(Event::Start);
    m.step(Event::SpawnFailed {
        reason: FailureReason::MissingTool,
        message: "x".into(),
    });
    assert!(m.step(Event::AuthRecovered).is_empty());
    assert!(m.step(Event::AuthExpired("x".into())).is_empty());
}

#[test]
fn start_after_failure_starts_fresh() {
    let mut m = running();
    m.step(Event::Output {
        reason: FailureReason::Credentials,
        message: "x".into(),
    });
    terminated(&mut m);
    let acts = m.step(Event::Start);
    assert!(has(&acts, &Action::Spawn));
    assert_eq!(m.reason, None);
}

#[test]
fn start_while_failed_and_still_terminating_waits_for_the_old_process() {
    let mut m = running();
    m.step(Event::Output {
        reason: FailureReason::Credentials,
        message: "x".into(),
    });
    let acts = m.step(Event::Start);
    assert!(!has(&acts, &Action::Spawn));
    let acts = terminated(&mut m);
    assert!(has(&acts, &Action::Spawn));
    assert_eq!(m.state, State::Starting);
}

#[test]
fn restart_terminates_then_starts_fresh() {
    let mut m = running();
    m.retries = 3;
    let acts = m.step(Event::Restart);
    assert!(has(&acts, &Action::Terminate));
    let acts = terminated(&mut m);
    assert!(has(&acts, &Action::Spawn));
    assert_eq!(m.retries, 0);

    // From stopped it simply starts.
    let mut m = machine();
    assert!(has(&m.step(Event::Restart), &Action::Spawn));
}

#[test]
fn stop_wins_over_a_pending_restart() {
    let mut m = running();
    m.step(Event::Restart);
    m.step(Event::Stop);
    terminated(&mut m);
    assert_eq!(m.state, State::Stopped);
}

#[test]
fn process_alive_readiness() {
    // FR-44
    let mut m = Machine::new(Policy {
        ready: ReadyMode::AliveFor(Duration::from_secs(5)),
        ..Policy::default()
    });
    m.step(Event::Start);
    let acts = m.step(Event::Spawned);
    assert_eq!(
        timer(&acts, Timer::AliveReady),
        Some(Duration::from_secs(5))
    );
    assert_eq!(timer(&acts, Timer::Probe), None);
    assert!(
        m.step(Event::Timer(Timer::Probe)).is_empty(),
        "no socket checks"
    );
    m.step(Event::Timer(Timer::AliveReady));
    assert_eq!(m.state, State::Running);
}

#[test]
fn wake_retries_now_and_revives_non_credential_failures() {
    // FR-19
    let mut m = running();
    m.step(Event::Exited("exited".into()));
    terminated(&mut m);
    let acts = m.step(Event::Wake);
    assert!(has(&acts, &Action::CancelTimer(Timer::Backoff)));
    assert!(has(&acts, &Action::Spawn));

    let mut m = Machine::new(Policy {
        max_retries: 0,
        ..Policy::default()
    });
    m.step(Event::Start);
    m.step(Event::Spawned);
    m.step(Event::Exited("exited".into()));
    terminated(&mut m);
    assert_eq!(m.reason, Some(FailureReason::RetriesExhausted));
    assert!(has(&m.step(Event::Wake), &Action::Spawn));

    let mut m = running();
    m.step(Event::Output {
        reason: FailureReason::AuthExpired,
        message: "x".into(),
    });
    terminated(&mut m);
    assert!(
        m.step(Event::Wake).is_empty(),
        "credential failures wait for the user"
    );

    let mut m = running();
    assert!(
        has(&m.step(Event::Wake), &Action::Probe),
        "a running tunnel is checked now"
    );
}

#[test]
fn stale_events_are_ignored() {
    let mut m = machine();
    assert!(m.step(Event::Spawned).is_empty());
    assert!(m.step(Event::Exited("x".into())).is_empty());
    assert!(m.step(Event::Probe(ProbeResult::Ready)).is_empty());
    assert_eq!(m.state, State::Stopped);
}
