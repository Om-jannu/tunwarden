//! File helpers for the state directory: private permissions, atomic writes
//! and size-based rotation (PRD section 9 Security and Reliability, FR-28).
//!
//! On Unix, directories are created with mode 0700 and files with 0600. On
//! Windows the state directory lives under `%LOCALAPPDATA%`, which is already
//! private to the user; explicit ACLs are not set yet.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Creates `dir` and its parents. The final directory is made private.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Opens `path` for appending, creating it as a private file.
pub fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Replaces `path` with `bytes` so that readers see either the old or the new
/// content, never a partial file: write a private temp file in the same
/// directory, flush it to disk, then rename it over the original.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(dir) = File::open(dir) {
            // Makes the rename itself durable.
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// An append-only file that moves to `<name>.1` once it passes `max_bytes`,
/// replacing the previous `<name>.1` (FR-28).
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    pub fn new(path: PathBuf, max_bytes: u64) -> RotatingFile {
        RotatingFile {
            path,
            max_bytes,
            file: None,
            size: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends `bytes`, rotating first if they would push the file past the limit.
    /// A single write larger than the limit still goes into one file.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.file.is_none() {
            let file = open_append(&self.path)?;
            self.size = file.metadata()?.len();
            self.file = Some(file);
        }
        if self.size > 0 && self.size + bytes.len() as u64 > self.max_bytes {
            self.file = None;
            std::fs::rename(&self.path, previous(&self.path))?;
            self.file = Some(open_append(&self.path)?);
            self.size = 0;
        }
        let file = self.file.as_mut().expect("opened above");
        file.write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }
}

/// The path of the rotated file: `x.log` → `x.log.1`.
pub fn previous(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// A timestamp such as `2026-11-26T09:12:03.120Z`.
pub fn timestamp(time: SystemTime) -> String {
    humantime::format_rfc3339_millis(time).to_string()
}

/// [`timestamp`] for the current time.
pub fn format_timestamp_now() -> String {
    timestamp(SystemTime::now())
}

pub fn parse_timestamp(text: &str) -> Option<SystemTime> {
    humantime::parse_rfc3339(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn files_and_dirs_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a/b");
        private_dir(&sub).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&sub), 0o700);
        let f = sub.join("x.log");
        open_append(&f).unwrap();
        assert_eq!(mode(&f), 0o600);
        let s = sub.join("s.json");
        write_atomic(&s, b"{}").unwrap();
        assert_eq!(mode(&s), 0o600);
    }

    #[test]
    fn rotation_keeps_one_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.log");
        let mut f = RotatingFile::new(path.clone(), 10);
        f.append(b"aaaa\n").unwrap();
        f.append(b"bbbb\n").unwrap(); // exactly 10 bytes: no rotation yet
        f.append(b"cccc\n").unwrap(); // rotates
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cccc\n");
        assert_eq!(
            std::fs::read_to_string(previous(&path)).unwrap(),
            "aaaa\nbbbb\n"
        );
        f.append(b"dddd\n").unwrap();
        f.append(b"eeee\n").unwrap(); // rotates again; the oldest lines are dropped
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "eeee\n");
        assert_eq!(
            std::fs::read_to_string(previous(&path)).unwrap(),
            "cccc\ndddd\n"
        );
    }

    #[test]
    fn rotation_counts_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.log");
        std::fs::write(&path, b"123456789\n").unwrap();
        let mut f = RotatingFile::new(path.clone(), 12);
        f.append(b"x\n").unwrap();
        f.append(b"y\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "y\n");
    }

    #[test]
    fn timestamps_round_trip() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_795_600_803_120);
        let text = timestamp(t);
        assert_eq!(text, "2026-11-25T10:00:03.120Z");
        assert_eq!(parse_timestamp(&text), Some(t));
    }
}
