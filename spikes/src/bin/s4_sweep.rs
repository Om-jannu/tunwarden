//! Spike S4: after the daemon is killed with SIGKILL, do tunnel processes
//! (including grandchildren) survive, and can a PID + start-time sweep on the
//! next start remove them safely?
//!
//! Usage:
//!   s4_sweep parent STATEFILE [--pdeathsig]   spawn a fake tunnel tree, record it, then wait to be killed
//!   s4_sweep sweep STATEFILE [--wrong-start]  kill the recorded group if PID and start time still match

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid as NixPid;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

fn start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new().expect("sysinfo");
    let p = [Pid::from_u32(pid)];
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&p), true, ProcessRefreshKind::nothing());
    sys.process(p[0]).map(|p| p.start_time())
}

fn group_alive(pgid: i32) -> bool {
    !matches!(killpg(NixPid::from_raw(pgid), None), Err(Errno::ESRCH))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let state = &args[2];
    let flag = args.get(3).map(String::as_str);

    match args[1].as_str() {
        "parent" => {
            // Fake tunnel: a shell (direct child) that starts a grandchild, like gcloud starting helpers.
            let mut cmd = Command::new("sh");
            // --leader-exits: the direct child exits and leaves an orphaned grandchild,
            // like a wrapper script that backgrounds the real tunnel.
            let script = if flag == Some("--leader-exits") { "sleep 300 & exit 0" } else { "sleep 300 & sleep 300" };
            cmd.args(["-c", script])
                .env("TUNWARDEN_TUNNEL", format!("spike-{}/demo", std::process::id()))
                .stdin(Stdio::null())
                .process_group(0);
            #[cfg(target_os = "linux")]
            if flag == Some("--pdeathsig") {
                unsafe {
                    cmd.pre_exec(|| {
                        nix::sys::prctl::set_pdeathsig(Signal::SIGKILL).map_err(std::io::Error::from)
                    });
                }
            }
            #[cfg(not(target_os = "linux"))]
            let _ = flag;
            let child = cmd.spawn().expect("spawn");
            let pid = child.id();
            let st = start_time(pid).expect("start time");
            std::thread::sleep(Duration::from_millis(300));
            std::fs::write(state, format!("{pid} {pid} {st} spike-{}/demo\n", std::process::id())).expect("write state");
            println!("parent {}: recorded tunnel pid={pid} pgid={pid} start_time={st}", std::process::id());
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        "sweep" => {
            let rec = std::fs::read_to_string(state).expect("read state");
            let f: Vec<u64> = rec.split_whitespace().take(3).map(|x| x.parse().unwrap()).collect();
            let (pid, pgid, mut recorded) = (f[0] as u32, f[1] as i32, f[2]);
            if flag == Some("--wrong-start") {
                recorded += 1000; // simulate a reused PID
            }
            // Rule: an ID cannot become a new PID while a process group with that ID has
            // members (POSIX). So:
            //   leader alive, start time matches  -> our group, kill it
            //   leader alive, start time differs  -> PID was reused, leave alone
            //   leader gone, group has members    -> still our group, kill it
            //   leader gone, group empty          -> nothing to do
            let kill_group = match start_time(pid) {
                Some(actual) if actual == recorded => {
                    println!("sweep: leader {pid} alive with recorded start time");
                    true
                }
                Some(actual) => {
                    println!("RESULT: pid {pid} start time {actual} != recorded {recorded}; left alone (PID reuse guard)");
                    false
                }
                None if group_alive(pgid) => {
                    println!("sweep: leader {pid} gone but group {pgid} still has members");
                    true
                }
                None => {
                    println!("RESULT: nothing left to sweep");
                    false
                }
            };
            if kill_group {
                let _ = killpg(NixPid::from_raw(pgid), Signal::SIGTERM);
                for _ in 0..50 {
                    if !group_alive(pgid) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if group_alive(pgid) {
                    let _ = killpg(NixPid::from_raw(pgid), Signal::SIGKILL);
                    std::thread::sleep(Duration::from_millis(200));
                }
                println!("RESULT: group {pgid} alive after sweep: {}", group_alive(pgid));
            }
        }
        "sweep-env" => {
            let rec = std::fs::read_to_string(state).expect("read state");
            let tag = format!("TUNWARDEN_TUNNEL={}", rec.split_whitespace().nth(3).unwrap());
            let mut sys = System::new().expect("sysinfo");
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_environ(sysinfo::UpdateKind::Always),
            );
            let found: Vec<u32> = sys
                .processes()
                .iter()
                .filter(|(_, p)| p.environ().iter().any(|e| e.to_str() == Some(tag.as_str())))
                .map(|(pid, _)| pid.as_u32())
                .collect();
            println!("sweep-env: processes tagged {tag}: {found:?}");
            for pid in &found {
                let _ = nix::sys::signal::kill(NixPid::from_raw(*pid as i32), Signal::SIGKILL);
            }
            std::thread::sleep(Duration::from_millis(300));
            let p: Vec<Pid> = found.iter().map(|p| Pid::from_u32(*p)).collect();
            sys.refresh_processes_specifics(ProcessesToUpdate::Some(&p), true, ProcessRefreshKind::nothing());
            let left = p.iter().filter(|p| sys.process(**p).is_some_and(|p| p.status() != sysinfo::ProcessStatus::Zombie)).count();
            println!("RESULT: tagged processes left after sweep-env: {left}");
        }
        _ => panic!("unknown mode"),
    }
}
