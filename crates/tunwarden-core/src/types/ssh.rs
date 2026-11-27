//! `ssh-local`, `ssh-remote` and `ssh-dynamic` (PRD 7.10, FR-42).

use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use super::fields::{Fields, forbid_forward_fields, require_remote_port};
use super::{OutputPattern, Program, Readiness, Tool, TypeDef, listen_addrs, pattern};
use crate::config::Forward;
use crate::failure::FailureReason;

/// Options added to every SSH tunnel (FR-42, design section 4 and 15).
const DEFAULT_OPTIONS: [&str; 6] = [
    "BatchMode=yes",
    "ControlMaster=no",
    "ControlPath=none",
    "ExitOnForwardFailure=yes",
    "ServerAliveInterval=15",
    "ServerAliveCountMax=3",
];

/// How long an `ssh-remote` tunnel must stay up to count as ready (FR-44).
pub const REMOTE_READY_AFTER: Duration = Duration::from_secs(5);

/// The connection fields shared by all SSH types.
#[derive(Clone, Debug, PartialEq)]
pub struct SshTarget {
    /// Host or alias from `~/.ssh/config`, optionally `user@host`.
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Jump hosts, in order (`ssh -J`).
    pub jump: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SshLocal {
    pub target: SshTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SshRemote {
    pub target: SshTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SshDynamic {
    pub target: SshTarget,
}

fn parse_target(f: &mut Fields<'_>, kind: &str) -> Option<SshTarget> {
    let host = f.required_argument("host", kind, "\"bastion\"");
    let user = f.argument("user");
    let port = f.port("port");
    let jump = f.argument_list("jump");
    Some(SshTarget {
        host: host?,
        user,
        port,
        jump,
    })
}

impl SshLocal {
    pub(super) fn parse(f: &mut Fields<'_>, forwards: &[Forward]) -> Option<Self> {
        let kind = "ssh-local";
        let target = parse_target(f, kind);
        forbid_forward_fields(f, kind, forwards, &["remote_host", "remote_port"]);
        require_remote_port(f, kind, forwards);
        Some(SshLocal { target: target? })
    }
}

impl SshRemote {
    pub(super) fn parse(f: &mut Fields<'_>, forwards: &[Forward]) -> Option<Self> {
        let kind = "ssh-remote";
        let target = parse_target(f, kind);
        forbid_forward_fields(f, kind, forwards, &["remote_port", "local_host"]);
        require_remote_port(f, kind, forwards);
        Some(SshRemote { target: target? })
    }
}

impl SshDynamic {
    pub(super) fn parse(f: &mut Fields<'_>, forwards: &[Forward]) -> Option<Self> {
        let kind = "ssh-dynamic";
        let target = parse_target(f, kind);
        forbid_forward_fields(f, kind, forwards, &[]);
        Some(SshDynamic { target: target? })
    }
}

/// `[::1]` for IPv6, unchanged otherwise. ssh needs brackets around IPv6 in forward specs.
fn bracket(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn bind(ip: IpAddr) -> String {
    bracket(&ip.to_string())
}

/// The full argument list: defaults, the forward flags, the target, then the host.
fn ssh_args(target: &SshTarget, forward_flags: Vec<String>) -> Vec<String> {
    let mut args = vec!["-N".to_owned(), "-T".to_owned()];
    for option in DEFAULT_OPTIONS {
        args.push("-o".to_owned());
        args.push(option.to_owned());
    }
    if let Some(port) = target.port {
        args.push("-p".to_owned());
        args.push(port.to_string());
    }
    if let Some(user) = &target.user {
        args.push("-l".to_owned());
        args.push(user.clone());
    }
    if !target.jump.is_empty() {
        args.push("-J".to_owned());
        args.push(target.jump.join(","));
    }
    args.extend(forward_flags);
    args.push(target.host.clone());
    args
}

fn hint_vars(target: &SshTarget) -> Vec<(&'static str, String)> {
    vec![("host", target.host.clone())]
}

impl TypeDef for SshLocal {
    fn program(&self) -> Program {
        Program::Tool(Tool::Ssh)
    }

    fn args(&self, forwards: &[Forward]) -> Vec<String> {
        let mut flags = Vec::new();
        for f in forwards {
            let remote_host = f.remote_host.as_deref().unwrap_or("localhost");
            flags.push("-L".to_owned());
            flags.push(format!(
                "{}:{}:{}:{}",
                bind(f.bind),
                f.local_port,
                bracket(remote_host),
                f.remote_port.unwrap_or_default()
            ));
        }
        ssh_args(&self.target, flags)
    }

    fn readiness(&self, forwards: &[Forward]) -> Readiness {
        Readiness::Listening(listen_addrs(forwards))
    }

    fn output_patterns(&self) -> Vec<OutputPattern> {
        SSH_PATTERNS.clone()
    }

    fn hint_vars(&self) -> Vec<(&'static str, String)> {
        hint_vars(&self.target)
    }
}

impl TypeDef for SshRemote {
    fn program(&self) -> Program {
        Program::Tool(Tool::Ssh)
    }

    fn args(&self, forwards: &[Forward]) -> Vec<String> {
        let mut flags = Vec::new();
        for f in forwards {
            let local_host = f.local_host.as_deref().unwrap_or("localhost");
            flags.push("-R".to_owned());
            flags.push(format!(
                "{}:{}:{}",
                f.remote_port.unwrap_or_default(),
                bracket(local_host),
                f.local_port
            ));
        }
        ssh_args(&self.target, flags)
    }

    fn readiness(&self, _forwards: &[Forward]) -> Readiness {
        Readiness::AliveFor(REMOTE_READY_AFTER)
    }

    fn output_patterns(&self) -> Vec<OutputPattern> {
        SSH_PATTERNS.clone()
    }

    fn hint_vars(&self) -> Vec<(&'static str, String)> {
        hint_vars(&self.target)
    }
}

impl TypeDef for SshDynamic {
    fn program(&self) -> Program {
        Program::Tool(Tool::Ssh)
    }

    fn args(&self, forwards: &[Forward]) -> Vec<String> {
        let mut flags = Vec::new();
        for f in forwards {
            flags.push("-D".to_owned());
            flags.push(format!("{}:{}", bind(f.bind), f.local_port));
        }
        ssh_args(&self.target, flags)
    }

    fn readiness(&self, forwards: &[Forward]) -> Readiness {
        Readiness::Listening(listen_addrs(forwards))
    }

    fn output_patterns(&self) -> Vec<OutputPattern> {
        SSH_PATTERNS.clone()
    }

    fn hint_vars(&self) -> Vec<(&'static str, String)> {
        hint_vars(&self.target)
    }
}

/// Output that means retrying cannot help. Texts are from OpenSSH 10.3 (see
/// `docs/spikes.md`) and the OpenSSH source.
pub(super) static SSH_PATTERNS: LazyLock<Vec<OutputPattern>> = LazyLock::new(|| {
    vec![
        pattern(
            r"Host key verification failed|REMOTE HOST IDENTIFICATION HAS CHANGED|No (ED25519|ECDSA|RSA)? ?host key is known",
            FailureReason::HostKey,
            "the SSH host key for {host} is unknown or has changed; run `tunwarden trust {host}`",
        ),
        pattern(
            r"Permission denied \(|Too many authentication failures|no such identity",
            FailureReason::Credentials,
            "{host} refused your SSH key; check that it is loaded (`ssh-add -l`) and allowed on the server",
        ),
        pattern(
            r"remote port forwarding failed for listen port",
            FailureReason::PortConflict,
            "the remote port is already in use on {host}; choose another remote_port",
        ),
        pattern(
            r"Address already in use|cannot listen to port|Could not request local forwarding",
            FailureReason::PortConflict,
            "a local port of {tunnel} is already in use; stop the other program or change local_port",
        ),
    ]
});
