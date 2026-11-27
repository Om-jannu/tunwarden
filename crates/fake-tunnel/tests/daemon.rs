//! System tests for the daemon, the protocol and the client (design 5,
//! PRD FR-37, FR-38, FR-52, FR-53, FR-15, AC-6).

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tunwarden_core::client::{Client, ClientError, Launcher};
use tunwarden_core::daemon::{self, DaemonError};
use tunwarden_core::ipc;
use tunwarden_core::proto::*;
use tunwarden_core::state::Desired;
use tunwarden_core::supervisor::State;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");
const TEST_DAEMON: &str = env!("CARGO_BIN_EXE_tw-test-daemon");

/// Port tests take turns, so a port freed by one test is not grabbed by another.
static PORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Setup {
    _dir: tempfile::TempDir,
    config: PathBuf,
    state: PathBuf,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A config with two listening fake tunnels, `a` (group `daily`) and `b`.
fn setup(extra_args: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let (pa, pb) = (free_port(), free_port());
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "version = 1\n\
             [[tunnels]]\nname = \"a\"\ngroups = [\"daily\"]\ncommand = \"'{FAKE}' --listen 127.0.0.1:{pa} {extra_args}\"\nforwards = [{{ local_port = {pa} }}]\n\
             [[tunnels]]\nname = \"b\"\ncommand = \"'{FAKE}' --listen 127.0.0.1:{pb} {extra_args}\"\nforwards = [{{ local_port = {pb} }}]\n"
        ),
    )
    .unwrap();
    let state = dir.path().join("state");
    Setup {
        _dir: dir,
        config,
        state,
    }
}

fn in_process(s: &Setup) -> tokio::task::JoinHandle<Result<(), DaemonError>> {
    let opts = daemon::Options {
        config_path: s.config.clone(),
        state_dir: s.state.clone(),
        handle_signals: false,
        auth_interval: None,
    };
    tokio::spawn(daemon::run(opts))
}

async fn connect(state: &Path) -> Client {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Client::connect(state, "test").await {
            Ok(c) => return c,
            Err(ClientError::NotRunning) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(e) => panic!("cannot connect: {e}"),
        }
    }
}

async fn status(c: &mut Client) -> StatusResult {
    c.call(method::STATUS, json!({})).await.unwrap()
}

