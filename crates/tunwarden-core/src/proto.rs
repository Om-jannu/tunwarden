//! The daemon control protocol (design 5.3): a subset of JSON-RPC 2.0, one
//! JSON object per line. Shared by the daemon, the CLI and the desktop app.
//! The CLI's `--json` output prints these result types unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::failure::FailureReason;
use crate::logs::ActivityEvent;
use crate::state::Desired;
use crate::supervisor::State;

/// Changes only on incompatible changes; adding optional fields keeps it.
pub const PROTOCOL: u32 = 1;
/// Longest accepted message line.
pub const MAX_LINE: usize = 1024 * 1024;

/// JSON-RPC error codes.
pub const RPC_APP_ERROR: i64 = -32000;
pub const RPC_PROTOCOL_MISMATCH: i64 = -32001;
pub const RPC_PARSE_ERROR: i64 = -32700;
pub const RPC_INVALID_PARAMS: i64 = -32602;
pub const RPC_UNKNOWN_METHOD: i64 = -32601;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

/// An error with a stable string code and a next step for the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    /// For example `unknown_tunnel` or `protocol_mismatch`.
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl RpcError {
    pub fn app(code: &str, message: impl Into<String>, hint: Option<String>) -> RpcError {
        RpcError {
            code: RPC_APP_ERROR,
            message: message.into(),
            data: Some(ErrorData {
                code: code.to_owned(),
                hint,
            }),
        }
    }

    /// The stable string code, if any.
    pub fn kind(&self) -> Option<&str> {
        self.data.as_ref().map(|d| d.code.as_str())
    }

    pub fn hint(&self) -> Option<&str> {
        self.data.as_ref().and_then(|d| d.hint.as_deref())
    }
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<u64>, error: RpcError) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Notification {
        Notification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// --- Method parameters and results -----------------------------------------

pub mod method {
    pub const HELLO: &str = "hello";
    pub const STATUS: &str = "status";
    pub const LIST: &str = "list";
    pub const START: &str = "start";
    pub const STOP: &str = "stop";
    pub const RESTART: &str = "restart";
    pub const LOGS_TAIL: &str = "logs.tail";
    pub const ACTIVITY: &str = "activity";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const SHUTDOWN: &str = "daemon.shutdown";
    pub const AUTH_STATUS: &str = "auth.status";
    pub const AUTH_CHECK: &str = "auth.check";
    pub const AUTH_RECOVERED: &str = "auth.recovered";
    pub const CONFIG_ADD: &str = "config.add";
    pub const CONFIG_EDIT: &str = "config.edit";
    pub const CONFIG_REMOVE: &str = "config.remove";
    pub const CONFIG_ADD_ENTRY: &str = "config.add_entry";
    pub const DOCTOR: &str = "doctor";
}

/// Notification methods.
pub mod notice {
    pub const TUNNEL_STATE: &str = "tunnel.state";
    pub const ACTIVITY: &str = "activity";
    pub const LOG: &str = "log";
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub client: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol: u32,
    pub daemon_version: String,
    pub pid: u32,
    pub config_path: String,
    pub state_dir: String,
}

/// Which tunnels a start, stop or restart applies to (FR-9).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selector {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub all: bool,
    /// Only resolve the selection, for example to ask before starting
    /// production tunnels (FR-33); nothing is started or stopped.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    /// The tunnels the request applied to.
    pub tunnels: Vec<String>,
}

