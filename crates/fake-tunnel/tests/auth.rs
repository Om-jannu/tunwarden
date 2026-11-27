//! Login handling with a fake login tool (PRD 7.5, AC-3, FR-25).
//!
//! The fake is a shell script with `status` and `login` commands. Marker
//! files in its directory decide what it answers: `expired-<account>` means
//! that account's login has expired, and `network-down` makes every check
//! fail with a network error.
#![cfg(unix)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tunwarden_core::client::{Client, ClientError};
use tunwarden_core::daemon;
use tunwarden_core::failure::FailureReason;
use tunwarden_core::proto::*;
use tunwarden_core::supervisor::State;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");
const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");

static PORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FAKE_TOOL: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo "$* | account=${TOOL_ACCOUNT:-} project=${TOOL_PROJECT:-}" >> "$dir/calls.log"
account=${TOOL_ACCOUNT:-default}
case "$1" in
  status)
    echo "token: secret-token-value"
    if [ -f "$dir/network-down" ]; then
      echo "ERROR: connection to auth.example.com timed out" >&2
      exit 1
    fi
    if [ -f "$dir/expired-$account" ]; then
      echo "ERROR: Reauthentication failed. cannot prompt during non-interactive execution." >&2
      exit 1
    fi
    exit 0 ;;
  login)
    rm -f "$dir/expired-$account"
    echo "You are now logged in as [$account]."
    exit 0 ;;
esac
echo "fake tool: unsupported: $*" >&2
exit 2
"#;

struct Setup {
    dir: tempfile::TempDir,
}

impl Setup {
    /// Profiles `p` and `q` share login `a`; profile `r` uses login `b`.
    /// Tunnels `tp`, `tq` and `tr` use those profiles; `plain` has none.
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("faketool");
        std::fs::write(&tool, FAKE_TOOL).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut tunnels = String::new();
        for (name, profile) in [
            ("tp", Some("p")),
            ("tq", Some("q")),
            ("tr", Some("r")),
            ("plain", None),
        ] {
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            tunnels.push_str(&format!(
                "[[tunnels]]\nname = \"{name}\"\ncommand = \"'{FAKE}' --listen 127.0.0.1:{port}\"\nforwards = [{{ local_port = {port} }}]\n"
            ));
            if let Some(p) = profile {
                tunnels.push_str(&format!("profile = \"{p}\"\n"));
            }
        }
        let mut logins = String::new();
        for (name, account) in [("a", "a@example.com"), ("b", "b@example.com")] {
            logins.push_str(&format!(
                "[[logins]]\nname = \"{name}\"\ncheck = \"faketool status\"\nlogin = \"faketool login\"\n\
                 expired = [\"Reauthentication failed\"]\nenv = {{ TOOL_ACCOUNT = \"{account}\" }}\n"
            ));
        }
        let config = format!(
            "version = 1\n[tools]\npath = [\"{}\"]\n{logins}\
             [[profiles]]\nname = \"p\"\nlogin = \"a\"\nenv = {{ TOOL_PROJECT = \"example-p\" }}\n\
             [[profiles]]\nname = \"q\"\nlogin = \"a\"\nenv = {{ TOOL_PROJECT = \"example-q\" }}\n\
             [[profiles]]\nname = \"r\"\nlogin = \"b\"\nenv = {{ TOOL_PROJECT = \"example-r\" }}\n{tunnels}",
            bin.display()
        );
        std::fs::write(dir.path().join("config.toml"), config).unwrap();
        Setup { dir }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn marker(&self, name: &str) -> PathBuf {
        self.dir.path().join("bin").join(name)
    }

