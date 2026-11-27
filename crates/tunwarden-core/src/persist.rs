//! The observer that persists what supervisors report: log lines go to the
//! tunnel's log file, state changes to the activity log, and every status to
//! the state file.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::fsutil::timestamp;
use crate::logs::{Activity, ActivityEvent, Logs};
use crate::state::StateStore;
use crate::supervisor::{LogSource, Observer, State, Status};

pub struct Persister {
    pub logs: Logs,
    pub activity: Activity,
    pub state: StateStore,
    last: Mutex<HashMap<String, State>>,
    error: Mutex<Option<String>>,
}

impl Persister {
    pub fn new(logs: Logs, activity: Activity, state: StateStore) -> Persister {
        Persister {
            logs,
            activity,
            state,
            last: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
        }
    }

    /// The most recent write error, for `doctor`. Writing never stops a tunnel.
    pub fn last_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Writes a log line.
    pub fn record_log(&self, tunnel: &str, source: LogSource, line: &str) {
        self.note(
            self.logs.write(tunnel, source, line),
            &format!("the log of {tunnel}"),
        );
    }

    /// Saves a status, and returns the activity event if the state changed.
    pub fn record_status(&self, status: &Status) -> Option<ActivityEvent> {
        let from = self
            .last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(status.name.clone(), status.state)
            .unwrap_or(State::Stopped);
        let event = (from != status.state).then(|| ActivityEvent {
            time: timestamp(SystemTime::now()),
            tunnel: status.name.clone(),
            from,
            to: status.state,
            reason: status.reason,
            message: status
                .last_error
                .clone()
                .filter(|_| status.state != State::Running),
        });
        if let Some(event) = &event {
            self.note(self.activity.record(event.clone()), "the activity log");
        }
        self.note(self.state.record(status), "the state file");
        event
    }

    fn note(&self, result: std::io::Result<()>, what: &str) {
        if let Err(e) = result {
            *self.error.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("cannot write {what}: {e}"));
        }
    }
}

impl Observer for Persister {
    fn log(&self, tunnel: &str, source: LogSource, line: &str) {
        self.record_log(tunnel, source, line);
    }

    fn status(&self, status: &Status) {
        self.record_status(status);
    }
}