async fn wait_states(
    c: &mut Client,
    what: &str,
    f: impl Fn(&[TunnelView]) -> bool,
) -> Vec<TunnelView> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = status(c).await;
        if f(&s.tunnels) {
            return s.tunnels;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:#?}",
            s.tunnels
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn all_in(state: State) -> impl Fn(&[TunnelView]) -> bool {
    move |ts| ts.iter().all(|t| t.state == state)
}

async fn shutdown(c: &mut Client, keep: bool) {
    let _: Value = c
        .call(method::SHUTDOWN, ShutdownParams { keep })
        .await
        .unwrap();
}

#[tokio::test]
async fn lifecycle_over_the_protocol() {
    let _turn = PORTS.lock().await;
    let s = setup("");
    let daemon = in_process(&s);
    let mut c = connect(&s.state).await;

    let hello = c.hello().clone();
    assert_eq!(hello.protocol, PROTOCOL);
    assert_eq!(hello.pid, std::process::id(), "in-process daemon");
    assert_eq!(hello.config_path, s.config.display().to_string());

    let st = status(&mut c).await;
    assert!(st.config_errors.is_empty() && st.notes.is_empty(), "{st:?}");
    assert_eq!(
        st.tunnels
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(
        st.tunnels
            .iter()
            .all(|t| t.state == State::Stopped && t.desired == Desired::Stopped)
    );

    let list: ListResult = c.call(method::LIST, json!({})).await.unwrap();
    assert!(
        list.tunnels[0].command.contains("--listen 127.0.0.1:"),
        "{:?}",
        list.tunnels[0]
    );

    // AC-1: one command starts everything.
    let started: ActionResult = c
        .call(
            method::START,
            Selector {
                all: true,
                ..Selector::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(started.tunnels, ["a", "b"]);
    let running = wait_states(&mut c, "both running", all_in(State::Running)).await;
    assert!(
        running
            .iter()
            .all(|t| t.pid.is_some() && t.desired == Desired::Running)
    );

    // Selectors.
    let stopped: ActionResult = c
        .call(
            method::STOP,
            Selector {
                group: Some("daily".into()),
                ..Selector::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.tunnels, ["a"]);
    wait_states(&mut c, "a stopped", |ts| {
        ts[0].state == State::Stopped && ts[1].state == State::Running
    })
    .await;

    let logs: LogsResult = c
        .call(
            method::LOGS_TAIL,
            LogsParams {
                name: "a".into(),
                lines: 50,
            },
        )
        .await
        .unwrap();
    assert!(
        logs.lines.iter().any(|l| l.contains(" tw started PID ")),
        "{:?}",
        logs.lines
    );
    let activity: ActivityResult = c
        .call(method::ACTIVITY, ActivityParams { limit: 50 })
        .await
        .unwrap();
    assert!(
        activity
            .events
            .iter()
            .any(|e| e.tunnel == "a" && e.to == State::Stopped)
    );

    // Errors carry a stable code and a hint.
    let e = c
        .call::<_, ActionResult>(
            method::START,
            Selector {
                names: vec!["nope".into()],
                ..Selector::default()
            },
        )
        .await
        .unwrap_err();
    let ClientError::Rpc(e) = e else {
        panic!("{e}")
    };
    assert_eq!(e.kind(), Some("unknown_tunnel"));
    assert_eq!(e.hint(), Some("configured tunnels: a, b"));
    let e = c
        .call::<_, ActionResult>(method::START, Selector::default())
        .await
        .unwrap_err();
    assert!(
        matches!(e, ClientError::Rpc(ref r) if r.kind() == Some("no_selection")),
        "{e}"
    );
    let e = c
        .call::<_, Value>("no.such.method", json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(e, ClientError::Rpc(ref r) if r.code == RPC_UNKNOWN_METHOD),
        "{e}"
    );

    shutdown(&mut c, false).await;
    daemon.await.unwrap().unwrap();

    // A clean stop without --keep records nothing to restore.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(s.state.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["daemon"], Value::Null);
    assert_eq!(saved["tunnels"]["b"]["desired"], "stopped");
    assert_eq!(saved["tunnels"]["b"]["state"], "stopped");
    assert!(matches!(
        Client::connect(&s.state, "test").await,
        Err(ClientError::NotRunning)
    ));
}

#[tokio::test]
async fn subscribers_get_state_changes_and_log_lines() {
    let _turn = PORTS.lock().await;
    let s = setup("--print-stdout hello-from-a");
    let daemon = in_process(&s);
    let mut watcher = connect(&s.state).await;
    let _: Value = watcher
        .call(
            method::SUBSCRIBE,
            SubscribeParams {
                topics: vec!["tunnels".into(), "activity".into(), "log:a".into()],
            },
        )
        .await
        .unwrap();
    let mut control = connect(&s.state).await;
    let _: ActionResult = control
        .call(
            method::START,
            Selector {
                names: vec!["a".into()],
                ..Selector::default()
            },
        )
        .await
        .unwrap();

    let (mut saw_running, mut saw_activity, mut saw_log) = (false, false, false);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(saw_running && saw_activity && saw_log) {
        assert!(
            Instant::now() < deadline,
            "running {saw_running}, activity {saw_activity}, log {saw_log}"
        );
        let n = tokio::time::timeout(Duration::from_secs(10), watcher.next_notification())
            .await
            .unwrap()
            .unwrap();
        match n.method.as_str() {
            notice::TUNNEL_STATE => {
                let v: TunnelView = serde_json::from_value(n.params).unwrap();
                assert_eq!(v.name, "a", "b was not started");
                saw_running |= v.state == State::Running;
            }
            notice::ACTIVITY => saw_activity = true,
            notice::LOG => {
                let l: LogNotice = serde_json::from_value(n.params).unwrap();
                saw_log |= l.line.ends_with(" out hello-from-a");
            }
            other => panic!("unexpected notification {other}"),
        }
    }
    shutdown(&mut control, false).await;
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn handshake_and_version_mismatch() {
    // FR-53
    let s = setup("");
    let daemon = in_process(&s);
    let mut c = connect(&s.state).await; // waits until it listens

    let raw = ipc::connect(&ipc::endpoint(&s.state)).await.unwrap();
    let (read, mut write) = tokio::io::split(raw);
    let mut lines = BufReader::new(read).lines();
    let mut ask = async |req: Value| -> Value {
        write
            .write_all(format!("{req}\n").as_bytes())
            .await
            .unwrap();
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
    };

    let r = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "status"})).await;
    assert_eq!(r["error"]["data"]["code"], "handshake_required");

    let r = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "hello", "params": {"protocol": 99, "client": "app", "version": "9.0.0"}})).await;
    assert_eq!(r["error"]["code"], RPC_PROTOCOL_MISMATCH);
    assert_eq!(r["error"]["data"]["code"], "protocol_mismatch");
    assert_eq!(
        r["error"]["data"]["hint"],
        "run `tunwarden daemon restart` to start the installed version"
    );
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("speaks protocol 1, but this app (Tunwarden 9.0.0) speaks protocol 99")
    );

    let r = ask(json!("not a request")).await;
    assert_eq!(r["error"]["code"], RPC_PARSE_ERROR);

    shutdown(&mut c, false).await;
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn only_one_daemon_per_state_dir() {
    let s = setup("");
    let first = in_process(&s);
    let mut c = connect(&s.state).await;
    let second = in_process(&s).await.unwrap();
    assert!(
        matches!(second, Err(DaemonError::AlreadyRunning(_))),
        "{second:?}"
    );
    shutdown(&mut c, false).await;
    first.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_invalid_config_is_reported_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "version = 1\n[[tunnels]]\nname = \"bad name\"\ncommand = \"x\"\n",
    )
    .unwrap();
    let s = Setup {
        config,
        state: dir.path().join("state"),
        _dir: dir,
    };
    let daemon = in_process(&s);
    let mut c = connect(&s.state).await;
    let st = status(&mut c).await;
    assert!(st.tunnels.is_empty());
    assert_eq!(st.config_errors.len(), 1);
    assert!(
        st.config_errors[0].contains("tunnels[0].name: tunnel name \"bad name\" contains ' '"),
        "{:?}",
        st.config_errors
    );
    shutdown(&mut c, false).await;
    daemon.await.unwrap().unwrap();

    std::fs::remove_file(&s.config).unwrap();
    let daemon = in_process(&s);
    let mut c = connect(&s.state).await;
    let st = status(&mut c).await;
    assert!(
        st.config_errors[0].contains("run `tunwarden init`"),
        "{:?}",
        st.config_errors
    );
    shutdown(&mut c, false).await;
    daemon.await.unwrap().unwrap();
}

