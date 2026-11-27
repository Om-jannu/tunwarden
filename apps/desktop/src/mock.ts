// A stand-in daemon for design previews in a browser and for tests. It holds
// a few tunnels and answers like the bridge does.
import type { Api, ConfigEntries, DaemonEvent, LoginLine } from "./api";
import type { Entry } from "./forms";
import type { ActivityEvent, StatusResult, TunnelView } from "./protocol";

function view(p: Partial<TunnelView> & { name: string }): TunnelView {
  return {
    type: "command",
    profile: null,
    production: false,
    groups: [],
    local: [],
    state: "stopped",
    reason: null,
    last_error: null,
    pid: null,
    restarts: 0,
    retries: 0,
    since: new Date(Date.now() - 2 * 3600_000 - 13 * 60_000).toISOString(),
    desired: "stopped",
    pending_restart: false,
    note: null,
    ...p,
  };
}

export function sampleTunnels(): TunnelView[] {
  return [
    view({ name: "staging-db", profile: "staging", groups: ["daily"], local: ["127.0.0.1:5433"], state: "running", pid: 48113, desired: "running" }),
    view({ name: "staging-redis", profile: "staging", groups: ["daily"], local: ["127.0.0.1:6380"], state: "reconnecting", restarts: 2, last_error: "the command exited with code 255", desired: "running" }),
    view({ name: "prod-api", profile: "prod", production: true, local: ["127.0.0.1:8080"], state: "failed", reason: "auth expired", last_error: "the google login has expired; run `tunwarden login google`", desired: "running" }),
    view({ name: "prod-db", profile: "prod", production: true, local: ["127.0.0.1:5434"], state: "stopped" }),
    view({ name: "bastion-socks", type: "ssh-dynamic", local: ["127.0.0.1:1080"], state: "running", pid: 48201, desired: "running", pending_restart: true }),
    view({ name: "grafana", local: ["127.0.0.1:3000"], state: "stopped" }),
  ];
}

const LOG = [
  "tw stopped -> starting",
  "tw started PID 48113: gcloud compute start-iap-tunnel db-1 5432 --zone=europe-west1-b --local-host-port=127.0.0.1:5433",
  "err Testing if tunnel connection works.",
  "err Listening on port [5433].",
  "tw starting -> running",
  "tw ready",
];

export function sampleEntries(): ConfigEntries {
  return {
    tunnels: [
      { name: "staging-db", profile: "staging", groups: ["daily"], command: "gcloud compute start-iap-tunnel db-1 5432 --local-host-port=127.0.0.1:5433", forwards: [{ local_port: 5433 }] },
      { name: "staging-redis", type: "ssh-local", host: "bastion", profile: "staging", groups: ["daily"], max_retries: 20, forwards: [{ local_port: 6380, remote_host: "redis.internal", remote_port: 6379 }] },
      { name: "prod-api", profile: "prod", command: "gcloud compute ssh app-1 --tunnel-through-iap -- -N -L 8080:localhost:80", forwards: [{ local_port: 8080 }] },
      { name: "prod-db", profile: "prod", command: "gcloud compute start-iap-tunnel db-2 5432 --local-host-port=127.0.0.1:5434", forwards: [{ local_port: 5434 }] },
      { name: "bastion-socks", type: "ssh-dynamic", host: "bastion", forwards: [{ local_port: 1080 }] },
      { name: "grafana", command: "kubectl port-forward svc/grafana 3000:80", forwards: [{ local_port: 3000 }] },
    ],
    profiles: [
      { name: "staging", login: "google", env: { CLOUDSDK_CORE_PROJECT: "example-staging-123" } },
      { name: "prod", login: "google", production: true, env: { CLOUDSDK_CORE_PROJECT: "example-prod-456" } },
    ],
    logins: [
      { name: "google", check: "gcloud config config-helper --force-auth-refresh --format=none", login: "gcloud auth login me@example.com", expired: ["Reauthentication failed"], env: { CLOUDSDK_CORE_ACCOUNT: "me@example.com" } },
    ],
  };
}

