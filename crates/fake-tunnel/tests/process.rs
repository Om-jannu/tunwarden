//! System tests for `tunwarden_core::process`, using the real `fake-tunnel`
//! binary. They live in this crate because Cargo only provides a binary's path
//! (`CARGO_BIN_EXE_*`) to tests of the same package.

use std::ffi::OsStr;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
#[cfg(unix)]
use tunwarden_core::process::ProcessRecord;
use tunwarden_core::process::{self, StopOutcome, SweepOutcome, TunnelProcess, group_alive};

fn fake() -> &'static OsStr {
    OsStr::new(env!("CARGO_BIN_EXE_fake-tunnel"))
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Whether something accepts connections on the port. (Binding is no test on
/// macOS: Rust sets SO_REUSEADDR, so a second bind can succeed.)
fn port_in_use(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // A zombie counts as gone: it holds no resources and only waits to be reaped.
    match kill(Pid::from_raw(pid as i32), None) {
        Err(Errno::ESRCH) => false,
        _ => process::start_time(pid).is_some(),
    }
}

/// Polls until `f` is true, for up to 5 seconds.
async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Reads stdout lines until one starts with `prefix`, and returns the rest.
async fn read_line_with(p: &mut TunnelProcess, prefix: &str) -> String {
    let stdout = p.take_stdout().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();
    let read = async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(rest) = line.strip_prefix(prefix) {
                return rest.to_owned();
            }
        }
        panic!("stdout closed before a line starting with {prefix:?}");
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .expect("timed out reading stdout")
}

async fn child_pid(p: &mut TunnelProcess) -> u32 {
    read_line_with(p, "child ").await.parse().unwrap()
}

#[tokio::test]
async fn stop_removes_the_whole_tree_and_frees_the_port() {
    // AC-4: stopping leaves no child processes behind.
    let port = free_port();
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&["--spawn-child", "--listen", &format!("127.0.0.1:{port}")]),
        &[],
    )
    .unwrap();
    let child = child_pid(&mut p).await;
    eventually("port listening", || port_in_use(port)).await;
    assert!(p.alive());

    let outcome = p.stop(Duration::from_secs(5)).await.unwrap();

    assert_eq!(outcome, StopOutcome::Graceful);
    assert!(!p.alive());
    assert!(!group_alive(&p.record()));
    #[cfg(unix)]
    assert!(!pid_alive(child), "grandchild {child} survived the stop");
    let _ = child;
    eventually("port freed", || !port_in_use(port)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn stop_forces_when_sigterm_is_ignored() {
    let mut p =
        TunnelProcess::spawn(fake(), &args(&["--spawn-child", "--ignore-sigterm"]), &[]).unwrap();
    let child = child_pid(&mut p).await;

    let started = Instant::now();
    let outcome = p.stop(Duration::from_millis(300)).await.unwrap();

    assert_eq!(outcome, StopOutcome::Forced);
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(!p.alive());
    assert!(!pid_alive(child));
}

#[cfg(unix)]
#[tokio::test]
async fn stop_after_exit_removes_leftover_children() {
    // The command exits but leaves a grandchild running, like a wrapper
    // script that backgrounds the real tunnel.
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&["--spawn-child", "--exit-after", "200ms"]),
        &[],
    )
    .unwrap();
    let child = child_pid(&mut p).await;

    let status = p.wait().await.unwrap();
    assert!(status.success());
    assert!(pid_alive(child), "the grandchild should still be running");
    assert!(
        p.alive(),
        "the group is still alive while the grandchild runs"
    );

    p.stop(Duration::from_secs(2)).await.unwrap();
    assert!(!pid_alive(child));
    assert!(!p.alive());
}

#[tokio::test]
async fn exit_status_and_output_are_reported() {
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&[
            "--print",
            "Permission denied (publickey).",
            "--exit-after",
            "100ms",
            "--exit-code",
            "255",
        ]),
        &[],
    )
    .unwrap();
    let stderr = p.take_stderr().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.as_deref(), Some("Permission denied (publickey)."));
    let status = p.wait().await.unwrap();
    assert_eq!(status.code(), Some(255));
    assert_eq!(p.try_wait().unwrap(), Some(status));
}

