// The bridge to the daemon (src-tauri/src/bridge.rs). Inside Tauri it calls
// the Rust commands; in a plain browser (design previews, tests) it uses a
// mock with sample tunnels.
import type {
  ActionResult,
  ActivityEvent,
  ActivityResult,
  AuthResult,
  ConfigEditResult,
  Report,
  LogsResult,
  Selector,
  StatusResult,
  TunnelView,
} from "./protocol";
import { mockApi } from "./mock";
import type { Entry, TableName } from "./forms";

export type Action = "start" | "stop" | "restart";

export type Failure = {
  message: string;
  hint: string | null;
  kind: "not_running" | "protocol_mismatch" | "start_failed" | "other";
};

export type ConfigInfo = {
  path: string;
  exists: boolean;
  errors: string[];
  tunnels: number;
};

export type ImportEntry = {
  name: string;
  kind: string;
  profile: string | null;
  summary: string;
  local: number[];
};

export type ImportPreview = {
  tunnels: ImportEntry[];
  profiles: string[];
  warnings: string[];
};

export type DaemonEvent =
  | { kind: "connected"; status: StatusResult; version: string }
  | { kind: "disconnected"; failure: Failure }
  | { kind: "tunnel"; view: TunnelView }
  | { kind: "activity"; event: ActivityEvent };

export type ConfirmStart = { selector: Selector; production: string[] };

export type ConfigEntries = { tunnels: Entry[]; profiles: Entry[]; logins: Entry[] };
export type AutostartInfo = { enabled: boolean; location: string; problem: string | null; other_binary: string | null };
export type LoginDone = { login: string; restarted: string[] };
export type LoginLine = { login: string; line: string };

export interface Api {
  configInfo(): Promise<ConfigInfo>;
  initExample(): Promise<string>;
  openConfig(): Promise<void>;
  importPreview(text: string): Promise<ImportPreview>;
  importApply(text: string, names: string[]): Promise<string[]>;
  status(): Promise<StatusResult>;
  act(action: Action, selector: Selector): Promise<ActionResult>;
  logsTail(name: string, lines: number): Promise<LogsResult>;
  authStatus(): Promise<AuthResult>;
  authCheck(): Promise<AuthResult>;
  configEntries(): Promise<ConfigEntries>;
  saveEntry(table: TableName, name: string, old: Entry | null, fields: Entry): Promise<ConfigEditResult>;
  removeEntry(table: TableName, name: string): Promise<ConfigEditResult>;
  doctor(): Promise<Report>;
  activity(limit: number): Promise<ActivityResult>;
  autostartStatus(): Promise<AutostartInfo>;
  autostartSet(enabled: boolean): Promise<AutostartInfo>;
  loginRun(name: string): Promise<LoginDone>;
  loginInTerminal(name: string): Promise<void>;
  onLoginOutput(f: (l: LoginLine) => void): Promise<() => void>;
  restartDaemon(): Promise<void>;
  quit(stopTunnels: boolean): Promise<void>;
  copy(text: string): Promise<void>;
  notify(title: string, body: string): Promise<void>;
  onDaemon(f: (e: DaemonEvent) => void): Promise<() => void>;
  onQuitRequested(f: () => void): Promise<() => void>;
  onConfirmStart(f: (c: ConfirmStart) => void): Promise<() => void>;
}

export function selector(part: Partial<Selector>): Selector {
  return { names: [], group: null, profile: null, all: false, dry_run: false, ...part };
}

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriApi(): Promise<Api> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const notification = await import("@tauri-apps/plugin-notification");
  let allowed: boolean | null = null;
  return {
    configInfo: () => invoke("config_info"),
    initExample: () => invoke("init_example"),
    openConfig: () => invoke("open_config"),
    importPreview: (text) => invoke("import_preview", { text }),
    importApply: (text, names) => invoke("import_apply", { text, names }),
    status: () => invoke("status"),
    act: (action, sel) => invoke("act", { action, selector: sel }),
    logsTail: (name, lines) => invoke("logs_tail", { name, lines }),
    authStatus: () => invoke("auth_status"),
    authCheck: () => invoke("auth_check"),
    configEntries: () => invoke("config_entries"),
    saveEntry: (table, name, old, fields) => invoke("save_entry", { table, name, old, fields }),
    removeEntry: (table, name) => invoke("remove_entry", { table, name }),
    doctor: () => invoke("doctor"),
    activity: (limit) => invoke("activity", { limit }),
    autostartStatus: () => invoke("autostart_status"),
    autostartSet: (enabled) => invoke("autostart_set", { enabled }),
    loginRun: (name) => invoke("login_run", { name }),
    loginInTerminal: (name) => invoke("login_in_terminal", { name }),
    onLoginOutput: (f) => listen<LoginLine>("login-output", (e) => f(e.payload)),
    restartDaemon: () => invoke("restart_daemon"),
    quit: (stopTunnels) => invoke("quit", { stopTunnels }),
    copy: (text) => navigator.clipboard.writeText(text),
    notify: async (title, body) => {
      if (allowed === null) {
        allowed = await notification.isPermissionGranted();
        if (!allowed) allowed = (await notification.requestPermission()) === "granted";
      }
      if (allowed) notification.sendNotification({ title, body });
    },
    onDaemon: (f) => listen<DaemonEvent>("daemon", (e) => f(e.payload)),
    onQuitRequested: (f) => listen("quit-requested", () => f()),
    onConfirmStart: (f) => listen<ConfirmStart>("confirm-start", (e) => f(e.payload)),
  };
}

let current: Promise<Api> | null = null;

/** The API for this environment. Tests replace it with `setApi`. */
export function api(): Promise<Api> {
  current ??= inTauri ? tauriApi() : Promise.resolve(mockApi());
  return current;
}

export function setApi(a: Api) {
  current = Promise.resolve(a);
}

/** Errors from Tauri commands arrive as the bridge's `Failure` object. */
export function asFailure(e: unknown): Failure {
  if (e && typeof e === "object" && "message" in e) return e as Failure;
  return { message: String(e), hint: null, kind: "other" };
}
