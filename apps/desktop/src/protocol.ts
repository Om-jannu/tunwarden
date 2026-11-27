// Generated from crates/tunwarden-core/src/proto.rs; do not edit.
// Regenerate: cargo test -p tunwarden-core --features ts export_typescript

export type FailureReason = "auth expired" | "credentials" | "host key" | "port conflict" | "missing tool" | "spawn error" | "retries exhausted" | "invalid config";
export type State = "stopped" | "starting" | "running" | "unhealthy" | "reconnecting" | "failed";
export type Desired = "running" | "stopped";
export type HelloResult = { protocol: number, daemon_version: string, pid: number, config_path: string, state_dir: string, };
export type Selector = { names: Array<string>, group: string | null, profile: string | null, all: boolean, 
/**
 * Only resolve the selection, for example to ask before starting
 * production tunnels (FR-33); nothing is started or stopped.
 */
dry_run: boolean, };
export type ActionResult = { 
/**
 * The tunnels the request applied to.
 */
tunnels: Array<string>, };
export type TunnelView = { name: string, type: string, profile: string | null, production: boolean, groups: Array<string>, 
/**
 * Local addresses, for example `127.0.0.1:5433`.
 */
local: Array<string>, state: State, reason: FailureReason | null, last_error: string | null, pid: number | null, restarts: number, retries: number, 
/**
 * When the current state began.
 */
since: string, desired: Desired, pending_restart: boolean, 
/**
 * An explanation from the daemon, for example after restoring from a crash.
 */
note: string | null, };
export type StatusResult = { tunnels: Array<TunnelView>, 
/**
 * Problems in the config file; the daemon keeps the last valid config (FR-5).
 */
config_errors: Array<string>, 
/**
 * Daemon-level notes, for example a damaged state file.
 */
notes: Array<string>, };
export type TunnelInfo = { name: string, type: string, profile: string | null, production: boolean, groups: Array<string>, local: Array<string>, 
/**
 * The command as a shell line, for showing and copying (FR-40).
 */
command: string, };
export type ListResult = { tunnels: Array<TunnelInfo>, };
export type LogsResult = { lines: Array<string>, };
export type LogNotice = { tunnel: string, line: string, };
export type LoginView = { name: string, profiles: Array<string>, 
/**
 * Whether the login has a `check` command.
 */
checkable: boolean, 
/**
 * `ok`, `expired`, `unknown`, or `not checked`.
 */
result: string, detail: string | null, checked_at: string | null, 
/**
 * Tunnels using this login that are active now.
 */
active_tunnels: Array<string>, };
export type AuthResult = { logins: Array<LoginView>, };
export type RecoveredResult = { 
/**
 * Tunnels that failed because of the expired login and are starting again.
 */
restarted: Array<string>, };
export type ConfigEditResult = { tunnel: string, 
/**
 * `added`, `updated` or `removed`.
 */
change: string, config_path: string, 
/**
 * The tunnel is running with its old settings until it is restarted.
 */
restart_to_apply: boolean, };
export type ActivityEvent = { time: string, tunnel: string, from: State, to: State, reason?: FailureReason, message?: string, };
export type ActivityResult = { events: Array<ActivityEvent>, };
export type Mark = "ok" | "warning" | "problem";
export type Row = { section: string, mark: Mark, item: string, detail: string, };
export type Report = { rows: Array<Row>, };
