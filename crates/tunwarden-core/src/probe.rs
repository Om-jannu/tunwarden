//! Readiness from the OS socket table (PRD FR-16, design 8.1).
//!
//! The probe never connects to a tunnel's port: a connection would go through
//! the tunnel (a new IAP or kubectl session each time) and leave noise in the
//! remote service's logs. Instead it reads which process owns each listening
//! socket and checks that the owner belongs to the tunnel's process tree.
//! Spike S2 measured one snapshot at well under 10 ms.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// How long a snapshot is reused, so many tunnels probing together cost one read.
pub const SNAPSHOT_MAX_AGE: Duration = Duration::from_millis(500);

/// A listening TCP socket and the process that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenSocket {
    pub addr: SocketAddr,
    pub pid: u32,
    pub process_name: String,
}

/// A process holding a port, for port-conflict messages (FR-13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub name: String,
}

/// The result of a readiness check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ready {
    /// Every wanted address is listening and owned by the tunnel.
    Yes,
    /// These addresses are not listening yet.
    Waiting(Vec<SocketAddr>),
    /// A wanted address is held by a process outside the tunnel.
    HeldByOther(SocketAddr, Holder),
}

/// The listening sockets and process parents at one moment.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    sockets: Vec<ListenSocket>,
    parents: HashMap<u32, u32>,
}

impl Snapshot {
    /// Reads the socket table and the process table. Blocking; use
    /// [`Prober::snapshot`] from async code.
    pub fn take() -> io::Result<Snapshot> {
        let sockets = listeners::get_all()
            .map_err(|e| io::Error::other(format!("cannot read the socket table: {e}")))?
            .into_iter()
            .filter(|l| {
                l.protocol == listeners::Protocol::TCP && l.state == listeners::SocketState::Listen
            })
            .map(|l| ListenSocket {
                addr: l.socket,
                pid: l.process.pid,
                process_name: l.process.name,
            })
            .collect::<Vec<_>>();
        let mut sockets = sockets;
        // A socket can have several owners, for example after a child inherited
        // it. Sorting by PID makes the reported owner stable: usually the creator.
        sockets.sort_by_key(|s: &ListenSocket| (s.pid, s.addr));

        let mut sys = System::new()
            .map_err(|e| io::Error::other(format!("cannot read the process table: {e}")))?;
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let parents = sys
            .processes()
            .iter()
            .filter_map(|(pid, p)| p.parent().map(|parent| (pid.as_u32(), parent.as_u32())))
            .collect();

        Ok(Snapshot { sockets, parents })
    }

    /// A snapshot built from parts, for tests.
    pub fn from_parts(mut sockets: Vec<ListenSocket>, parents: HashMap<u32, u32>) -> Snapshot {
        sockets.sort_by_key(|s| (s.pid, s.addr));
        Snapshot { sockets, parents }
    }

    /// Whether `pid` is `root` or a descendant of it.
    fn in_tree(&self, root: u32, pid: u32) -> bool {
        let mut current = pid;
        // The bound guards against a cycle in a racy process table.
        for _ in 0..64 {
            if current == root {
                return true;
            }
            match self.parents.get(&current) {
                Some(&parent) if parent != current => current = parent,
                _ => return false,
            }
        }
        false
    }

    /// The sockets that serve `wanted`: the same address, or a wildcard
    /// (`0.0.0.0` or `::`) on the same port.
    fn serving(&self, wanted: SocketAddr) -> impl Iterator<Item = &ListenSocket> {
        self.sockets.iter().filter(move |s| {
            s.addr.port() == wanted.port()
                && (s.addr.ip() == wanted.ip() || s.addr.ip().is_unspecified())
        })
    }

    /// Whether the tunnel whose command has PID `root` is listening on every
    /// address in `wanted` (FR-16).
    pub fn ready(&self, root: u32, wanted: &[SocketAddr]) -> Ready {
        let mut missing = Vec::new();
        for &addr in wanted {
            let mut serving = self.serving(addr).peekable();
            if serving.peek().is_none() {
                missing.push(addr);
                continue;
            }
            let sockets: Vec<&ListenSocket> = serving.collect();
            if !sockets.iter().any(|s| self.in_tree(root, s.pid)) {
                let other = sockets[0];
                return Ready::HeldByOther(
                    addr,
                    Holder {
                        pid: other.pid,
                        name: other.process_name.clone(),
                    },
                );
            }
        }
        if missing.is_empty() {
            Ready::Yes
        } else {
            Ready::Waiting(missing)
        }
    }

    /// The process already listening on `addr`, if any (FR-13). With several
    /// owners, the one with the lowest PID.
    pub fn holder(&self, addr: SocketAddr) -> Option<Holder> {
        self.serving(addr).next().map(|s| Holder {
            pid: s.pid,
            name: s.process_name.clone(),
        })
    }
}

