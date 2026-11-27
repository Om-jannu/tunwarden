//! The tunnel types a config can name (PRD 7.10).

use std::fmt;

/// A tunnel type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TunnelKind {
    SshLocal,
    SshRemote,
    SshDynamic,
    /// Any command that opens a local port.
    Command,
}

impl TunnelKind {
    pub const ALL: [TunnelKind; 4] = [
        TunnelKind::Command,
        TunnelKind::SshLocal,
        TunnelKind::SshRemote,
        TunnelKind::SshDynamic,
    ];

    /// The name used in the config file.
    pub const fn id(self) -> &'static str {
        match self {
            TunnelKind::SshLocal => "ssh-local",
            TunnelKind::SshRemote => "ssh-remote",
            TunnelKind::SshDynamic => "ssh-dynamic",
            TunnelKind::Command => "command",
        }
    }

    pub fn from_id(id: &str) -> Option<TunnelKind> {
        TunnelKind::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Whether the forwards' local ports are ports this tunnel listens on.
    /// For `ssh-remote` the local port is the target of the remote forward.
    pub const fn listens_locally(self) -> bool {
        !matches!(self, TunnelKind::SshRemote)
    }

    /// Whether a tunnel of this type needs at least one forward. `command`
    /// may have none and use the "process alive" readiness check (FR-44).
    pub const fn requires_forward(self) -> bool {
        !matches!(self, TunnelKind::Command)
    }
}

impl fmt::Display for TunnelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for kind in TunnelKind::ALL {
            assert_eq!(TunnelKind::from_id(kind.id()), Some(kind));
        }
        assert_eq!(TunnelKind::from_id("gcloud-iap"), None);
    }
}