/// One tunnel as `status` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelView {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub profile: Option<String>,
    pub production: bool,
    pub groups: Vec<String>,
    /// Local addresses, for example `127.0.0.1:5433`.
    pub local: Vec<String>,
    pub state: State,
    pub reason: Option<FailureReason>,
    pub last_error: Option<String>,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub retries: u32,
    /// When the current state began.
    pub since: String,
    pub desired: Desired,
    pub pending_restart: bool,
    /// An explanation from the daemon, for example after restoring from a crash.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusParams {
    #[serde(default)]
    pub names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResult {
    pub tunnels: Vec<TunnelView>,
    /// Problems in the config file; the daemon keeps the last valid config (FR-5).
    pub config_errors: Vec<String>,
    /// Daemon-level notes, for example a damaged state file.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub profile: Option<String>,
    pub production: bool,
    pub groups: Vec<String>,
    pub local: Vec<String>,
    /// The command as a shell line, for showing and copying (FR-40).
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListResult {
    pub tunnels: Vec<TunnelInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogsParams {
    pub name: String,
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogsResult {
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityParams {
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityResult {
    pub events: Vec<ActivityEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeParams {
    /// `tunnels`, `activity`, or `log:<tunnel>`.
    pub topics: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownParams {
    /// Restore the running tunnels on the next start (FR-15).
    #[serde(default)]
    pub keep: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthParams {
    /// A login name, or a profile name meaning that profile's login. All
    /// logins if empty.
    #[serde(default)]
    pub name: Option<String>,
}

/// One login and the profiles that use it (FR-22, FR-23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginView {
    pub name: String,
    pub profiles: Vec<String>,
    /// Whether the login has a `check` command.
    pub checkable: bool,
    /// `ok`, `expired`, `unknown`, or `not checked`.
    pub result: String,
    pub detail: Option<String>,
    pub checked_at: Option<String>,
    /// Tunnels using this login that are active now.
    pub active_tunnels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResult {
    pub logins: Vec<LoginView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredParams {
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredResult {
    /// Tunnels that failed because of the expired login and are starting again.
    pub restarted: Vec<String>,
}

/// Adds a `[[logins]]` or `[[profiles]]` entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigAddEntryParams {
    /// `logins` or `profiles`.
    pub table: String,
    pub name: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
}

/// Adds a tunnel. `fields` are the tunnel's config fields except `name`,
/// for example `{"type": "ssh-local", "host": "bastion", "forwards": [...]}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigAddParams {
    pub name: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
}

/// Changes a tunnel: `set` replaces fields, `unset` removes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigEditParams {
    pub name: String,
    #[serde(default)]
    pub set: serde_json::Map<String, Value>,
    #[serde(default)]
    pub unset: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRemoveParams {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigEditResult {
    pub tunnel: String,
    /// `added`, `updated` or `removed`.
    pub change: String,
    pub config_path: String,
    /// The tunnel is running with its old settings until it is restarted.
    pub restart_to_apply: bool,
}

/// Where a program was found and its version (FR-36, FR-46).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCheck {
    pub program: String,
    pub path: Option<String>,
    /// For example `found on PATH` or `set in [tools]`.
    pub source: Option<String>,
    pub version: Option<String>,
    /// Why it was not found.
    pub error: Option<String>,
}

/// The daemon's side of `doctor`: what it sees can differ from the shell,
/// for example the PATH of a daemon started by the app on macOS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorResult {
    pub tools: Vec<ToolCheck>,
    /// The last error writing logs or state, if any.
    pub write_error: Option<String>,
}

/// The `log` notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogNotice {
    pub tunnel: String,
    pub line: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_format() {
        let req = Request::new(
            1,
            method::HELLO,
            json!({"protocol": 1, "client": "cli", "version": "0.1.0"}),
        );
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"client":"cli","protocol":1,"version":"0.1.0"}}"#
        );
        let err = Response::err(
            Some(7),
            RpcError::app(
                "port_conflict",
                "Port 5433 is in use by postgres (PID 812)",
                Some("Stop that process".into()),
            ),
        );
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32000,"message":"Port 5433 is in use by postgres (PID 812)","data":{"code":"port_conflict","hint":"Stop that process"}}}"#
        );
        let ok: Response =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":2,"result":{"tunnels":["a"]}}"#).unwrap();
        let result: ActionResult = serde_json::from_value(ok.result.unwrap()).unwrap();
        assert_eq!(result.tunnels, ["a"]);
    }

    #[test]
    fn selector_defaults() {
        let s: Selector = serde_json::from_str(r#"{"group":"daily"}"#).unwrap();
        assert_eq!(
            s,
            Selector {
                group: Some("daily".into()),
                ..Selector::default()
            }
        );
    }
}
