//! System tests for `tunwarden_core::probe` against real processes and sockets.

use std::ffi::OsStr;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tunwarden_core::probe::{Prober, Ready, Snapshot};
use tunwarden_core::process::TunnelProcess;

/// Tests here pick free ports and also hold ports on purpose. Run in parallel,
/// the OS can hand a just-freed port to another test, so they take turns.
static PORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fake() -> &'static OsStr {
    OsStr::new(env!("CARGO_BIN_EXE_fake-tunnel"))
}

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Probes until `ready` returns something other than `Waiting`, for up to 5 s.
async fn settle(prober: &Prober, pid: u32, wanted: &[SocketAddr]) -> (Ready, Duration) {
    let started = Instant::now();
    loop {
        let r = prober.fresh_snapshot().await.unwrap().ready(pid, wanted);
        if !matches!(r, Ready::Waiting(_)) || started.elapsed() > Duration::from_secs(5) {
            return (r, started.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn waiting_then_ready_without_connecting() {
    let _turn = PORTS.lock().await;
    let addr = free_addr();
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&["--listen", &addr.to_string(), "--listen-after", "400ms"]),
        &[],
    )
    .unwrap();
    let pid = p.record().pid;
    let prober = Prober::new();

    let first = prober.fresh_snapshot().await.unwrap().ready(pid, &[addr]);
    assert_eq!(first, Ready::Waiting(vec![addr]));

    let (r, took) = settle(&prober, pid, &[addr]).await;
    assert_eq!(r, Ready::Yes);
    assert!(
        took >= Duration::from_millis(300),
        "became ready too early: {took:?}"
    );
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn every_port_must_listen() {
    let _turn = PORTS.lock().await;
    let (a, b) = (free_addr(), free_addr());
    let mut p = TunnelProcess::spawn(fake(), &args(&["--listen", &a.to_string()]), &[]).unwrap();
    let pid = p.record().pid;
    let prober = Prober::new();
    // Once the first port is up, only the second is reported missing.
    let (first, _) = settle(&prober, pid, &[a]).await;
    assert_eq!(first, Ready::Yes);
    let r = prober.fresh_snapshot().await.unwrap().ready(pid, &[a, b]);
    assert_eq!(r, Ready::Waiting(vec![b]));
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_grandchild_listener_counts() {
    let _turn = PORTS.lock().await;
    // Like a wrapper script that runs the real tunnel as a child.
    let addr = free_addr();
    let script = format!(
        "'{}' --listen {addr}; true",
        env!("CARGO_BIN_EXE_fake-tunnel")
    );
    let mut p = TunnelProcess::spawn(OsStr::new("sh"), &args(&["-c", &script]), &[]).unwrap();
    let (r, _) = settle(&Prober::new(), p.record().pid, &[addr]).await;
    assert_eq!(r, Ready::Yes);
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn a_port_held_by_someone_else_is_not_ready() {
    let _turn = PORTS.lock().await;
    let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = squatter.local_addr().unwrap();
    let mut p = TunnelProcess::spawn(fake(), &[], &[]).unwrap();

    let r = Prober::new()
        .fresh_snapshot()
        .await
        .unwrap()
        .ready(p.record().pid, &[addr]);

    match r {
        Ready::HeldByOther(at, holder) => {
            assert_eq!(at, addr);
            assert_eq!(
                holder.pid,
                std::process::id(),
                "the test process holds the port"
            );
        }
        other => panic!("expected HeldByOther, got {other:?}"),
    }
    p.stop(Duration::from_secs(2)).await.unwrap();
    drop(squatter);
}

#[tokio::test]
async fn holder_names_the_process_for_port_conflicts() {
    let _turn = PORTS.lock().await;
    let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = squatter.local_addr().unwrap();
    let snapshot = tokio::task::spawn_blocking(Snapshot::take)
        .await
        .unwrap()
        .unwrap();
    let holder = snapshot.holder(addr).expect("holder");
    assert_eq!(holder.pid, std::process::id());
    assert!(!holder.name.is_empty());
    assert_eq!(snapshot.holder(free_addr()), None);
}

#[tokio::test]
async fn wildcard_listener_serves_loopback() {
    let _turn = PORTS.lock().await;
    let port = free_addr().port();
    let mut p = TunnelProcess::spawn(
        fake(),
        &args(&["--listen", &format!("0.0.0.0:{port}")]),
        &[],
    )
    .unwrap();
    let wanted: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (r, _) = settle(&Prober::new(), p.record().pid, &[wanted]).await;
    assert_eq!(r, Ready::Yes);
    p.stop(Duration::from_secs(2)).await.unwrap();
}

#[tokio::test]
async fn snapshots_are_shared_briefly() {
    let _turn = PORTS.lock().await;
    let prober = Prober::new();
    let first = prober.snapshot().await.unwrap();
    let second = prober.snapshot().await.unwrap();
    assert!(
        Arc::ptr_eq(&first, &second),
        "a second probe within 500 ms reuses the snapshot"
    );
    let fresh = prober.fresh_snapshot().await.unwrap();
    assert!(!Arc::ptr_eq(&first, &fresh));
}
