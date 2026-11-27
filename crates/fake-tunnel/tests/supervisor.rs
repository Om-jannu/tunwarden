//! System tests for `tunwarden_core::supervisor` with real `fake-tunnel`
//! processes. Timings are shortened through `Policy`; the rules are the PRD's.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tunwarden_core::config::{self, Config};
use tunwarden_core::failure::FailureReason;
use tunwarden_core::paths::SystemEnv;
use tunwarden_core::probe::Prober;
use tunwarden_core::supervisor::{
    Event, LogSource, Observer, ReadyMode, Spec, State, Status, Supervisor,
};

/// Port tests take turns, so a port freed by one test is not grabbed by another.
static PORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

#[derive(Default)]
struct Recorder {
    lines: Mutex<Vec<(LogSource, String)>>,
    states: Mutex<Vec<State>>,
}

impl Observer for Recorder {
    fn log(&self, _tunnel: &str, source: LogSource, line: &str) {
        self.lines.lock().unwrap().push((source, line.to_owned()));
    }

    fn status(&self, status: &Status) {
        let mut states = self.states.lock().unwrap();
        if states.last() != Some(&status.state) {
            states.push(status.state);
        }
    }
}

impl Recorder {
    fn count(&self, needle: &str) -> usize {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| l.contains(needle))
            .count()
    }

    fn output(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .any(|(s, l)| *s != LogSource::Tunwarden && l.contains(needle))
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn connectable(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Loads a config with a login `work` (expired when the output says
/// "Reauthentication failed"), a profile `p` that uses it, and the given tunnels.
fn load(tunnels: &str) -> Config {
    let src = format!(
        "version = 1\n[[logins]]\nname = \"work\"\nlogin = \"true\"\nexpired = [\"Reauthentication failed\"]\n\
         [[profiles]]\nname = \"p\"\nlogin = \"work\"\nenv = {{ TOOL_PROJECT = \"example-p-1\" }}\n\n{tunnels}"
    );
    match config::parse(&src, Path::new("test.toml"), &SystemEnv) {
        Ok(c) => c,
        Err(e) => panic!("bad test config:\n{e}"),
    }
}

/// A command tunnel running fake-tunnel with `args`, optionally listening on `port`.
fn tunnel(name: &str, args: &str, port: Option<u16>, profile: bool) -> String {
    let mut t = format!("[[tunnels]]\nname = \"{name}\"\ncommand = \"'{FAKE}' {args}\"\n");
    if let Some(port) = port {
        t.push_str(&format!("forwards = [{{ local_port = {port} }}]\n"));
    }
    if profile {
        t.push_str("profile = \"p\"\n");
    }
    t
}

/// A spec with short timings.
fn spec(config: &Config, index: usize) -> Spec {
    let mut s = Spec::new(&config.tunnels[index], config);
    s.policy.starting_probe = Duration::from_millis(50);
    s.policy.health_interval = Duration::from_millis(200);
    s.policy.startup_grace = Duration::from_secs(5);
    s.policy.backoff_base = Duration::from_millis(100);
    s.policy.backoff_max = Duration::from_millis(400);
    s.policy.stable_after = Duration::from_secs(30);
    s.stop_grace = Duration::from_secs(1);
    s
}

fn start(spec: Spec) -> (Supervisor, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let sup = Supervisor::spawn(spec, Arc::new(Prober::new()), recorder.clone());
    (sup, recorder)
}

async fn wait_for(
    sup: &Supervisor,
    what: &str,
    limit: Duration,
    f: impl Fn(&Status) -> bool,
) -> Status {
    let mut rx = sup.subscribe();
    let result = tokio::time::timeout(limit, async {
        loop {
            let s = rx.borrow_and_update().clone();
            if f(&s) {
                return s;
            }
            rx.changed().await.unwrap();
        }
    })
    .await;
    match result {
        Ok(s) => s,
        Err(_) => panic!("timed out waiting for {what}; status: {:?}", sup.status()),
    }
}

/// Polls until `f` is true, for up to `limit`. For conditions that do not
/// change the status, such as log output.
async fn until(what: &str, limit: Duration, f: impl Fn() -> bool) {
    let deadline = Instant::now() + limit;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn state(sup: &Supervisor, wanted: State) -> Status {
    wait_for(sup, wanted.as_str(), Duration::from_secs(10), |s| {
        s.state == wanted
    })
    .await
}

#[tokio::test]
async fn start_becomes_running_and_stop_cleans_up() {
    let _turn = PORTS.lock().await;
    let port = free_port();
    let c = load(&tunnel(
        "db",
        &format!("--spawn-child --listen 127.0.0.1:{port}"),
        Some(port),
        false,
    ));
    let (sup, rec) = start(spec(&c, 0));

    sup.send(Event::Start);
    let running = state(&sup, State::Running).await;
    assert!(connectable(port));
    let record = running.process.expect("a process record");
    assert!(running.desired_running);

    sup.send(Event::Stop);
    let stopped = state(&sup, State::Stopped).await;
    assert_eq!(stopped.process, None);
    assert!(!connectable(port), "the port is closed after stop");
    assert!(
        !tunwarden_core::process::group_alive(&record),
        "no processes left (AC-4)"
    );
    assert_eq!(
        *rec.states.lock().unwrap(),
        [State::Starting, State::Running, State::Stopped]
    );
    assert_eq!(rec.count("started PID"), 1);
    sup.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_killed_process_comes_back() {
    // AC-2
    let _turn = PORTS.lock().await;
    let port = free_port();
    let c = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{port}"),
        Some(port),
        false,
    ));
    let (sup, rec) = start(spec(&c, 0));
    sup.send(Event::Start);
    let first = state(&sup, State::Running).await.process.unwrap();

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(first.pid as i32),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();

    let back = wait_for(&sup, "running again", Duration::from_secs(10), |s| {
        s.state == State::Running && s.process.is_some_and(|p| p.pid != first.pid)
    })
    .await;
    assert_eq!(back.restarts, 1);
    assert!(
        rec.count("was killed by signal 9") >= 1,
        "the exit is logged"
    );
    assert!(rec.count("retry 1/10") == 1, "the retry is logged");
    assert!(connectable(port));
    sup.shutdown().await;
}

#[tokio::test]
async fn an_auth_error_fails_at_once_and_login_recovers() {
    // AC-3: a tunnel whose profile uses a login prints that login's expired text
    // (here the real gcloud wording).
    let _turn = PORTS.lock().await;
    let port = free_port();
    let text = "ERROR: (gcloud.compute.start-iap-tunnel) There was a problem refreshing your current auth tokens: Reauthentication failed. cannot prompt during non-interactive execution.";
    let c = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{port} --print-after 300ms --print '{text}'"),
        Some(port),
        true,
    ));
    let (sup, rec) = start(spec(&c, 0));

    let started = Instant::now();
    sup.send(Event::Start);
    let failed = state(&sup, State::Failed).await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "within 10 seconds (AC-3)"
    );
    assert_eq!(failed.reason, Some(FailureReason::AuthExpired));
    assert_eq!(
        failed.last_error.as_deref(),
        Some("the work login has expired; run `tunwarden login work`")
    );
    assert_eq!(failed.process, None, "the process was stopped");

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        rec.count("started PID"),
        1,
        "no retries after an auth error"
    );
    assert_eq!(sup.status().state, State::Failed);

    sup.send(Event::AuthRecovered);
    until("a new start", Duration::from_secs(5), || {
        rec.count("started PID") == 2
    })
    .await;
    sup.shutdown().await;
}

