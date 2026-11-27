// App state, fed by daemon events. Screens read from here and call `act`.
import { create } from "zustand";
import { api, asFailure, selector, type Action, type ConfigInfo, type Failure } from "./api";
import { Notifier } from "./logic";
import type { TableName } from "./forms";
import type { ActivityEvent, LoginView, Selector, TunnelView } from "./protocol";

export type Pending = { action: Action; selector: Selector; production: string[] };
export type View = "tunnels" | "logins" | "activity" | "doctor" | "settings";
export type Editing = { table: TableName; name: string | null };

type Store = {
  phase: "loading" | "first-run" | "ready";
  config: ConfigInfo | null;
  tunnels: TunnelView[] | null;
  configErrors: string[];
  version: string | null;
  failure: Failure | null;
  expired: LoginView[];
  logins: LoginView[];
  activity: ActivityEvent[];
  view: View;
  editing: Editing | null;
  /** The login whose `login` command runs in the app now. */
  loggingIn: string | null;
  open: string | null;
  confirm: Pending | null;
  quitOpen: boolean;
  busy: Record<string, boolean>;
  start(): Promise<void>;
  refreshConfig(): Promise<void>;
  setOpen(name: string | null): void;
  setQuitOpen(open: boolean): void;
  /** Runs an action, asking first when it would start production tunnels. */
  act(action: Action, sel: Selector, confirmed?: boolean): Promise<void>;
  cancelConfirm(): void;
  restartDaemon(): Promise<void>;
  setView(v: View): void;
  edit(e: Editing | null): void;
  loadLogins(): Promise<void>;
};

let notifier: Notifier | null = null;

export const useStore = create<Store>((set, get) => ({
  phase: "loading",
  config: null,
  tunnels: null,
  configErrors: [],
  version: null,
  failure: null,
  expired: [],
  logins: [],
  activity: [],
  view: "tunnels",
  editing: null,
  loggingIn: null,
  open: null,
  confirm: null,
  quitOpen: false,
  busy: {},

  async start() {
    const a = await api();
    notifier ??= new Notifier((n) => void a.notify(n.title, n.body));
    await get().refreshConfig();
    await a.onDaemon((e) => {
      switch (e.kind) {
        case "connected":
          set({ tunnels: e.status.tunnels, configErrors: e.status.config_errors, version: e.version, failure: null });
          void loadLogins();
          break;
        case "disconnected":
          set({ tunnels: null, failure: e.failure });
          break;
        case "tunnel": {
          const before = get().tunnels?.find((t) => t.name === e.view.name)?.state;
          notifier?.observe(before, e.view);
          set((s) => ({
            tunnels: s.tunnels
              ? s.tunnels.some((t) => t.name === e.view.name)
                ? s.tunnels.map((t) => (t.name === e.view.name ? e.view : t))
                : [...s.tunnels, e.view]
              : s.tunnels,
          }));
          if (e.view.reason === "auth expired" || before === "failed") void loadLogins();
          break;
        }
        case "activity":
          set((s) => ({ activity: [...s.activity, e.event].slice(-500) }));
          break;
      }
    });
    // The watcher sends `connected` once, as soon as it reaches the daemon.
    // With a daemon already running that happens before this listener exists
    // and the event is dropped, so ask for the status once ourselves. If the
    // daemon is not reachable, the watcher's next `disconnected` says why.
    if (get().tunnels === null) {
      try {
        const s = await a.status();
        if (get().tunnels === null) set({ tunnels: s.tunnels, configErrors: s.config_errors, failure: null });
      } catch {
        // Left to the watcher.
      }
    }
    await a.onQuitRequested(() => set({ quitOpen: true }));
    await a.onConfirmStart((c) => set({ confirm: { action: "start", selector: c.selector, production: c.production } }));
  },

  async refreshConfig() {
    const a = await api();
    const config = await a.configInfo();
    set({ config, phase: config.exists ? "ready" : "first-run" });
  },

  setOpen: (open) => set({ open }),
  setView: (view) => set({ view }),
  edit: (editing) => set({ editing }),
  loadLogins: () => loadLogins(),
  setQuitOpen: (quitOpen) => set({ quitOpen }),
  cancelConfirm: () => set({ confirm: null }),

  async act(action, sel, confirmed = false) {
    const tunnels = get().tunnels ?? [];
    const chosen = sel.all
      ? tunnels
      : tunnels.filter(
          (t) =>
            sel.names.includes(t.name) ||
            (sel.group !== null && t.groups.includes(sel.group)) ||
            (sel.profile !== null && t.profile === sel.profile),
        );
    const production = chosen.filter((t) => t.production).map((t) => t.name);
    if (action !== "stop" && production.length > 0 && !confirmed) {
      set({ confirm: { action, selector: sel, production } });
      return;
    }
    set((s) => ({ confirm: null, busy: { ...s.busy, ...Object.fromEntries(chosen.map((t) => [t.name, true])) } }));
    try {
      await (await api()).act(action, sel);
    } catch (e) {
      set({ failure: asFailure(e) });
    } finally {
      set((s) => ({ busy: Object.fromEntries(Object.entries(s.busy).filter(([n]) => !chosen.some((t) => t.name === n))) }));
    }
  },

  async restartDaemon() {
    try {
      await (await api()).restartDaemon();
      set({ failure: null });
    } catch (e) {
      set({ failure: asFailure(e) });
    }
  },
}));

async function loadLogins() {
  try {
    const r = await (await api()).authStatus();
    useStore.setState({ logins: r.logins, expired: r.logins.filter((l) => l.result === "expired") });
  } catch {
    // The banner is only a shortcut; the tunnel rows already say what failed.
  }
}

export const one = (name: string) => selector({ names: [name] });
