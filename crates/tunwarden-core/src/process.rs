//! Spawning and stopping tunnel processes, and the crash sweep (design section 6).
//!
//! Every tunnel runs in its own process group (Unix) or job object (Windows),
//! so a stop reaches the command and everything it started (FR-12, AC-4).
//!
//! Windows status: written against `process-wrap`'s job objects but not run
//! yet. Graceful stop on Windows waits for spike S1 (`docs/spikes.md`); until
//! then a Windows stop terminates the job object directly, which PRD FR-12 allows.

use std::ffi::OsStr;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use process_wrap::tokio::{ChildWrapper, CommandWrap};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::process::{ChildStderr, ChildStdout, Command};

/// Default time a tunnel gets to exit after a graceful stop (FR-12).
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// How long to wait for the group to disappear after a forced kill.
const KILL_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(25);

/// What the daemon records about a running tunnel process, so it can find it
/// again after a crash (FR-52, design 8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub pid: u32,
    /// Process group ID on Unix (equal to `pid`, since the tunnel leads its
    /// group). On Windows, the PID of the job's first process.
    pub pgid: u32,
    /// Start time of `pid` in seconds since the Unix epoch, to detect PID reuse.
    pub start_time: u64,
}

/// How a stop ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// Everything exited within the grace period.
    Graceful,
    /// Something had to be killed.
    Forced,
}

/// Result of the crash sweep for one record (design 6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepOutcome {
    /// The tunnel's leftover processes were found and killed.
    Killed,
    /// The PID now belongs to a different process; it was left alone.
    PidReused,
    /// Nothing from this tunnel is still running.
    NothingLeft,
}

/// A running tunnel command.
pub struct TunnelProcess {
    child: Box<dyn ChildWrapper>,
    record: ProcessRecord,
    exit: Option<ExitStatus>,
}

impl TunnelProcess {
    /// Starts `program` with `args` and extra `env` variables. Never uses a
    /// shell. Standard input is null; output is piped for the supervisor.
    pub fn spawn(program: &OsStr, args: &[String], env: &[(String, String)]) -> io::Result<Self> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut wrap = CommandWrap::from(command);
        platform::wrap(&mut wrap);
        let child = wrap.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the process exited before its PID could be read"))?;
        let record = ProcessRecord {
            pid,
            pgid: pid,
            start_time: start_time(pid).unwrap_or(0),
        };
        Ok(TunnelProcess {
            child,
            record,
            exit: None,
        })
    }

    pub fn record(&self) -> ProcessRecord {
        self.record
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout().take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr().take()
    }

    /// Waits for the tunnel command itself to exit. Processes it started may
    /// still be running; [`TunnelProcess::stop`] removes them.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.exit {
            return Ok(status);
        }
        let status = self.child.wait().await?;
        self.exit = Some(status);
        Ok(status)
    }

    /// The exit status if the command has exited, without waiting.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait()?;
        }
        Ok(self.exit)
    }

    /// Stops the command and everything it started: graceful first, then
    /// forced after `grace` (FR-12). Safe to call after the command has exited,
    /// to remove anything it left behind. Returns once nothing is left.
    pub async fn stop(&mut self, grace: Duration) -> io::Result<StopOutcome> {
        platform::stop(self, grace).await
    }

    /// Whether anything from this tunnel is still running.
    pub fn alive(&mut self) -> bool {
        let _ = self.try_wait();
        platform::group_alive(&self.record)
    }
}

/// Dropping a handle without [`TunnelProcess::stop`] kills everything at once,
/// so a panic or an aborted task cannot leave tunnels running. (On Windows the
/// job object's kill-on-close does the same.) A daemon killed with SIGKILL runs
/// no destructors; the crash sweep covers that case.
impl Drop for TunnelProcess {
    fn drop(&mut self) {
        platform::kill_on_drop(&self.record);
    }
}

/// The start time of a process, in seconds since the Unix epoch.
pub fn start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new().ok()?;
    let pids = [Pid::from_u32(pid)];
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pids[0])
        .filter(|p| p.status() != sysinfo::ProcessStatus::Zombie)
        .map(|p| p.start_time())
}

/// Whether anything from a recorded tunnel is still running.
pub fn group_alive(record: &ProcessRecord) -> bool {
    platform::group_alive(record)
}