fn launcher(s: &Setup) -> Launcher {
    Launcher {
        program: TEST_DAEMON.into(),
        args: vec!["--config".into(), s.config.clone().into_os_string()],
    }
}

#[tokio::test]
async fn starts_on_demand_and_keep_restores_tunnels() {
    // FR-15: `stop --keep` restores, a plain stop does not.
    let _turn = PORTS.lock().await;
    let s = setup("");
    let mut c = Client::connect_or_start(&s.state, "test", &launcher(&s))
        .await
        .unwrap();
    let first_pid = c.hello().pid;
    assert_ne!(first_pid, std::process::id(), "a separate daemon process");
    let _: ActionResult = c
        .call(
            method::START,
            Selector {
                names: vec!["a".into()],
                ..Selector::default()
            },
        )
        .await
        .unwrap();
    wait_states(&mut c, "a running", |ts| ts[0].state == State::Running).await;

    shutdown(&mut c, true).await;
    drop(c);
    let mut c = start_again(&s).await;
    let ts = wait_states(&mut c, "a restored", |ts| ts[0].state == State::Running).await;
    assert_eq!(ts[1].state, State::Stopped, "b was never started");
    assert_eq!(ts[0].note, None, "a clean --keep restart needs no note");

    shutdown(&mut c, false).await;
    drop(c);
    let mut c = start_again(&s).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        all_in(State::Stopped)(&status(&mut c).await.tunnels),
        "nothing restored after a plain stop"
    );
    shutdown(&mut c, false).await;
    assert!(
        std::fs::read_to_string(s.state.join("daemon.log"))
            .unwrap()
            .contains("listening on")
    );
}