#[tokio::test]
async fn an_occupied_port_fails_immediately() {
    // AC-5
    let _turn = PORTS.lock().await;
    let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    let c = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{port}"),
        Some(port),
        false,
    ));
    let (sup, rec) = start(spec(&c, 0));

    let started = Instant::now();
    sup.send(Event::Start);
    let failed = state(&sup, State::Failed).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(failed.reason, Some(FailureReason::PortConflict));
    let message = failed.last_error.unwrap();
    assert!(message.contains(&format!("127.0.0.1:{port}")), "{message}");
    assert!(
        message.contains(&format!("(PID {})", std::process::id())),
        "{message}"
    );
    assert_eq!(rec.count("started PID"), 0, "nothing was started");
    sup.shutdown().await;
    drop(squatter);
}

#[tokio::test]
async fn repeated_exits_exhaust_the_retries() {
    let _turn = PORTS.lock().await;
    let c = load(&tunnel(
        "flaky",
        "--exit-after 100ms --exit-code 1",
        None,
        false,
    ));
    let mut s = spec(&c, 0);
    s.policy.max_retries = 2;
    s.policy.ready = ReadyMode::AliveFor(Duration::from_secs(5));
    let (sup, rec) = start(s);

    sup.send(Event::Start);
    let failed = state(&sup, State::Failed).await;
    assert_eq!(failed.reason, Some(FailureReason::RetriesExhausted));
    assert_eq!(failed.restarts, 2);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("the command exited with code 1")
    );
    assert_eq!(rec.count("started PID"), 3);
    assert_eq!(rec.count("giving up after 2 retries"), 1);
    sup.shutdown().await;
}

#[tokio::test]
async fn never_ready_is_restarted_after_the_grace_period() {
    let _turn = PORTS.lock().await;
    let port = free_port();
    let c = load(&tunnel("slow", "", Some(port), false));
    let mut s = spec(&c, 0);
    s.policy.startup_grace = Duration::from_secs(1);
    s.policy.max_retries = 1;
    let (sup, rec) = start(s);

    sup.send(Event::Start);
    let failed = state(&sup, State::Failed).await;
    assert_eq!(failed.reason, Some(FailureReason::RetriesExhausted));
    assert_eq!(
        failed.last_error.as_deref(),
        Some("not ready within 1 seconds")
    );
    assert_eq!(rec.count("started PID"), 2);
    sup.shutdown().await;
}