#[tokio::test]
async fn env_is_passed_and_stdin_is_null() {
    let env = vec![("CLOUDSDK_CORE_PROJECT".to_owned(), "example-123".to_owned())];
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&["--print-env", "CLOUDSDK_CORE_PROJECT", "--exit-after", "1s"]),
        &env,
    )
    .unwrap();
    let value = read_line_with(&mut p, "env CLOUDSDK_CORE_PROJECT=").await;
    assert_eq!(value, "example-123");
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn missing_program_is_a_spawn_error() {
    let err = TunnelProcess::spawn(OsStr::new("/nonexistent/tunwarden-test-binary"), &[], &[])
        .err()
        .expect("spawn should fail");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn record_has_a_start_time() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut p = TunnelProcess::spawn(fake(), &[], &[]).unwrap();
        let r = p.record();
        assert_eq!(r.pid, r.pgid);
        assert_eq!(process::start_time(r.pid), Some(r.start_time));
        assert!(
            r.start_time > 1_700_000_000,
            "start time should be seconds since 1970"
        );
        p.stop(Duration::from_secs(2)).await.unwrap();
    });
}

// --- Crash sweep (design 6.3, FR-52) ------------------------------------------

/// Spawns a tunnel whose handle the caller forgets, as a crashed daemon would. Returns the record and the grandchild PID.
#[cfg(unix)]
async fn orphan(extra: &[&str]) -> (ProcessRecord, TunnelProcess, u32) {
    let mut list = vec!["--spawn-child"];
    list.extend_from_slice(extra);
    let mut p = TunnelProcess::spawn(fake(), &args(&list), &[]).unwrap();
    let child = child_pid(&mut p).await;
    (p.record(), p, child)
}

#[cfg(unix)]
#[tokio::test]
async fn sweep_kills_a_leftover_tree() {
    let (record, p, child) = orphan(&[]).await;
    // Forgetting the handle runs no destructor, like a daemon killed with SIGKILL.
    std::mem::forget(p);
    assert!(group_alive(&record));

    let outcome =
        tokio::task::spawn_blocking(move || process::sweep(&record, Duration::from_secs(2)))
            .await
            .unwrap();

    assert_eq!(outcome, SweepOutcome::Killed);
    // The forgotten leader stays a zombie of this test process (in a real
    // crash, the dead daemon's zombies are reaped by the system), so check
    // that every process is gone rather than the group.
    eventually("tree gone", || !pid_alive(record.pid) && !pid_alive(child)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn sweep_leaves_a_reused_pid_alone() {
    let (record, mut p, child) = orphan(&[]).await;
    let wrong = ProcessRecord {
        start_time: record.start_time + 1000,
        ..record
    };

    let outcome =
        tokio::task::spawn_blocking(move || process::sweep(&wrong, Duration::from_secs(2)))
            .await
            .unwrap();

    assert_eq!(outcome, SweepOutcome::PidReused);
    assert!(
        pid_alive(record.pid) && pid_alive(child),
        "nothing may be killed"
    );
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn sweep_kills_a_group_whose_leader_is_gone() {
    let (record, mut p, child) = orphan(&["--exit-after", "200ms"]).await;
    p.wait().await.unwrap(); // the leader exits and is reaped
    std::mem::forget(p);
    assert!(pid_alive(child));

    let outcome =
        tokio::task::spawn_blocking(move || process::sweep(&record, Duration::from_secs(2)))
            .await
            .unwrap();

    assert_eq!(outcome, SweepOutcome::Killed);
    eventually("grandchild gone", || !pid_alive(child)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_handle_kills_the_tree() {
    let (record, p, child) = orphan(&[]).await;
    drop(p);
    eventually("group gone", || !group_alive(&record)).await;
    assert!(!pid_alive(child));
}

#[tokio::test]
async fn sweep_reports_nothing_left() {
    let mut p = TunnelProcess::spawn(fake(), &[], &[]).unwrap();
    let record = p.record();
    p.stop(Duration::from_secs(2)).await.unwrap();

    let outcome =
        tokio::task::spawn_blocking(move || process::sweep(&record, Duration::from_secs(2)))
            .await
            .unwrap();

    assert_eq!(outcome, SweepOutcome::NothingLeft);
}
