//! Waking from sleep and network changes (PRD FR-19, design 11).
//!
//! Both are found by polling, the same way on every OS and without native
//! notification APIs:
//!
//! - **Wake.** A timer ticks every few seconds. If the wall clock moved much
//!   further than the tick between two ticks, the machine was asleep (the
//!   process does not run while it sleeps). This also fires when the clock is
//!   set forward by a lot, which is harmless: a wake only makes tunnels check
//!   and retry sooner.
//! - **Network.** The set of non-loopback interface addresses is compared on
//!   each tick. A change (new Wi-Fi, VPN up or down, cable) is reported once
//!   the addresses have stayed the same for one more tick, so a DHCP renewal
//!   that takes a moment counts once.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc;

/// How often the watcher looks.
pub const TICK: Duration = Duration::from_secs(5);
/// How much more than one tick the wall clock must move to count as a wake.
pub const WAKE_GAP: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemEvent {
    /// The machine woke after sleeping for about this long.
    Woke { slept: Duration },
    /// The machine's network addresses changed.
    NetworkChanged,
}

impl SystemEvent {
    pub fn describe(&self) -> String {
        match self {
            SystemEvent::Woke { slept } => {
                format!("woke from sleep (about {} s)", slept.as_secs())
            }
            SystemEvent::NetworkChanged => "the network changed".to_owned(),
        }
    }
}

/// Finds wake-ups from the wall-clock time between ticks.
#[derive(Debug)]
pub struct WakeDetector {
    last: SystemTime,
}

impl WakeDetector {
    pub fn new(now: SystemTime) -> Self {
        WakeDetector { last: now }
    }

    /// Called on each tick with the current wall-clock time. Returns how long
    /// the machine slept, if the gap was too large to be a normal tick.
    pub fn tick(&mut self, now: SystemTime, tick: Duration) -> Option<Duration> {
        // A clock set backwards gives an error here: not a wake.
        let elapsed = now.duration_since(self.last).ok();
        self.last = now;
        let elapsed = elapsed?;
        (elapsed > tick + WAKE_GAP).then(|| elapsed.saturating_sub(tick))
    }
}

/// Finds settled changes in the set of addresses.
#[derive(Debug)]
pub struct NetworkDetector {
    reported: BTreeSet<IpAddr>,
    pending: Option<BTreeSet<IpAddr>>,
}

impl NetworkDetector {
    pub fn new(current: BTreeSet<IpAddr>) -> Self {
        NetworkDetector {
            reported: current,
            pending: None,
        }
    }

    /// Called on each tick with the current addresses. Returns true once a
    /// change has stayed the same for a whole tick.
    pub fn tick(&mut self, current: BTreeSet<IpAddr>) -> bool {
        if current == self.reported {
            self.pending = None;
            return false;
        }
        if self.pending.as_ref() == Some(&current) {
            self.reported = current;
            self.pending = None;
            return true;
        }
        self.pending = Some(current);
        false
    }
}

/// The machine's addresses, without loopback. Empty if they cannot be read,
/// which then simply never changes.
pub fn addresses(networks: &mut sysinfo::Networks) -> BTreeSet<IpAddr> {
    networks.refresh(true);
    networks
        .values()
        .flat_map(|n| n.ip_networks())
        .map(|n| n.addr)
        .filter(|a| !a.is_loopback() && !a.is_unspecified())
        .collect()
}

/// Watches until `tx` is closed. Runs on a blocking thread for the address
/// lookups, which are system calls.
pub async fn watch(tx: mpsc::UnboundedSender<SystemEvent>) {
    let mut networks = sysinfo::Networks::new_with_refreshed_list().ok();
    let mut wake = WakeDetector::new(SystemTime::now());
    let mut net = NetworkDetector::new(match &mut networks {
        Some(n) => addresses(n),
        None => BTreeSet::new(),
    });
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        interval.tick().await;
        if tx.is_closed() {
            return;
        }
        if let Some(slept) = wake.tick(SystemTime::now(), TICK) {
            let _ = tx.send(SystemEvent::Woke { slept });
        }
        if let Some(n) = networks.take() {
            let lookup = tokio::task::spawn_blocking(move || {
                let mut n = n;
                let current = addresses(&mut n);
                (n, current)
            });
            let Ok((n, current)) = lookup.await else {
                return; // the runtime is shutting down
            };
            networks = Some(n);
            if net.tick(current) {
                let _ = tx.send(SystemEvent::NetworkChanged);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs)
    }

    #[test]
    fn a_long_gap_between_ticks_is_a_wake() {
        let mut w = WakeDetector::new(at(0));
        assert_eq!(w.tick(at(5), TICK), None, "a normal tick");
        assert_eq!(w.tick(at(12), TICK), None, "a late tick under load");
        assert_eq!(
            w.tick(at(12 + 3600), TICK),
            Some(Duration::from_secs(3595)),
            "an hour asleep"
        );
        assert_eq!(w.tick(at(3617), TICK), None, "back to normal");
        assert_eq!(w.tick(at(100), TICK), None, "a clock set backwards");
        assert_eq!(w.tick(at(105), TICK), None, "and then normal again");
    }

    fn set(addrs: &[&str]) -> BTreeSet<IpAddr> {
        addrs.iter().map(|a| a.parse().unwrap()).collect()
    }

    #[test]
    fn a_network_change_counts_once_it_settles() {
        let mut n = NetworkDetector::new(set(&["192.168.1.5"]));
        assert!(!n.tick(set(&["192.168.1.5"])));
        // Wi-Fi drops, then a new network hands out an address.
        assert!(!n.tick(set(&[])), "first sight of a change");
        assert!(!n.tick(set(&["10.0.0.7"])), "still changing");
        assert!(n.tick(set(&["10.0.0.7"])), "settled");
        assert!(!n.tick(set(&["10.0.0.7"])), "reported once");
        // A blip that goes back is not a change.
        assert!(!n.tick(set(&["10.0.0.7", "100.64.0.2"])));
        assert!(!n.tick(set(&["10.0.0.7"])));
        assert!(!n.tick(set(&["10.0.0.7"])));
        // A VPN coming up is.
        assert!(!n.tick(set(&["10.0.0.7", "100.64.0.2"])));
        assert!(n.tick(set(&["10.0.0.7", "100.64.0.2"])));
    }

    #[test]
    fn addresses_skip_loopback() {
        let Ok(mut networks) = sysinfo::Networks::new_with_refreshed_list() else {
            return;
        };
        assert!(addresses(&mut networks).iter().all(|a| !a.is_loopback()));
    }
}