#[tokio::test]
async fn a_missing_program_is_reported() {
    let c = load("[[tunnels]]\nname = \"t\"\ncommand = \"/nonexistent/tunwarden-tool --x\"\n");
    let (sup, _) = start(spec(&c, 0));
    sup.send(Event::Start);
    let failed = state(&sup, State::Failed).await;
    assert_eq!(failed.reason, Some(FailureReason::MissingTool));
    assert_eq!(
        failed.last_error.as_deref(),
        Some("/nonexistent/tunwarden-tool does not exist")
    );
    sup.shutdown().await;
}

#[tokio::test]
async fn process_alive_readiness() {
    let c = load(&tunnel("vpn", "", None, false));
    let mut s = spec(&c, 0);
    s.policy.ready = ReadyMode::AliveFor(Duration::from_millis(300));
    let (sup, _) = start(s);
    let started = Instant::now();
    sup.send(Event::Start);
    state(&sup, State::Running).await;
    assert!(started.elapsed() >= Duration::from_millis(300));
    sup.shutdown().await;
}

#[tokio::test]
async fn two_profiles_get_their_own_environment() {
    // AC-8: each tunnel gets its own profile's variables, set only in its environment.
    let _turn = PORTS.lock().await;
    let src = format!(
        "version = 1\n[[profiles]]\nname = \"a\"\nenv = {{ TOOL_PROJECT = \"example-a-1\" }}\n\
         [[profiles]]\nname = \"b\"\nenv = {{ TOOL_PROJECT = \"example-b-2\", TOOL_ACCOUNT = \"b@example.com\" }}\n\
         [[tunnels]]\nname = \"ta\"\nprofile = \"a\"\ncommand = \"'{FAKE}' --print-env TOOL_PROJECT --print-env TOOL_ACCOUNT\"\n\
         [[tunnels]]\nname = \"tb\"\nprofile = \"b\"\ncommand = \"'{FAKE}' --print-env TOOL_PROJECT --print-env TOOL_ACCOUNT\"\n"
    );
    let c = config::parse(&src, Path::new("t.toml"), &SystemEnv).unwrap();
    let (a, ra) = start(spec(&c, 0));
    let (b, rb) = start(spec(&c, 1));
    a.send(Event::Start);
    b.send(Event::Start);
    until("output", Duration::from_secs(5), || {
        ra.output("TOOL_ACCOUNT")
    })
    .await;
    until("output", Duration::from_secs(5), || {
        rb.output("TOOL_ACCOUNT")
    })
    .await;

    assert!(ra.output("env TOOL_PROJECT=example-a-1"));
    assert!(ra.output("env TOOL_ACCOUNT=<unset>"));
    assert!(rb.output("env TOOL_PROJECT=example-b-2"));
    assert!(rb.output("env TOOL_ACCOUNT=b@example.com"));
    assert!(
        std::env::var("TOOL_PROJECT").is_err(),
        "Tunwarden's own environment is untouched"
    );
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn config_changes_wait_for_a_restart() {
    // FR-5 "restart to apply"
    let _turn = PORTS.lock().await;
    let (p1, p2) = (free_port(), free_port());
    let c1 = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{p1}"),
        Some(p1),
        false,
    ));
    let c2 = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{p2}"),
        Some(p2),
        false,
    ));
    let (sup, _) = start(spec(&c1, 0));
    sup.send(Event::Start);
    state(&sup, State::Running).await;

    sup.update(spec(&c2, 0));
    let s = wait_for(&sup, "pending restart", Duration::from_secs(2), |s| {
        s.pending_restart
    })
    .await;
    assert_eq!(
        s.state,
        State::Running,
        "the running tunnel is not interrupted"
    );
    assert!(connectable(p1));

    sup.send(Event::Restart);
    wait_for(&sup, "applied", Duration::from_secs(10), |s| {
        s.state == State::Running && !s.pending_restart
    })
    .await;
    assert!(connectable(p2));
    assert!(!connectable(p1));
    sup.shutdown().await;
}

#[tokio::test]
async fn shutdown_stops_the_process() {
    let _turn = PORTS.lock().await;
    let port = free_port();
    let c = load(&tunnel(
        "db",
        &format!("--listen 127.0.0.1:{port}"),
        Some(port),
        false,
    ));
    let (sup, _) = start(spec(&c, 0));
    sup.send(Event::Start);
    let record = state(&sup, State::Running).await.process.unwrap();
    sup.shutdown().await;
    assert!(!tunwarden_core::process::group_alive(&record));
    assert!(!connectable(port));
}