    fn set(&self, name: &str, on: bool) {
        if on {
            std::fs::write(self.marker(name), "").unwrap();
        } else {
            let _ = std::fs::remove_file(self.marker(name));
        }
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.marker("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn daemon(
        &self,
        interval: Duration,
    ) -> tokio::task::JoinHandle<Result<(), daemon::DaemonError>> {
        tokio::spawn(daemon::run(daemon::Options {
            config_path: self.config(),
            state_dir: self.state(),
            handle_signals: false,
            auth_interval: Some(interval),
            system_events: None,
        }))
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(CLI)
            .arg("--config")
            .arg(self.config())
            .args(args)
            .env("TUNWARDEN_STATE_DIR", self.state())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
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

async fn views(c: &mut Client) -> Vec<TunnelView> {
    let s: StatusResult = c.call(method::STATUS, json!({})).await.unwrap();
    s.tunnels
}

async fn wait(c: &mut Client, what: &str, f: impl Fn(&[TunnelView]) -> bool) -> Vec<TunnelView> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let v = views(c).await;
        if f(&v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {v:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn get<'a>(v: &'a [TunnelView], name: &str) -> &'a TunnelView {
    v.iter().find(|t| t.name == name).unwrap()
}

async fn start_all(c: &mut Client) {
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
    wait(c, "all running", |v| {
        v.iter().all(|t| t.state == State::Running)
    })
    .await;
}

#[tokio::test]
async fn an_expired_login_fails_its_tunnels_and_one_login_recovers_both_profiles() {
    let _turn = PORTS.lock().await;
    let s = Setup::new();
    let daemon = s.daemon(Duration::from_millis(300));
    let mut c = connect(&s.state()).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        s.calls().is_empty(),
        "no checks while no tunnel uses a login: {:?}",
        s.calls()
    );

    start_all(&mut c).await;
    s.set("expired-a@example.com", true);

    // Both profiles of login a fail; b and the tunnel without a profile keep running.
    let v = wait(&mut c, "a's tunnels failed", |v| {
        get(v, "tp").state == State::Failed && get(v, "tq").state == State::Failed
    })
    .await;
    for name in ["tp", "tq"] {
        let t = get(&v, name);
        assert_eq!(t.reason, Some(FailureReason::AuthExpired));
        assert_eq!(
            t.last_error.as_deref(),
            Some("the a login has expired; run `tunwarden login a`")
        );
        assert_eq!(t.pid, None, "the process was stopped");
    }
    assert_eq!(get(&v, "tr").state, State::Running);
    assert_eq!(get(&v, "plain").state, State::Running);

    // The check ran with each login's env and never with a profile's env.
    let calls = s.calls();
    assert!(calls.iter().all(|l| l.starts_with("status |")), "{calls:?}");
    assert!(calls.iter().any(|l| l.contains("account=a@example.com")));
    assert!(calls.iter().any(|l| l.contains("account=b@example.com")));
    assert!(calls.iter().all(|l| l.ends_with("project=")), "{calls:?}");

    let auth: AuthResult = c
        .call(method::AUTH_STATUS, AuthParams::default())
        .await
        .unwrap();
    let a = auth.logins.iter().find(|x| x.name == "a").unwrap();
    assert_eq!(a.result, "expired");
    assert_eq!(a.profiles, ["p", "q"]);
    assert!(
        !a.detail
            .as_deref()
            .unwrap_or_default()
            .contains("secret-token"),
        "the check's standard output is never read: {a:?}"
    );

    // `tunwarden login p` runs login a's command, then the daemon restarts both.
    // The CLI is a separate process; run it off this test's runtime thread,
    // which also runs the in-process daemon it talks to.
    let (config, state) = (s.config(), s.state());
    let out = tokio::task::spawn_blocking(move || {
        Command::new(CLI)
            .arg("--config")
            .arg(config)
            .args(["login", "p"])
            .env("TUNWARDEN_STATE_DIR", state)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("You are now logged in as [a@example.com]."),
        "the tool's own output reaches the terminal: {stdout}"
    );
    assert!(stdout.contains("logged in; restarting tp, tq"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Running the a login (used by p, q): faketool login"),
        "{stderr}"
    );
    wait(&mut c, "all running again", |v| {
        v.iter().all(|t| t.state == State::Running)
    })
    .await;

    let _: Value = c
        .call(method::SHUTDOWN, ShutdownParams::default())
        .await
        .unwrap();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_inconclusive_check_never_stops_tunnels() {
    let _turn = PORTS.lock().await;
    let s = Setup::new();
    let daemon = s.daemon(Duration::from_millis(200));
    let mut c = connect(&s.state()).await;
    start_all(&mut c).await;
    s.set("network-down", true);
    let before = s.calls().len();
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.calls().len() < before + 4 {
        assert!(Instant::now() < deadline, "no checks ran");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        views(&mut c)
            .await
            .iter()
            .all(|t| t.state == State::Running),
        "a network error looks like nothing"
    );
    let auth: AuthResult = c
        .call(
            method::AUTH_STATUS,
            AuthParams {
                name: Some("r".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(auth.logins.len(), 1, "profile r means login b");
    assert_eq!(auth.logins[0].name, "b");
    assert_eq!(auth.logins[0].result, "unknown");
    assert_eq!(
        auth.logins[0].detail.as_deref(),
        Some("ERROR: connection to auth.example.com timed out")
    );
    let _: Value = c
        .call(method::SHUTDOWN, ShutdownParams::default())
        .await
        .unwrap();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_tunnel_reporting_expiry_triggers_a_check_for_its_login() {
    // tp's own output reports the expiry; tq shares the login and must fail
    // too, long before the next scheduled check.
    let _turn = PORTS.lock().await;
    let s = Setup::new();
    let text =
        "ERROR: tunnel: Reauthentication failed. cannot prompt during non-interactive execution.";
    let config = std::fs::read_to_string(s.config()).unwrap();
    let mut lines: Vec<String> = config.lines().map(str::to_owned).collect();
    let i = lines.iter().position(|l| l == "name = \"tp\"").unwrap() + 1;
    lines[i] = lines[i].replace(
        "--listen",
        &format!("--print-after 1s --print '{text}' --listen"),
    );
    std::fs::write(s.config(), lines.join("\n") + "\n").unwrap();

    let daemon = s.daemon(Duration::from_secs(3600));
    let mut c = connect(&s.state()).await;
    // The login has already expired; only tp's output reveals it, because the
    // scheduled check is an hour away.
    s.set("expired-a@example.com", true);
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
    let v = wait(&mut c, "both failed", |v| {
        get(v, "tp").state == State::Failed && get(v, "tq").state == State::Failed
    })
    .await;
    assert_eq!(get(&v, "tq").reason, Some(FailureReason::AuthExpired));
    assert_eq!(
        s.calls().len(),
        1,
        "exactly one check, triggered by tp: {:?}",
        s.calls()
    );
    wait(&mut c, "tr running", |v| {
        get(v, "tr").state == State::Running
    })
    .await;
    let _: Value = c
        .call(method::SHUTDOWN, ShutdownParams::default())
        .await
        .unwrap();
    daemon.await.unwrap().unwrap();
}

#[test]
fn auth_command_without_a_daemon() {
    let s = Setup::new();
    let out = s.cli(&["auth"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("LOGIN"), "{stdout}");
    let row_a = stdout.lines().find(|l| l.starts_with("a ")).unwrap();
    assert!(row_a.contains("p, q") && row_a.contains("ok"), "{stdout}");
    assert!(!stdout.contains("secret-token"), "{stdout}");

    s.set("expired-b@example.com", true);
    let out = s.cli(&["auth", "r"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(!out.status.success(), "an expired login exits non-zero");
    assert!(
        stdout.contains("expired") && stdout.contains("run `tunwarden login b`"),
        "{stdout}"
    );
    assert!(
        !stdout.lines().any(|l| l.starts_with("a ")),
        "only r's login: {stdout}"
    );

    let out = s.cli(&["auth", "--json"]);
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["logins"].as_array().unwrap().len(), 2);

    let out = s.cli(&["login", "nope"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no login or profile named \"nope\"")
            && stderr.contains("hint: configured logins: a, b"),
        "{stderr}"
    );

    // Login without a daemon checks the result directly.
    let out = s.cli(&["login", "r"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("logged in"));
    assert!(!s.marker("expired-b@example.com").exists());
    assert!(
        s.calls()
            .iter()
            .any(|l| l.starts_with("login | account=b@example.com")),
        "login used the login's env: {:?}",
        s.calls()
    );
}