/// Waits for the old daemon to exit, then starts a new one.
async fn start_again(s: &Setup) -> Client {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match Client::connect(&s.state, "test").await {
            Err(ClientError::NotRunning) => break,
            _ => {
                assert!(Instant::now() < deadline, "the old daemon did not exit");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    Client::connect_or_start(&s.state, "test", &launcher(s))
        .await
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_crashed_daemon_is_cleaned_up_and_restored() {
    // AC-6 and FR-52: kill -9 the daemon; its tunnels keep running, so the next
    // daemon must remove them and start fresh ones.
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    let _turn = PORTS.lock().await;
    let s = setup("--spawn-child");
    let mut c = Client::connect_or_start(&s.state, "test", &launcher(&s))
        .await
        .unwrap();
    let daemon_pid = c.hello().pid;
    let _: ActionResult = c
        .call(
            method::START,
            Selector {
                all: true,
                ..Selector::default()
            },
        )
        .await
        .unwrap();
    let before = wait_states(&mut c, "running", all_in(State::Running)).await;
    let old: Vec<u32> = before.iter().map(|t| t.pid.unwrap()).collect();
    let logs: LogsResult = c
        .call(
            method::LOGS_TAIL,
            LogsParams {
                name: "a".into(),
                lines: 50,
            },
        )
        .await
        .unwrap();
    let child: i32 = logs
        .lines
        .iter()
        .find_map(|l| l.split(" out child ").nth(1))
        .expect("fake-tunnel prints its child PID")
        .parse()
        .unwrap();
    drop(c);

    kill(Pid::from_raw(daemon_pid as i32), Signal::SIGKILL).unwrap();
    let alive = |pid: i32| {
        kill(Pid::from_raw(pid), None).is_ok()
            && tunwarden_core::process::start_time(pid as u32).is_some()
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        old.iter().all(|p| alive(*p as i32)) && alive(child),
        "tunnels outlive a killed daemon"
    );

    let mut c = start_again(&s).await;
    let after = wait_states(&mut c, "restored", all_in(State::Running)).await;
    for (t, old_pid) in after.iter().zip(&old) {
        assert_ne!(t.pid, Some(*old_pid), "a fresh process for {}", t.name);
        assert_eq!(
            t.note.as_deref(),
            Some("restored after the previous daemon stopped unexpectedly")
        );
    }
    let st = status(&mut c).await;
    assert!(
        st.notes.iter().any(|n| n.contains("stopped unexpectedly")),
        "{:?}",
        st.notes
    );
    assert!(
        old.iter().all(|p| !alive(*p as i32)),
        "old tunnel processes were swept"
    );
    assert!(!alive(child), "the old grandchild was swept too");
    let logs: LogsResult = c
        .call(
            method::LOGS_TAIL,
            LogsParams {
                name: "a".into(),
                lines: 100,
            },
        )
        .await
        .unwrap();
    assert!(
        logs.lines
            .iter()
            .any(|l| l.contains("removed processes left by the previous daemon"))
    );

    shutdown(&mut c, false).await;
}

#[tokio::test]
async fn no_daemon_means_not_running() {
    let s = setup("");
    let e = Client::connect(&s.state, "test").await.err().unwrap();
    assert!(matches!(e, ClientError::NotRunning));
    assert_eq!(e.hint(), Some("start it with `tunwarden daemon start`"));
}
