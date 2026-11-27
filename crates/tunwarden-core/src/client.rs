//! Talking to the daemon: connect, handshake, requests, notifications, and
//! starting the daemon on demand (design 5.1, PRD FR-53).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

use crate::fsutil::{open_append, private_dir};
use crate::ipc::{self, Stream};
use crate::logs::tail;
use crate::proto::{
    Hello, HelloResult, Notification, PROTOCOL, RPC_PROTOCOL_MISMATCH, Request, Response, RpcError,
    method,
};

/// How long one request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a freshly started daemon gets to accept connections.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Tunwarden daemon is not running")]
    NotRunning,
    #[error("{0}")]
    Rpc(RpcError),
    #[error("lost the connection to the daemon: {0}")]
    Io(io::Error),
    #[error("the daemon did not answer within {} seconds", .0.as_secs())]
    Timeout(Duration),
    #[error("could not start the daemon: {0}")]
    Start(String),
    #[error("unexpected reply from the daemon: {0}")]
    Protocol(String),
}

impl ClientError {
    /// True for a version mismatch with the running daemon (FR-53).
    pub fn is_protocol_mismatch(&self) -> bool {
        matches!(self, ClientError::Rpc(e) if e.code == RPC_PROTOCOL_MISMATCH)
    }

    pub fn hint(&self) -> Option<&str> {
        match self {
            ClientError::Rpc(e) => e.hint(),
            ClientError::NotRunning => Some("start it with `tunwarden daemon start`"),
            _ => None,
        }
    }
}

pub struct Client {
    reader: BufReader<ReadHalf<Box<dyn Stream>>>,
    writer: WriteHalf<Box<dyn Stream>>,
    next_id: u64,
    pending: VecDeque<Notification>,
    hello: Option<HelloResult>,
}

impl Client {
    /// Connects to the daemon for `state_dir` and does the handshake.
    /// `client` names the caller in mismatch messages, for example `cli`.
    pub async fn connect(state_dir: &Path, client: &str) -> Result<Client, ClientError> {
        let stream = ipc::connect(&ipc::endpoint(state_dir)).await.map_err(|e| {
            if ipc::is_not_running(&e) {
                ClientError::NotRunning
            } else {
                ClientError::Io(e)
            }
        })?;
        let (read, writer) = tokio::io::split(stream);
        let mut c = Client {
            reader: BufReader::new(read),
            writer,
            next_id: 1,
            pending: VecDeque::new(),
            hello: None,
        };
        let hello: HelloResult = c
            .call(
                method::HELLO,
                Hello {
                    protocol: PROTOCOL,
                    client: client.to_owned(),
                    version: crate::VERSION.to_owned(),
                },
            )
            .await?;
        c.hello = Some(hello);
        Ok(c)
    }

    /// Connects, starting the daemon with `launcher` if none is running.
    pub async fn connect_or_start(
        state_dir: &Path,
        client: &str,
        launcher: &Launcher,
    ) -> Result<Client, ClientError> {
        match Client::connect(state_dir, client).await {
            Err(ClientError::NotRunning) => {}
            other => return other,
        }
        let mut child = launcher.spawn(state_dir).map_err(|e| {
            ClientError::Start(format!("cannot run {}: {e}", launcher.program.display()))
        })?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match Client::connect(state_dir, client).await {
                Ok(c) => return Ok(c),
                Err(ClientError::NotRunning) => {}
                Err(e) => return Err(e),
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(ClientError::Start(format!(
                    "it exited ({status}). {}",
                    log_hint(state_dir)
                )));
            }
            if Instant::now() > deadline {
                return Err(ClientError::Start(format!(
                    "it did not accept connections within {} seconds. {}",
                    START_TIMEOUT.as_secs(),
                    log_hint(state_dir)
                )));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The daemon's handshake reply.
    pub fn hello(&self) -> &HelloResult {
        self.hello.as_ref().expect("set by connect")
    }

    /// Sends a request and waits for its result. Notifications that arrive
    /// meanwhile are kept for [`Client::next_notification`].
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let params =
            serde_json::to_value(params).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let mut line = serde_json::to_string(&Request::new(id, method, params))
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(ClientError::Io)?;
        let wait = async {
            loop {
                let value = self.read_message().await?;
                if value.get("id").is_some_and(|v| !v.is_null()) || value.get("error").is_some() {
                    let response: Response = serde_json::from_value(value)
                        .map_err(|e| ClientError::Protocol(e.to_string()))?;
                    if response.id.is_some() && response.id != Some(id) {
                        continue;
                    }
                    if let Some(error) = response.error {
                        return Err(ClientError::Rpc(error));
                    }
                    let result = response.result.unwrap_or(Value::Null);
                    return serde_json::from_value(result)
                        .map_err(|e| ClientError::Protocol(e.to_string()));
                }
                if let Ok(n) = serde_json::from_value::<Notification>(value) {
                    self.pending.push_back(n);
                }
            }
        };
        tokio::time::timeout(REQUEST_TIMEOUT, wait)
            .await
            .unwrap_or(Err(ClientError::Timeout(REQUEST_TIMEOUT)))
    }

    /// The next notification, after `subscribe`.
    pub async fn next_notification(&mut self) -> Result<Notification, ClientError> {
        if let Some(n) = self.pending.pop_front() {
            return Ok(n);
        }
        loop {
            let value = self.read_message().await?;
            if value.get("id").is_none()
                && let Ok(n) = serde_json::from_value::<Notification>(value)
            {
                return Ok(n);
            }
        }
    }

    async fn read_message(&mut self) -> Result<Value, ClientError> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .await
            .map_err(ClientError::Io)?;
        if read == 0 {
            return Err(ClientError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the daemon closed the connection",
            )));
        }
        serde_json::from_str(&line)
            .map_err(|e| ClientError::Protocol(format!("{e}: {}", line.trim())))
    }
}

fn log_hint(state_dir: &Path) -> String {
    let log = state_dir.join("daemon.log");
    let last = tail(&log, 5).unwrap_or_default();
    if last.is_empty() {
        format!("See {}", log.display())
    } else {
        format!("Last lines of {}:\n{}", log.display(), last.join("\n"))
    }
}

/// How to start the daemon.
#[derive(Clone, Debug)]
pub struct Launcher {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl Launcher {
    /// `<this executable> daemon run --config <config>`.
    pub fn current_exe(config: &Path) -> io::Result<Launcher> {
        Ok(Launcher {
            program: std::env::current_exe()?,
            args: vec![
                "daemon".into(),
                "run".into(),
                "--config".into(),
                config.as_os_str().to_owned(),
            ],
        })
    }

    /// Starts the daemon in the background, detached from the terminal, with
    /// its output in `<state>/daemon.log`.
    pub fn spawn(&self, state_dir: &Path) -> io::Result<std::process::Child> {
        private_dir(state_dir)?;
        let log = open_append(&state_dir.join("daemon.log"))?;
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args)
            .env(crate::paths::STATE_DIR_ENV, state_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own process group, so Ctrl-C in the terminal does not reach
            // it. The daemon also ignores SIGHUP from a closed terminal.
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        cmd.spawn()
    }
}