export function mockApi(options: { configExists?: boolean } = {}): Api {
  let tunnels = sampleTunnels();
  const entries = sampleEntries();
  const loginListeners = new Set<(l: LoginLine) => void>();
  let autostart = false;
  const activity: ActivityEvent[] = [
    { time: new Date(Date.now() - 600_000).toISOString(), tunnel: "staging-db", from: "starting", to: "running" },
    { time: new Date(Date.now() - 300_000).toISOString(), tunnel: "prod-api", from: "running", to: "failed", reason: "auth expired", message: "the google login has expired; run `tunwarden login google`" },
    { time: new Date(Date.now() - 60_000).toISOString(), tunnel: "staging-redis", from: "running", to: "reconnecting", message: "the command exited with code 255" },
  ];
  let configExists = options.configExists ?? !new URLSearchParams(globalThis.location?.search ?? "").has("fresh");
  const listeners = new Set<(e: DaemonEvent) => void>();
  const emit = (e: DaemonEvent) => listeners.forEach((f) => f(e));
  const status = (): StatusResult => ({ tunnels: structuredClone(tunnels), config_errors: [], notes: [] });
  const set = (name: string, p: Partial<TunnelView>) => {
    tunnels = tunnels.map((t) => (t.name === name ? { ...t, ...p, since: new Date().toISOString() } : t));
    const v = tunnels.find((t) => t.name === name);
    if (v) emit({ kind: "tunnel", view: structuredClone(v) });
  };
  return {
    configInfo: async () => ({ path: "~/.config/tunwarden/config.toml", exists: configExists, errors: [], tunnels: configExists ? tunnels.length : 0 }),
    initExample: async () => {
      configExists = true;
      return "~/.config/tunwarden/config.toml";
    },
    openConfig: async () => {},
    importPreview: async (text) => ({
      tunnels: text.trim()
        ? [
            { name: "uatdb", kind: "command", profile: "imported", summary: "gcloud compute start-iap-tunnel uat-vm 3371 --local-host-port=localhost:3371", local: [3371] },
            { name: "redis", kind: "ssh-local", profile: null, summary: "bastion", local: [6380] },
          ]
        : [],
      profiles: ["imported"],
      warnings: text.includes("-f") ? ["line 2: uses `ssh -f`; dropped, because Tunwarden runs tunnels in the background itself"] : [],
    }),
    importApply: async (_text, names) => {
      configExists = true;
      return names;
    },
    status: async () => status(),
    act: async (action, sel) => {
      const names = sel.all ? tunnels.map((t) => t.name) : sel.names;
      for (const n of names) {
        if (action === "stop") set(n, { state: "stopped", pid: null, reason: null, last_error: null, desired: "stopped" });
        else {
          set(n, { state: "starting", reason: null, last_error: null, desired: "running" });
          setTimeout(() => set(n, { state: "running", pid: 40000 + Math.floor(Math.random() * 9000) }), 900);
        }
      }
      return { tunnels: names };
    },
    logsTail: async (_name, lines) => {
      const base = Date.now() - LOG.length * 1000;
      return { lines: LOG.slice(-lines).map((l, i) => `${new Date(base + i * 1000).toISOString()} ${l}`) };
    },
    authStatus: async () => ({
      logins: [{ name: "google", profiles: ["staging", "prod"], checkable: true, result: "expired", detail: "Reauthentication failed", checked_at: new Date().toISOString(), active_tunnels: [] }],
    }),
    authCheck: async () => ({
      logins: [{ name: "google", profiles: ["staging", "prod"], checkable: true, result: "expired", detail: "Reauthentication failed", checked_at: new Date().toISOString(), active_tunnels: ["staging-db"] }],
    }),
    configEntries: async () => structuredClone(entries),
    saveEntry: async (table, name, old, fields) => {
      if (table === "tunnels" && fields.type !== undefined && !String(fields.host ?? "")) {
        throw { message: `the change would make the config invalid, so nothing was written:\nconfig.toml:9:1: tunnels[5].host: a ${String(fields.type)} tunnel needs \`host\``, hint: null, kind: "other" };
      }
      const list = entries[table] as Entry[];
      const entry = { name, ...fields };
      if (old) list.splice(list.findIndex((e) => e.name === name), 1, entry);
      else list.push(entry);
      return { tunnel: name, change: old ? "updated" : "added", config_path: "~/.config/tunwarden/config.toml", restart_to_apply: false };
    },
    removeEntry: async (table, name) => {
      const list = entries[table] as Entry[];
      list.splice(list.findIndex((e) => e.name === name), 1);
      return { tunnel: name, change: "removed", config_path: "~/.config/tunwarden/config.toml", restart_to_apply: false };
    },
    doctor: async () => ({
      rows: [
        { section: "Config", mark: "ok", item: "~/.config/tunwarden/config.toml", detail: "6 tunnels, 2 profiles, 1 logins" },
        { section: "Daemon", mark: "ok", item: "running", detail: "PID 4120, version 0.1.0" },
        { section: "Daemon", mark: autostart ? "ok" : "ok", item: autostart ? "autostart on" : "autostart off", detail: autostart ? "~/Library/LaunchAgents/io.github.om-jannu.tunwarden.plist" : "run `tunwarden autostart enable` to start the daemon at login" },
        { section: "Tools", mark: "ok", item: "gcloud", detail: "/opt/homebrew/bin/gcloud (PATH, Google Cloud SDK 591.0.0)" },
        { section: "Tools", mark: "problem", item: "kubectl", detail: "kubectl was not found; install it, use its full path, or add its folder to tools.path" },
        { section: "Logins", mark: "problem", item: "google (staging, prod)", detail: "expired; run `tunwarden login google`" },
        { section: "Ports", mark: "ok", item: "local ports", detail: "free for every tunnel that is not running" },
      ],
    }),
    activity: async (limit) => ({ events: activity.slice(-limit) }),
    autostartStatus: async () => ({ enabled: autostart, location: "~/Library/LaunchAgents/io.github.om-jannu.tunwarden.plist", problem: null, other_binary: null }),
    autostartSet: async (enabled) => {
      autostart = enabled;
      return { enabled, location: "~/Library/LaunchAgents/io.github.om-jannu.tunwarden.plist", problem: null, other_binary: null };
    },
    loginRun: async (name) => {
      for (const line of ["Your browser has been opened to visit:", "    https://accounts.google.com/o/oauth2/auth?...", "You are now logged in as [me@example.com]."]) {
        await new Promise((r) => setTimeout(r, 300));
        loginListeners.forEach((f) => f({ login: name, line }));
      }
      return { login: name, restarted: ["prod-api"] };
    },
    loginInTerminal: async () => {},
    onLoginOutput: async (f) => {
      loginListeners.add(f);
      return () => loginListeners.delete(f);
    },
    restartDaemon: async () => {},
    quit: async () => {},
    copy: async (text) => {
      await navigator.clipboard?.writeText(text).catch(() => {});
    },
    notify: async (title, body) => console.info("notification:", title, body),
    onDaemon: async (f) => {
      listeners.add(f);
      setTimeout(() => f({ kind: "connected", status: status(), version: "0.1.0" }), 50);
      return () => listeners.delete(f);
    },
    onQuitRequested: async () => () => {},
    onConfirmStart: async () => () => {},
  };
}
