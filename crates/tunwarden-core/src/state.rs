//! The state file, `<state>/state.json` (design 8.2).
//!
//! It records what each tunnel's user asked for (`desired`), its last known
//! state and its process, so a new daemon can sweep leftover processes after a
//! crash (FR-52) and restore tunnels (FR-15). It is written atomically on every
//! change and only read when the daemon starts; at runtime the supervisors are
//! the source of truth.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::failure::FailureReason;
use crate::fsutil::{private_dir, timestamp, write_atomic};
use crate::process::ProcessRecord;
use crate::supervisor::{State, Status};

/// The state file format version.
pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Desired {
    Running,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub started_at: String,
    pub protocol: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelRecord {
    pub desired: Desired,
    pub state: State,
    #[serde(default)]
    pub reason: Option<FailureReason>,
    #[serde(default)]
    pub process: Option<ProcessRecord>,
    #[serde(default)]
    pub restarts: u32,
    pub since: String,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    #[serde(default)]
    pub daemon: Option<DaemonInfo>,
    #[serde(default)]
    pub tunnels: BTreeMap<String, TunnelRecord>,
}

impl Default for StateFile {
    fn default() -> Self {
        StateFile {
            version: STATE_VERSION,
            daemon: None,
            tunnels: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{path} was written by a newer Tunwarden (state version {found}, this one supports {STATE_VERSION}); upgrade Tunwarden"
    )]
    Newer { path: PathBuf, found: u32 },
}

/// What [`StateStore::open`] found from the previous run.
#[derive(Debug)]
pub struct Previous {
    pub state: StateFile,
    /// Set when the old file could not be read and was moved aside.
    pub note: Option<String>,
}

pub struct StateStore {
    path: PathBuf,
    current: Mutex<StateFile>,
}

impl StateStore {
    /// Opens `<state_dir>/state.json`. A missing file is an empty state. A file
    /// that does not parse is renamed to `state.json.corrupt` and reported in
    /// [`Previous::note`], so a damaged file never stops the daemon.
    pub fn open(state_dir: &Path) -> Result<(StateStore, Previous), StateError> {
        let path = state_dir.join("state.json");
        let io_err = |source| StateError::Io {
            path: path.clone(),
            source,
        };
        private_dir(state_dir).map_err(io_err)?;
        let (state, note) = match std::fs::read(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (StateFile::default(), None),
            Err(e) => return Err(io_err(e)),
            Ok(bytes) => match parse(&bytes) {
                Ok(state) if state.version > STATE_VERSION => {
                    return Err(StateError::Newer {
                        path,
                        found: state.version,
                    });
                }
                Ok(state) => (state, None),
                Err(e) => {
                    let aside = state_dir.join("state.json.corrupt");
                    std::fs::rename(&path, &aside).map_err(io_err)?;
                    let note = format!(
                        "{} could not be read ({e}); it was moved to {} and Tunwarden started with no saved state",
                        path.display(),
                        aside.display()
                    );
                    (StateFile::default(), Some(note))
                }
            },
        };
        let store = StateStore {
            path,
            current: Mutex::new(state.clone()),
        };
        Ok((store, Previous { state, note }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current contents.
    pub fn snapshot(&self) -> StateFile {
        self.lock().clone()
    }

    /// Updates one tunnel from its status and writes the file.
    pub fn record(&self, status: &Status) -> io::Result<()> {
        self.change(|s| {
            s.tunnels.insert(status.name.clone(), record_of(status));
        })
    }

    /// Forgets a tunnel, for example after it was removed from the config.
    pub fn remove(&self, tunnel: &str) -> io::Result<()> {
        self.change(|s| {
            s.tunnels.remove(tunnel);
        })
    }

    pub fn set_daemon(&self, daemon: Option<DaemonInfo>) -> io::Result<()> {
        self.change(|s| s.daemon = daemon)
    }

    /// Sets every tunnel's desired state, for `daemon stop` without `--keep` (FR-15).
    pub fn set_all_desired(&self, desired: Desired) -> io::Result<()> {
        self.change(|s| s.tunnels.values_mut().for_each(|t| t.desired = desired))
    }

    /// Marks every tunnel stopped with no process, after the daemon stopped
    /// them or swept what a crashed daemon left. `desired` overrides what the
    /// user asked for; `None` keeps it, so the tunnels are restored next time.
    pub fn mark_all_stopped(&self, desired: Option<Desired>) -> io::Result<()> {
        self.change(|s| {
            for t in s.tunnels.values_mut() {
                if t.state != State::Stopped {
                    t.since = now();
                }
                t.state = State::Stopped;
                t.process = None;
                t.reason = None;
                if let Some(d) = desired {
                    t.desired = d;
                }
            }
        })
    }

    fn change(&self, f: impl FnOnce(&mut StateFile)) -> io::Result<()> {
        let mut state = self.lock();
        f(&mut state);
        let mut bytes = serde_json::to_vec_pretty(&*state).map_err(io::Error::other)?;
        bytes.push(b'\n');
        // Written while holding the lock, so writes never reorder.
        write_atomic(&self.path, &bytes)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StateFile> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn parse(bytes: &[u8]) -> Result<StateFile, String> {
    // Read the version first, so a newer format with other fields is reported
    // as "newer" rather than "corrupt".
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let v: Version = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if v.version > STATE_VERSION {
        return Ok(StateFile {
            version: v.version,
            ..StateFile::default()
        });
    }
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

fn record_of(status: &Status) -> TunnelRecord {
    TunnelRecord {
        desired: if status.desired_running {
            Desired::Running
        } else {
            Desired::Stopped
        },
        state: status.state,
        reason: status.reason,
        process: status.process,
        restarts: status.restarts,
        since: timestamp(status.since),
        last_error: status.last_error.clone(),
    }
}

/// A timestamp for "now", for [`DaemonInfo::started_at`].
pub fn now() -> String {
    timestamp(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, state: State, desired: bool) -> Status {
        Status {
            name: name.into(),
            state,
            reason: (state == State::Failed).then_some(FailureReason::PortConflict),
            last_error: (state == State::Failed).then(|| "port 5433 is in use".to_owned()),
            process: (state == State::Running).then_some(ProcessRecord {
                pid: 4188,
                pgid: 4188,
                start_time: 1_795_600_803,
            }),
            restarts: 1,
            retries: 0,
            since: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_795_600_803),
            desired_running: desired,
            pending_restart: false,
        }
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, prev) = StateStore::open(dir.path()).unwrap();
        assert_eq!(prev.state, StateFile::default());
        store
            .set_daemon(Some(DaemonInfo {
                pid: 4120,
                started_at: "2026-11-26T09:00:00.000Z".into(),
                protocol: 1,
            }))
            .unwrap();
        store
            .record(&status("staging-db", State::Running, true))
            .unwrap();
        store.record(&status("redis", State::Failed, true)).unwrap();

        let text = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
        assert!(text.contains(r#""desired": "running""#), "{text}");
        assert!(text.contains(r#""reason": "port conflict""#), "{text}");
        assert!(
            text.contains(r#""since": "2026-11-25T10:00:03.000Z""#),
            "{text}"
        );

        let (_, prev) = StateStore::open(dir.path()).unwrap();
        assert_eq!(prev.note, None);
        assert_eq!(prev.state, store.snapshot());
        let db = &prev.state.tunnels["staging-db"];
        assert_eq!(db.process.unwrap().pid, 4188);
        assert_eq!(db.state, State::Running);
    }

    #[test]
    fn desired_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = StateStore::open(dir.path()).unwrap();
        store.record(&status("a", State::Running, true)).unwrap();
        store.record(&status("b", State::Stopped, false)).unwrap();
        store.set_all_desired(Desired::Stopped).unwrap();
        assert!(
            store
                .snapshot()
                .tunnels
                .values()
                .all(|t| t.desired == Desired::Stopped)
        );
        store.remove("a").unwrap();
        let (_, prev) = StateStore::open(dir.path()).unwrap();
        assert_eq!(prev.state.tunnels.keys().collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("state.json"), b"{ not json").unwrap();
        let (store, prev) = StateStore::open(dir.path()).unwrap();
        assert_eq!(prev.state, StateFile::default());
        let note = prev.note.unwrap();
        assert!(
            note.contains("could not be read") && note.contains("state.json.corrupt"),
            "{note}"
        );
        assert!(dir.path().join("state.json.corrupt").exists());
        store.record(&status("a", State::Stopped, false)).unwrap();
        assert!(dir.path().join("state.json").exists());
    }

    #[test]
    fn a_newer_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("state.json"),
            br#"{"version": 9, "new_field": []}"#,
        )
        .unwrap();
        let err = StateStore::open(dir.path()).err().unwrap();
        assert!(
            err.to_string()
                .contains("written by a newer Tunwarden (state version 9"),
            "{err}"
        );
        assert!(
            dir.path().join("state.json").exists(),
            "the newer file is left alone"
        );
    }
}
