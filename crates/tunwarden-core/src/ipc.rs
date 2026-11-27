//! The local control connection (design 5.2).
//!
//! - macOS and Linux: a Unix domain socket in the private state directory, mode
//!   0600. macOS limits socket paths to about 104 bytes, so a long state
//!   directory falls back to a short path in the per-user temp directory.
//! - Windows: a named pipe that refuses remote clients. Restricting it to the
//!   current user with an explicit security descriptor waits for spike S3.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncWrite};

/// A bidirectional byte stream to the other side.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Where the daemon listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Socket(PathBuf),
    Pipe(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Socket(p) => write!(f, "{}", p.display()),
            Endpoint::Pipe(name) => f.write_str(name),
        }
    }
}

/// Socket paths longer than this use the short fallback.
const MAX_SOCKET_PATH: usize = 100;

/// The endpoint for a state directory. Client and daemon compute the same one.
pub fn endpoint(state_dir: &Path) -> Endpoint {
    if cfg!(windows) {
        return Endpoint::Pipe(format!(r"\\.\pipe\tunwarden-{:016x}", fnv(state_dir)));
    }
    let preferred = state_dir.join("tunwarden.sock");
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
        return Endpoint::Socket(preferred);
    }
    Endpoint::Socket(short_dir().join(format!("{:016x}.sock", fnv(state_dir))))
}

/// A short private directory for sockets whose preferred path is too long.
fn short_dir() -> PathBuf {
    #[cfg(unix)]
    let user = nix::unistd::getuid().as_raw().to_string();
    #[cfg(not(unix))]
    let user = "user".to_owned();
    std::env::temp_dir().join(format!("tunwarden-{user}"))
}

/// FNV-1a, a small stable hash for names derived from paths.
fn fnv(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Whether a connect error means "no daemon is listening".
pub fn is_not_running(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub async fn connect(endpoint: &Endpoint) -> io::Result<Box<dyn Stream>> {
    platform::connect(endpoint).await
}

pub struct Listener {
    inner: platform::Listener,
}

impl Listener {
    /// Binds the endpoint. Only the holder of the daemon lock may call this,
    /// because it removes a stale socket file first.
    pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
        Ok(Listener {
            inner: platform::Listener::bind(endpoint)?,
        })
    }

    pub async fn accept(&mut self) -> io::Result<Box<dyn Stream>> {
        self.inner.accept().await
    }

    /// Removes the socket file, if there is one.
    pub fn cleanup(&self) {
        self.inner.cleanup();
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    pub async fn connect(endpoint: &Endpoint) -> io::Result<Box<dyn Stream>> {
        match endpoint {
            Endpoint::Socket(path) => Ok(Box::new(UnixStream::connect(path).await?)),
            Endpoint::Pipe(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "named pipes are Windows-only",
            )),
        }
    }

    pub struct Listener {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
            use std::os::unix::fs::PermissionsExt;
            let Endpoint::Socket(path) = endpoint else {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "named pipes are Windows-only",
                ));
            };
            if let Some(dir) = path.parent() {
                crate::fsutil::private_dir(dir)?;
            }
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Listener {
                listener,
                path: path.clone(),
            })
        }

        pub async fn accept(&mut self) -> io::Result<Box<dyn Stream>> {
            let (stream, _) = self.listener.accept().await?;
            Ok(Box::new(stream))
        }

        pub fn cleanup(&self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::time::Duration;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    const ERROR_PIPE_BUSY: i32 = 231;

    fn name(endpoint: &Endpoint) -> io::Result<&str> {
        match endpoint {
            Endpoint::Pipe(name) => Ok(name),
            Endpoint::Socket(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unix sockets are not used on Windows",
            )),
        }
    }

    pub async fn connect(endpoint: &Endpoint) -> io::Result<Box<dyn Stream>> {
        let name = name(endpoint)?;
        for _ in 0..50 {
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the daemon's pipe stayed busy",
        ))
    }

    pub struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
            let name = name(endpoint)?.to_owned();
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&name)?;
            Ok(Listener { name, next })
        }

        pub async fn accept(&mut self) -> io::Result<Box<dyn Stream>> {
            self.next.connect().await?;
            let fresh = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?;
            let connected = std::mem::replace(&mut self.next, fresh);
            Ok(Box::new(connected))
        }

        pub fn cleanup(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn short_and_long_state_dirs() {
        let short = endpoint(Path::new("/home/me/.local/state/tunwarden"));
        assert_eq!(
            short,
            Endpoint::Socket("/home/me/.local/state/tunwarden/tunwarden.sock".into())
        );
        let long_dir = PathBuf::from(format!("/home/me/{}", "x".repeat(120)));
        let Endpoint::Socket(long) = endpoint(&long_dir) else {
            panic!()
        };
        assert!(
            long.as_os_str().len() <= MAX_SOCKET_PATH + 30,
            "{}",
            long.display()
        );
        assert!(long.extension().is_some_and(|e| e == "sock"));
        assert_eq!(
            endpoint(&long_dir),
            Endpoint::Socket(long),
            "stable for client and daemon"
        );
    }

    #[test]
    fn different_dirs_get_different_names() {
        assert_ne!(fnv(Path::new("/a")), fnv(Path::new("/b")));
    }
}
