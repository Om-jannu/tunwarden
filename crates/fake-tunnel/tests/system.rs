//! Wake-ups and network changes reach the tunnels (PRD FR-19, AC-7). The
//! test sends the events itself through `Options::system_events`, because the
//! real watcher only sees a real sleep.
#![cfg(unix)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tunwarden_core::client::{Client, ClientError};
use tunwarden_core::daemon;
use tunwarden_core::failure::FailureReason;
use tunwarden_core::proto::*;
use tunwarden_core::supervisor::State;
use tunwarden_core::system::SystemEvent;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Starts the listener only once `ready` exists; exits with an error before.
const GATED: &str = r#"#!/bin/sh
dir=$(dirname "$0")
[ -f "$dir/ready" ] || { echo "not ready yet" >&2; exit 1; }
exec "$@"
"#;

/// A login check that records each call.
const CHECK: &str = r#"#!/bin/sh
echo check >> "$(dirname "$0")/checks.log"
exit 0
"#;

struct Setup {
    dir: tempfile::TempDir,
    port: u16,
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

impl Setup {
    /// `gated` fails until the `ready` file exists, with one retry, so it
    /// ends `failed` with `retries exhausted`. `watched` uses a login.
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let gated = script(dir.path(), "gated", GATED);
        let check = script(dir.path(), "check", CHECK);
        let port = free_port();
        let watched = free_port();
        std::fs::write(
            dir.path().join("config.toml"),
            format!(
                "version = 1\n\
                 [[logins]]\nname = \"work\"\ncheck = \"'{}'\"\n\
                 [[profiles]]\nname = \"p\"\nlogin = \"work\"\n\
                 [[tunnels]]\nname = \"gated\"\nmax_retries = 1\n\
                 command = \"'{}' '{FAKE}' --listen 127.0.0.1:{port}\"\nforwards = [{{ local_port = {port} }}]\n\
                 [[tunnels]]\nname = \"watched\"\nprofile = \"p\"\n\
                 command = \"'{FAKE}' --listen 127.0.0.1:{watched}\"\nforwards = [{{ local_port = {watched} }}]\n",
                check.display(),
                gated.display()
            ),
        )
        .unwrap();
        Setup { dir, port }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn checks(&self) -> usize {
        std::fs::read_to_string(self.path("checks.log"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn daemon(
        &self,
        events: mpsc::UnboundedReceiver<SystemEvent>,
    ) -> tokio::task::JoinHandle<Result<(), daemon::DaemonError>> {
        tokio::spawn(daemon::run(daemon::Options {
            config_path: self.path("config.toml"),
            state_dir: self.path("state"),
            handle_signals: false,
            // The scheduled login check never runs during the test.
            auth_interval: Some(Duration::from_secs(3600)),
            system_events: Some(events),
        }))
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn connect(state: &Path) -> Client {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Client::connect(state, "test").await {
            Ok(c) => return c,
            Err(ClientError::NotRunning) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(e) => panic!("{e}"),
        }
    }
}

async fn view(c: &mut Client, name: &str) -> TunnelView {
    let s: StatusResult = c.call(method::STATUS, json!({})).await.unwrap();
    s.tunnels.into_iter().find(|t| t.name == name).unwrap()
}

async fn wait(c: &mut Client, name: &str, what: &str, f: impl Fn(&TunnelView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let v = view(c, name).await;
        if f(&v) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {v:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn start(c: &mut Client, name: &str) {
    let _: ActionResult = c
        .call(
            method::START,
            Selector {
                names: vec![name.into()],
                ..Selector::default()
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_wake_retries_failed_tunnels_and_checks_logins() {
    let _turn = TURN.lock().await;
    let s = Setup::new();
    let (events, rx) = mpsc::unbounded_channel();
    let daemon = s.daemon(rx);
    let mut c = connect(&s.path("state")).await;

    start(&mut c, "watched").await;
    wait(&mut c, "watched", "watched running", |t| {
        t.state == State::Running
    })
    .await;
    start(&mut c, "gated").await;
    wait(&mut c, "gated", "retries exhausted", |t| {
        t.state == State::Failed && t.reason == Some(FailureReason::RetriesExhausted)
    })
    .await;
    assert_eq!(s.checks(), 0, "no login check yet");

    // The network comes back: the tunnel gets fresh retries and starts.
    std::fs::write(s.path("ready"), "").unwrap();
    events.send(SystemEvent::NetworkChanged).unwrap();
    wait(&mut c, "gated", "gated running", |t| {
        t.state == State::Running
    })
    .await;
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", s.port)).is_ok(),
        "it really listens"
    );
    assert_eq!(s.checks(), 0, "a network change does not check logins");

    // A wake checks the logins that active tunnels use.
    events
        .send(SystemEvent::Woke {
            slept: Duration::from_secs(8 * 3600),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.checks() == 0 {
        assert!(Instant::now() < deadline, "the login was never checked");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let auth: AuthResult = c
        .call(method::AUTH_STATUS, AuthParams::default())
        .await
        .unwrap();
    assert_eq!(auth.logins[0].result, "ok");
    assert_eq!(view(&mut c, "gated").await.state, State::Running);
    assert_eq!(view(&mut c, "watched").await.state, State::Running);

    let _: Value = c
        .call(method::SHUTDOWN, ShutdownParams::default())
        .await
        .unwrap();
    daemon.await.unwrap().unwrap();
}