/// Removes what a crashed daemon left behind for one tunnel (design 6.3).
/// Blocks for up to `grace` plus a short kill wait.
pub fn sweep(record: &ProcessRecord, grace: Duration) -> SweepOutcome {
    match start_time(record.pid) {
        Some(actual) if actual != record.start_time => SweepOutcome::PidReused,
        Some(_) => {
            platform::kill_group_blocking(record, grace);
            SweepOutcome::Killed
        }
        // Leader gone. On Unix the group may still have members, and its ID
        // cannot be reused while it does, so it is still ours.
        None if platform::group_alive(record) => {
            platform::kill_group_blocking(record, grace);
            SweepOutcome::Killed
        }
        None => SweepOutcome::NothingLeft,
    }
}

/// Polls `done` until it returns true or `limit` passes.
#[cfg(unix)]
async fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

fn wait_until_blocking(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid as NixPid;
    use process_wrap::tokio::ProcessGroup;

    pub(super) fn wrap(wrap: &mut CommandWrap) {
        wrap.wrap(ProcessGroup::leader());
    }

    fn pgid(record: &ProcessRecord) -> NixPid {
        NixPid::from_raw(record.pgid as i32)
    }

    pub(super) fn group_alive(record: &ProcessRecord) -> bool {
        // EPERM means the group exists but is not ours; treat it as alive.
        !matches!(killpg(pgid(record), None), Err(Errno::ESRCH))
    }

    fn signal(record: &ProcessRecord, sig: Signal) {
        let _ = killpg(pgid(record), sig);
    }

    pub(super) async fn stop(p: &mut TunnelProcess, grace: Duration) -> io::Result<StopOutcome> {
        let record = p.record;
        if !p.alive() {
            return Ok(StopOutcome::Graceful);
        }
        signal(&record, Signal::SIGTERM);
        // Reaping the leader matters: an unreaped zombie still counts as a group member.
        if wait_until(grace, || !p.alive()).await {
            return Ok(StopOutcome::Graceful);
        }
        signal(&record, Signal::SIGKILL);
        if wait_until(KILL_WAIT, || !p.alive()).await {
            Ok(StopOutcome::Forced)
        } else {
            Err(io::Error::other(format!(
                "process group {} is still running after SIGKILL",
                record.pgid
            )))
        }
    }

    pub(super) fn kill_on_drop(record: &ProcessRecord) {
        if group_alive(record) {
            signal(record, Signal::SIGKILL);
        }
    }

    pub(super) fn kill_group_blocking(record: &ProcessRecord, grace: Duration) {
        signal(record, Signal::SIGTERM);
        if !wait_until_blocking(grace, || !group_alive(record)) {
            signal(record, Signal::SIGKILL);
            wait_until_blocking(KILL_WAIT, || !group_alive(record));
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use process_wrap::tokio::{CreationFlags, JobObject, KillOnDrop};

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub(super) fn wrap(wrap: &mut CommandWrap) {
        // KillOnDrop makes the job kill its processes when its handle closes,
        // including when the daemon dies (FR-52).
        wrap.wrap(CreationFlags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW))
            .wrap(JobObject)
            .wrap(KillOnDrop);
    }

    /// Without the job handle, only the recorded process and its descendants
    /// can be found.
    pub(super) fn group_alive(record: &ProcessRecord) -> bool {
        !tree(record).is_empty()
    }

    fn tree(record: &ProcessRecord) -> Vec<Pid> {
        let Ok(mut sys) = System::new() else {
            return Vec::new();
        };
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let root = Pid::from_u32(record.pid);
        let mut found: Vec<Pid> = sys.process(root).map(|_| vec![root]).unwrap_or_default();
        loop {
            let before = found.len();
            for (pid, p) in sys.processes() {
                if p.parent().is_some_and(|parent| found.contains(&parent)) && !found.contains(pid)
                {
                    found.push(*pid);
                }
            }
            if found.len() == before {
                return found;
            }
        }
    }

    pub(super) async fn stop(p: &mut TunnelProcess, _grace: Duration) -> io::Result<StopOutcome> {
        if p.try_wait()?.is_some() && !group_alive(&p.record) {
            return Ok(StopOutcome::Graceful);
        }
        // Terminates the whole job object; graceful stop awaits spike S1.
        p.child.start_kill()?;
        let _ = p.wait().await;
        Ok(StopOutcome::Forced)
    }

    /// The job object's kill-on-close handles this when the handle drops.
    pub(super) fn kill_on_drop(_record: &ProcessRecord) {}

    pub(super) fn kill_group_blocking(record: &ProcessRecord, _grace: Duration) {
        let Ok(mut sys) = System::new() else { return };
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        for pid in tree(record) {
            if let Some(p) = sys.process(pid) {
                p.kill();
            }
        }
        wait_until_blocking(KILL_WAIT, || !group_alive(record));
    }
}
