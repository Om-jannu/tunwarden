//! Per-tunnel logs and the activity log (PRD FR-27 to FR-30, design 8.3).
//!
//! - `<state>/logs/<tunnel>.log`: the command's output and Tunwarden's own
//!   events, one line each: `2026-11-26T09:12:03.120Z out|err|tw <text>`.
//! - `<state>/activity.jsonl`: one JSON object per state change across all
//!   tunnels, plus the most recent events in memory.
//!
//! Both rotate at 5 MB and keep one previous file.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::failure::FailureReason;
use crate::fsutil::{RotatingFile, previous, private_dir, timestamp};
use crate::supervisor::{LogSource, State};

/// Default rotation size (FR-28).
pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// How many activity events are kept in memory (design 8.3).
pub const RECENT_EVENTS: usize = 500;

impl LogSource {
    pub const fn tag(self) -> &'static str {
        match self {
            LogSource::Stdout => "out",
            LogSource::Stderr => "err",
            LogSource::Tunwarden => "tw",
        }
    }
}

/// One log line as written to the file.
pub fn format_line(time: SystemTime, source: LogSource, text: &str) -> String {
    // A line break inside the text would look like a new entry.
    let text = text.replace(['\r', '\n'], " ");
    format!("{} {} {}\n", timestamp(time), source.tag(), text)
}

/// The per-tunnel log files.
pub struct Logs {
    dir: PathBuf,
    max_bytes: u64,
    open: Mutex<HashMap<String, RotatingFile>>,
}

impl Logs {
    /// Uses `<state_dir>/logs`, creating it as a private directory.
    pub fn new(state_dir: &Path, max_bytes: u64) -> io::Result<Logs> {
        let dir = state_dir.join("logs");
        private_dir(&dir)?;
        Ok(Logs {
            dir,
            max_bytes,
            open: Mutex::new(HashMap::new()),
        })
    }

    pub fn path(&self, tunnel: &str) -> PathBuf {
        self.dir.join(format!("{tunnel}.log"))
    }

    pub fn write(&self, tunnel: &str, source: LogSource, text: &str) -> io::Result<()> {
        let line = format_line(SystemTime::now(), source, text);
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let file = open
            .entry(tunnel.to_owned())
            .or_insert_with(|| RotatingFile::new(self.path(tunnel), self.max_bytes));
        file.append(line.as_bytes())
    }

    /// Closes the tunnel's file, for example after it was removed from the config.
    pub fn close(&self, tunnel: &str) {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(tunnel);
    }

    /// The last `n` lines of a tunnel's log, oldest first (FR-29).
    pub fn tail(&self, tunnel: &str, n: usize) -> io::Result<Vec<String>> {
        tail(&self.path(tunnel), n)
    }
}

/// The last `n` lines of `path`, continuing into the rotated `path.1` if the
/// current file has fewer. A missing file counts as empty.
pub fn tail(path: &Path, n: usize) -> io::Result<Vec<String>> {
    let mut lines = last_lines(path, n)?;
    if lines.len() < n {
        let mut older = last_lines(&previous(path), n - lines.len())?;
        older.append(&mut lines);
        lines = older;
    }
    Ok(lines)
}

/// Reads backwards from the end in chunks, so a 5 MB file is not read whole.
fn last_lines(path: &Path, n: usize) -> io::Result<Vec<String>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    const CHUNK: u64 = 64 * 1024;
    let len = file.metadata()?.len();
    let mut start = len;
    let mut buf: Vec<u8> = Vec::new();
    // Stop once the buffer holds more than `n` line breaks: the first partial line is dropped.
    while start > 0 && buf.iter().filter(|&&b| b == b'\n').count() <= n {
        let step = CHUNK.min(start);
        start -= step;
        file.seek(SeekFrom::Start(start))?;
        let mut chunk = vec![0; step as usize];
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&buf);
        buf = chunk;
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // may be cut in the middle
    }
    let skip = lines.len().saturating_sub(n);
    Ok(lines[skip..].iter().map(|l| l.to_string()).collect())
}

/// A state change, for the activity view (FR-30).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub time: String,
    pub tunnel: String,
    pub from: State,
    pub to: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<FailureReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The activity log: a rotating JSON Lines file plus recent events in memory.
pub struct Activity {
    file: Mutex<RotatingFile>,
    recent: Mutex<VecDeque<ActivityEvent>>,
}

impl Activity {
    pub fn new(state_dir: &Path, max_bytes: u64) -> io::Result<Activity> {
        private_dir(state_dir)?;
        let path = state_dir.join("activity.jsonl");
        let recent = load_recent(&path)?;
        Ok(Activity {
            file: Mutex::new(RotatingFile::new(path, max_bytes)),
            recent: Mutex::new(recent),
        })
    }

