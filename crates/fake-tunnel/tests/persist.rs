//! A supervisor running a real `fake-tunnel`, persisting through `Persister`:
//! the tunnel log, the activity log and the state file (FR-27 to FR-30, design 8.2).

use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tunwarden_core::config;
use tunwarden_core::failure::FailureReason;
use tunwarden_core::logs::{Activity, DEFAULT_MAX_BYTES, Logs};
use tunwarden_core::paths::SystemEnv;
use tunwarden_core::persist::Persister;
use tunwarden_core::probe::Prober;
use tunwarden_core::state::{Desired, StateStore};
use tunwarden_core::supervisor::{Event, Spec, State, Supervisor};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

async fn until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn logs_activity_and_state_are_written() {
    let state_dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let src = format!(
        "version = 1\n[[tunnels]]\nname = \"db\"\n\
         command = \"'{FAKE}' --listen 127.0.0.1:{port} --print-stdout 'hello from stdout' --print 'hello from stderr'\"\n\
         forwards = [{{ local_port = {port} }}]\n"
    );
    let c = config::parse(&src, Path::new("t.toml"), &SystemEnv).unwrap();
    let mut spec = Spec::new(&c.tunnels[0], &c);
    spec.policy.starting_probe = Duration::from_millis(50);

    let (store, previous) = StateStore::open(state_dir.path()).unwrap();
    assert!(previous.state.tunnels.is_empty());
    let persister = Arc::new(Persister::new(
        Logs::new(state_dir.path(), DEFAULT_MAX_BYTES).unwrap(),
        Activity::new(state_dir.path(), DEFAULT_MAX_BYTES).unwrap(),
        store,
    ));
    let sup = Supervisor::spawn(spec, Arc::new(Prober::new()), persister.clone());

    sup.send(Event::Start);
    let tail = || persister.logs.tail("db", 50).unwrap().join("\n");
    until("running", || sup.status().state == State::Running).await;
    until("output in the log", || {
        tail().contains("err hello from stderr")
    })
    .await;

    let log = tail();
    assert!(log.contains(" out hello from stdout"), "{log}");
    assert!(log.contains(" tw started PID "), "{log}");
    assert!(log.contains(" tw stopped -> starting"), "{log}");
    assert!(log.contains(" tw starting -> running"), "{log}");

    // The state file shows the running process.
    let pid = sup.status().process.unwrap().pid;
    let saved = persister.state.snapshot();
    let db = &saved.tunnels["db"];
    assert_eq!((db.desired, db.state), (Desired::Running, State::Running));
    assert_eq!(db.process.unwrap().pid, pid);

    sup.send(Event::Stop);
    until("stopped", || sup.status().state == State::Stopped).await;
    sup.shutdown().await;

    // A new daemon reads it back.
    let (_, previous) = StateStore::open(state_dir.path()).unwrap();
    let db = &previous.state.tunnels["db"];
    assert_eq!(
        (db.desired, db.state, db.process),
        (Desired::Stopped, State::Stopped, None)
    );

    let activity: Vec<(State, State)> = persister
        .activity
        .recent(10)
        .iter()
        .map(|e| (e.from, e.to))
        .collect();
    assert_eq!(
        activity,
        [
            (State::Stopped, State::Starting),
            (State::Starting, State::Running),
            (State::Running, State::Stopped)
        ]
    );
    assert_eq!(persister.last_error(), None);
    let log_path = state_dir.path().join("logs").join("db.log");
    assert!(log_path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "logs are readable only by the user");
    }
}

#[tokio::test]
async fn a_failure_reason_reaches_activity_and_state() {
    let state_dir = tempfile::tempdir().unwrap();
    let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    let src = format!(
        "version = 1\n[[tunnels]]\nname = \"db\"\ncommand = \"'{FAKE}' --listen 127.0.0.1:{port}\"\nforwards = [{{ local_port = {port} }}]\n"
    );
    let c = config::parse(&src, Path::new("t.toml"), &SystemEnv).unwrap();
    let (store, _) = StateStore::open(state_dir.path()).unwrap();
    let persister = Arc::new(Persister::new(
        Logs::new(state_dir.path(), DEFAULT_MAX_BYTES).unwrap(),
        Activity::new(state_dir.path(), DEFAULT_MAX_BYTES).unwrap(),
        store,
    ));
    let sup = Supervisor::spawn(
        Spec::new(&c.tunnels[0], &c),
        Arc::new(Prober::new()),
        persister.clone(),
    );
    sup.send(Event::Start);
    until("failed", || sup.status().state == State::Failed).await;
    sup.shutdown().await;

    let last = persister.activity.recent(1).pop().unwrap();
    assert_eq!(last.to, State::Failed);
    assert_eq!(last.reason, Some(FailureReason::PortConflict));
    assert!(last.message.unwrap().contains(&format!("127.0.0.1:{port}")));
    let text = std::fs::read_to_string(state_dir.path().join("state.json")).unwrap();
    assert!(text.contains(r#""reason": "port conflict""#), "{text}");
    drop(squatter);
}