/// Shares one snapshot between all tunnels for [`SNAPSHOT_MAX_AGE`].
#[derive(Default)]
pub struct Prober {
    cache: Mutex<Option<(Instant, Arc<Snapshot>)>>,
}

impl Prober {
    pub fn new() -> Prober {
        Prober::default()
    }

    /// A snapshot no older than [`SNAPSHOT_MAX_AGE`].
    pub async fn snapshot(&self) -> io::Result<Arc<Snapshot>> {
        if let Some(snapshot) = self.cached() {
            return Ok(snapshot);
        }
        let fresh = Arc::new(
            tokio::task::spawn_blocking(Snapshot::take)
                .await
                .map_err(io::Error::other)??,
        );
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), fresh.clone()));
        Ok(fresh)
    }

    /// A new snapshot, ignoring the cache. Used right before a spawn, where a
    /// stale view could miss a port conflict.
    pub async fn fresh_snapshot(&self) -> io::Result<Arc<Snapshot>> {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.snapshot().await
    }

    fn cached(&self) -> Option<Arc<Snapshot>> {
        let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            Some((taken, snapshot)) if taken.elapsed() < SNAPSHOT_MAX_AGE => Some(snapshot.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sock(addr: &str, pid: u32) -> ListenSocket {
        ListenSocket {
            addr: addr.parse().unwrap(),
            pid,
            process_name: format!("p{pid}"),
        }
    }

    fn a(addr: &str) -> SocketAddr {
        addr.parse().unwrap()
    }

    /// Tree: 100 is the tunnel, 101 its child, 102 a grandchild; 200 is unrelated.
    fn parents() -> HashMap<u32, u32> {
        HashMap::from([(100, 1), (101, 100), (102, 101), (200, 1)])
    }

    #[test]
    fn ready_when_owned_by_the_tree() {
        let s = Snapshot::from_parts(vec![sock("127.0.0.1:5433", 102)], parents());
        assert_eq!(s.ready(100, &[a("127.0.0.1:5433")]), Ready::Yes);
    }

    #[test]
    fn waiting_lists_missing_addresses() {
        let s = Snapshot::from_parts(vec![sock("127.0.0.1:5433", 100)], parents());
        assert_eq!(
            s.ready(100, &[a("127.0.0.1:5433"), a("127.0.0.1:5434")]),
            Ready::Waiting(vec![a("127.0.0.1:5434")])
        );
    }

    #[test]
    fn port_held_by_another_process() {
        let s = Snapshot::from_parts(vec![sock("127.0.0.1:5433", 200)], parents());
        assert_eq!(
            s.ready(100, &[a("127.0.0.1:5433")]),
            Ready::HeldByOther(
                a("127.0.0.1:5433"),
                Holder {
                    pid: 200,
                    name: "p200".into()
                }
            )
        );
    }

    #[test]
    fn wildcard_listeners_serve_specific_addresses() {
        let s = Snapshot::from_parts(
            vec![sock("0.0.0.0:80", 100), sock("[::]:81", 100)],
            parents(),
        );
        assert_eq!(
            s.ready(100, &[a("127.0.0.1:80"), a("[::1]:81")]),
            Ready::Yes
        );
        // A specific listener does not serve a different specific address.
        let s = Snapshot::from_parts(vec![sock("127.0.0.2:80", 100)], parents());
        assert_eq!(
            s.ready(100, &[a("127.0.0.1:80")]),
            Ready::Waiting(vec![a("127.0.0.1:80")])
        );
    }

    #[test]
    fn ours_wins_when_an_unrelated_wildcard_also_listens() {
        let s = Snapshot::from_parts(
            vec![sock("0.0.0.0:80", 200), sock("127.0.0.1:80", 101)],
            parents(),
        );
        assert_eq!(s.ready(100, &[a("127.0.0.1:80")]), Ready::Yes);
    }

    #[test]
    fn holder_lookup() {
        let s = Snapshot::from_parts(vec![sock("0.0.0.0:5432", 200)], parents());
        assert_eq!(
            s.holder(a("127.0.0.1:5432")),
            Some(Holder {
                pid: 200,
                name: "p200".into()
            })
        );
        assert_eq!(s.holder(a("127.0.0.1:5433")), None);
    }

    #[test]
    fn shared_socket_reports_the_lowest_pid() {
        let s = Snapshot::from_parts(
            vec![sock("127.0.0.1:9", 300), sock("127.0.0.1:9", 200)],
            parents(),
        );
        assert_eq!(s.holder(a("127.0.0.1:9")).map(|h| h.pid), Some(200));
    }

    #[test]
    fn parent_cycle_does_not_hang() {
        let s = Snapshot::from_parts(
            vec![sock("127.0.0.1:1", 5)],
            HashMap::from([(5, 6), (6, 5)]),
        );
        assert_eq!(
            s.ready(100, &[a("127.0.0.1:1")]),
            Ready::HeldByOther(
                a("127.0.0.1:1"),
                Holder {
                    pid: 5,
                    name: "p5".into()
                }
            )
        );
    }
}
