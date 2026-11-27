//! Spike S2: can readiness be read from the OS socket table, attributed to the
//! tunnel's own process tree, without connecting to the port?
//!
//! Usage: s2_listeners PORT TIMEOUT_SECS -- COMMAND [ARGS...]

use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid as NixPid;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

fn tree(sys: &System, root: u32) -> HashSet<u32> {
    let mut set = HashSet::from([root]);
    loop {
        let before = set.len();
        for (pid, p) in sys.processes() {
            if let Some(parent) = p.parent() {
                if set.contains(&parent.as_u32()) {
                    set.insert(pid.as_u32());
                }
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args[1].parse().expect("port");
    let timeout = Duration::from_secs(args[2].parse().expect("timeout"));
    let cmd = &args[4..];

    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn");
    let root = child.id();
    println!("spawned {} as pid {root}", cmd[0]);

    let mut sys = System::new().expect("sysinfo");
    let started = Instant::now();
    let mut ready = None;
    while started.elapsed() < timeout {
        if let Ok(Some(status)) = child.try_wait() {
            println!("RESULT: command exited early with {status}");
            return;
        }
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        let pids = tree(&sys, root);
        let all = listeners::get_all().expect("listeners");
        let ours: Vec<_> = all
            .iter()
            .filter(|l| l.socket.port() == port && format!("{:?}", l.state) == "Listen")
            .collect();
        if !ours.is_empty() {
            for l in &ours {
                let owned = pids.contains(&l.process.pid);
                println!(
                    "  listener {} owned by pid {} ({}), in tunnel tree: {owned}",
                    l.socket, l.process.pid, l.process.name
                );
            }
            println!("  tunnel tree pids: {:?}", pids);
            ready = Some((started.elapsed(), ours.iter().all(|l| pids.contains(&l.process.pid))));
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    match ready {
        Some((t, true)) => println!("RESULT: ready after {t:?}, all listeners owned by the tunnel tree"),
        Some((t, false)) => println!("RESULT: ready after {t:?}, but a listener is NOT owned by the tunnel tree"),
        None => println!("RESULT: no listener on port {port} within {timeout:?}"),
    }

    // Cost of one socket-table snapshot.
    let n = 20;
    let t = Instant::now();
    for _ in 0..n {
        let _ = listeners::get_all();
    }
    println!("COST: listeners::get_all() avg {:?} over {n} calls", t.elapsed() / n);
    let t = Instant::now();
    for _ in 0..n {
        let _ = listeners::get_process_by_port(port, listeners::Protocol::TCP);
    }
    println!("COST: listeners::get_process_by_port() avg {:?} over {n} calls", t.elapsed() / n);
    let t = Instant::now();
    for _ in 0..n {
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    }
    println!("COST: sysinfo process refresh avg {:?} over {n} calls", t.elapsed() / n);

    let _ = killpg(NixPid::from_raw(root as i32), Signal::SIGTERM);
    let _ = child.wait();
    let _ = Pid::from_u32(root);
}
