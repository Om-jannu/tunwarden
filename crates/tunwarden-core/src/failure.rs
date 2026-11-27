//! Why a tunnel is `failed` (PRD FR-48).

use std::fmt;

/// The fixed list of reasons a tunnel can be `failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureReason {
    /// The tunnel's login has expired; the user has to log in again (FR-24).
    AuthExpired,
    /// A credential problem other than an expired login, for example an SSH
    /// key that is refused or a missing permission (FR-45).
    Credentials,
    /// An unknown or changed SSH host key (FR-42).
    HostKey,
    /// A local (or, for `ssh-remote`, remote) port is already in use (FR-13).
    PortConflict,
    /// A prerequisite tool is not installed (FR-46).
    MissingTool,
    /// The command could not be started.
    SpawnError,
    /// The tunnel kept failing until max retries (FR-18).
    RetriesExhausted,
    /// The tool rejected a setting, for example an unknown kube context.
    InvalidConfig,
}

impl FailureReason {
    /// Whether retrying can help. Credential and config problems need the
    /// user, so retrying would only hammer the remote side (FR-24).
    pub const fn retryable(self) -> bool {
        matches!(self, FailureReason::SpawnError)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            FailureReason::AuthExpired => "auth expired",
            FailureReason::Credentials => "credentials",
            FailureReason::HostKey => "host key",
            FailureReason::PortConflict => "port conflict",
            FailureReason::MissingTool => "missing tool",
            FailureReason::SpawnError => "spawn error",
            FailureReason::RetriesExhausted => "retries exhausted",
            FailureReason::InvalidConfig => "invalid config",
        }
    }
}

impl FailureReason {
    pub const ALL: [FailureReason; 8] = [
        FailureReason::AuthExpired,
        FailureReason::Credentials,
        FailureReason::HostKey,
        FailureReason::PortConflict,
        FailureReason::MissingTool,
        FailureReason::SpawnError,
        FailureReason::RetriesExhausted,
        FailureReason::InvalidConfig,
    ];

    pub fn from_name(name: &str) -> Option<FailureReason> {
        FailureReason::ALL.into_iter().find(|r| r.as_str() == name)
    }
}

/// Stored and sent as the FR-48 names, for example `"port conflict"`.
impl serde::Serialize for FailureReason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for FailureReason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        FailureReason::from_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown failure reason \"{name}\"")))
    }
}

impl fmt::Display for FailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