    pub fn record(&self, event: ActivityEvent) -> io::Result<()> {
        let mut line = serde_json::to_string(&event).map_err(io::Error::other)?;
        line.push('\n');
        {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            if recent.len() == RECENT_EVENTS {
                recent.pop_front();
            }
            recent.push_back(event);
        }
        self.file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .append(line.as_bytes())
    }

    /// Up to `limit` most recent events, oldest first.
    pub fn recent(&self, limit: usize) -> Vec<ActivityEvent> {
        let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        let skip = recent.len().saturating_sub(limit);
        recent.iter().skip(skip).cloned().collect()
    }
}

/// Recent events from an earlier run, so the activity view survives a restart.
/// Lines that do not parse are skipped.
fn load_recent(path: &Path) -> io::Result<VecDeque<ActivityEvent>> {
    let lines = tail(path, RECENT_EVENTS)?;
    Ok(lines
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn line_format() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_600_803_120);
        assert_eq!(
            format_line(t, LogSource::Stderr, "Permission denied (publickey)."),
            "2026-11-25T10:00:03.120Z err Permission denied (publickey).\n"
        );
        assert_eq!(
            format_line(t, LogSource::Tunwarden, "a\nb"),
            "2026-11-25T10:00:03.120Z tw a b\n"
        );
    }

    #[test]
    fn write_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Logs::new(dir.path(), DEFAULT_MAX_BYTES).unwrap();
        for i in 0..5 {
            logs.write("db", LogSource::Stdout, &format!("line {i}"))
                .unwrap();
        }
        logs.write("other", LogSource::Tunwarden, "x").unwrap();
        let last = logs.tail("db", 2).unwrap();
        assert_eq!(last.len(), 2);
        assert!(
            last[0].ends_with(" out line 3") && last[1].ends_with(" out line 4"),
            "{last:?}"
        );
        assert_eq!(logs.tail("db", 100).unwrap().len(), 5);
        assert!(logs.tail("missing", 10).unwrap().is_empty());
        assert_eq!(logs.path("db"), dir.path().join("logs").join("db.log"));
    }

    #[test]
    fn tail_spans_the_rotated_file() {
        let dir = tempfile::tempdir().unwrap();
        // Each line is 24 (timestamp) + 4 (" tw ") + 2 ("nn") + 1 = 31 bytes.
        let logs = Logs::new(dir.path(), 100).unwrap();
        for i in 10..17 {
            logs.write("t", LogSource::Tunwarden, &i.to_string())
                .unwrap();
        }
        let numbers = |n: usize| -> Vec<String> {
            logs.tail("t", n)
                .unwrap()
                .iter()
                .map(|l| l.rsplit(' ').next().unwrap().to_owned())
                .collect()
        };
        assert_eq!(
            numbers(100),
            ["13", "14", "15", "16"],
            "current file plus one previous file"
        );
        assert_eq!(numbers(2), ["15", "16"]);
    }

    #[test]
    fn tail_of_a_large_file_reads_from_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let mut text = String::new();
        for i in 0..20_000 {
            text.push_str(&format!("line {i:05} {}\n", "x".repeat(20)));
        }
        std::fs::write(&path, &text).unwrap();
        let last = tail(&path, 3).unwrap();
        assert_eq!(last.len(), 3);
        assert!(last[2].starts_with("line 19999"));
        assert!(last[0].starts_with("line 19997"));
    }

    #[test]
    fn activity_keeps_recent_events_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let event = |to: State| ActivityEvent {
            time: "2026-11-25T10:00:03.120Z".into(),
            tunnel: "db".into(),
            from: State::Starting,
            to,
            reason: (to == State::Failed).then_some(FailureReason::AuthExpired),
            message: None,
        };
        {
            let a = Activity::new(dir.path(), DEFAULT_MAX_BYTES).unwrap();
            a.record(event(State::Running)).unwrap();
            a.record(event(State::Failed)).unwrap();
            assert_eq!(a.recent(1), [event(State::Failed)]);
        }
        let text = std::fs::read_to_string(dir.path().join("activity.jsonl")).unwrap();
        assert!(
            text.contains(r#""to":"failed","reason":"auth expired""#),
            "{text}"
        );
        let again = Activity::new(dir.path(), DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(
            again.recent(10),
            [event(State::Running), event(State::Failed)]
        );
    }

    #[test]
    fn activity_memory_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let a = Activity::new(dir.path(), DEFAULT_MAX_BYTES).unwrap();
        for i in 0..RECENT_EVENTS + 10 {
            a.record(ActivityEvent {
                time: i.to_string(),
                tunnel: "t".into(),
                from: State::Stopped,
                to: State::Starting,
                reason: None,
                message: None,
            })
            .unwrap();
        }
        let recent = a.recent(usize::MAX);
        assert_eq!(recent.len(), RECENT_EVENTS);
        assert_eq!(recent[0].time, "10");
    }
}
