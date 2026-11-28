// Pure rules the screens use: grouping, health, notifications (FR-31,
// FR-33, FR-35). Kept free of React so they are easy to test.
import type { ActivityEvent, State, TunnelView } from "./protocol";

export type Health = "idle" | "good" | "busy" | "bad";

const BUSY: State[] = ["starting", "reconnecting", "unhealthy"];

export function health(tunnels: TunnelView[] | null): Health {
  if (!tunnels) return "idle";
  if (tunnels.some((t) => t.state === "failed")) return "bad";
  if (tunnels.some((t) => BUSY.includes(t.state))) return "busy";
  if (tunnels.some((t) => t.state === "running")) return "good";
  return "idle";
}

/** "3 running, 1 failed", or "Nothing running". */
export function summary(tunnels: TunnelView[]): string {
  const n = (states: State[]) => tunnels.filter((t) => states.includes(t.state)).length;
  const parts = [
    [n(["running"]), "running"],
    [n(BUSY), "connecting"],
    [n(["failed"]), "failed"],
  ] as const;
  const text = parts.filter(([c]) => c > 0).map(([c, w]) => `${c} ${w}`);
  return text.length ? text.join(", ") : "Nothing running";
}

export type ProfileGroup = { profile: string | null; production: boolean; tunnels: TunnelView[] };

/** Tunnels grouped by profile, in config order; "No profile" last (FR-33). */
export function byProfile(tunnels: TunnelView[], filter = ""): ProfileGroup[] {
  const q = filter.trim().toLowerCase();
  const shown = q
    ? tunnels.filter((t) =>
        [t.name, t.profile ?? "", ...t.local, ...t.groups].some((s) => s.toLowerCase().includes(q)),
      )
    : tunnels;
  const groups: ProfileGroup[] = [];
  for (const t of shown) {
    let g = groups.find((x) => x.profile === t.profile);
    if (!g) {
      g = { profile: t.profile, production: t.production, tunnels: [] };
      groups.push(g);
    }
    g.tunnels.push(t);
  }
  return groups.sort((a, b) => Number(a.profile === null) - Number(b.profile === null));
}

/** Whether a tunnel is doing something, so the main action is Stop. */
export function isActive(t: TunnelView): boolean {
  return t.state !== "stopped" && t.state !== "failed";
}

/** What the row says after its state: the reason, a pending restart, a note. */
export function detail(t: TunnelView): string {
  const parts: string[] = [];
  if (t.state === "failed" && t.reason) parts.push(t.last_error ? `${t.reason}: ${t.last_error}` : t.reason);
  else if (BUSY.includes(t.state) && t.last_error) parts.push(t.last_error);
  if (t.pending_restart) parts.push("config changed; restart to apply");
  if (t.note) parts.push(t.note);
  return parts.join("; ");
}

/** Uptime like "2h 13m", "45s". */
export function uptime(since: string, now = Date.now()): string {
  const start = Date.parse(since);
  if (Number.isNaN(start)) return "";
  let s = Math.max(0, Math.floor((now - start) / 1000));
  const d = Math.floor(s / 86400);
  s -= d * 86400;
  const h = Math.floor(s / 3600);
  s -= h * 3600;
  const m = Math.floor(s / 60);
  s -= m * 60;
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

// --- Notifications (FR-35) ---------------------------------------------------

export type Notice = { title: string; body: string };

type Event = { tunnel: string; kind: NoticeKind; reason: string };
type NoticeKind = "down" | "reconnecting" | "failed" | "recovered";

/** Which change of state is worth a notification. */
export function noticeKind(before: State | undefined, after: TunnelView): NoticeKind | null {
  if (before === undefined || before === after.state) return null;
  if (after.state === "failed") return "failed";
  if (after.state === "reconnecting" && before === "running") return "down";
  if (after.state === "running" && (before === "reconnecting" || before === "failed")) return "recovered";
  return null;
}

/**
 * Collects changes for a short window, then emits one notice per reason, so a
 * login expiry that fails five tunnels is one notification. The same tunnel
 * and kind is notified at most once per `quiet` milliseconds.
 */
export class Notifier {
  private pending: Event[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  private last = new Map<string, number>();

  constructor(
    private send: (n: Notice) => void,
    private windowMs = 2000,
    private quietMs = 5 * 60 * 1000,
    private now = () => Date.now(),
  ) {}

  observe(before: State | undefined, after: TunnelView) {
    const kind = noticeKind(before, after);
    if (!kind) return;
    const key = `${after.name}:${kind}`;
    const t = this.now();
    if ((this.last.get(key) ?? -Infinity) > t - this.quietMs) return;
    this.last.set(key, t);
    this.pending.push({ tunnel: after.name, kind, reason: after.last_error ?? after.reason ?? "" });
    this.timer ??= setTimeout(() => this.flush(), this.windowMs);
  }

  flush() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const groups = new Map<string, Event[]>();
    for (const e of this.pending) {
      const k = `${e.kind}|${e.reason}`;
      groups.set(k, [...(groups.get(k) ?? []), e]);
    }
    this.pending = [];
    for (const events of groups.values()) this.send(describe(events));
  }
}

function describe(events: Event[]): Notice {
  const first = events[0]!;
  const names = events.map((e) => e.tunnel);
  const who = names.length === 1 ? names[0]! : `${names.length} tunnels`;
  const list = names.length > 1 ? `${names.join(", ")}: ` : "";
  switch (first.kind) {
    case "failed":
      return { title: `${who} failed`, body: `${list}${first.reason || "see the log for details"}` };
    case "down":
      return { title: `${who} dropped`, body: `${list}reconnecting` };
    case "recovered":
      return { title: `${who} running again`, body: names.length > 1 ? names.join(", ") : "" };
    case "reconnecting":
      return { title: `${who} reconnecting`, body: list };
  }
}

/** A stretch of a tunnel's state; "unknown" where the activity log has no record. */
export type Segment = { state: State | "unknown"; start: number; end: number };

export const DAY = 24 * 60 * 60 * 1000;

/**
 * A tunnel's states over the `span` before `now`, from the activity log.
 * `events` are all tunnels' events, oldest first. When the log may have been
 * cut off (`complete` false), the time before its oldest event is unknown.
 */
export function timeline(events: ActivityEvent[], name: string, current: State, now: number, complete: boolean, span = DAY): Segment[] {
  const windowStart = now - span;
  const oldest = events.length ? Date.parse(events[0]!.time) : now;
  const start = complete ? windowStart : Math.max(windowStart, Math.min(oldest, now));
  const mine = events.filter((e) => e.tunnel === name).map((e) => ({ ...e, at: Date.parse(e.time) }));
  const before = mine.filter((e) => e.at <= start).at(-1);
  const after = mine.filter((e) => e.at > start && e.at <= now);
  let state: State = before?.to ?? after[0]?.from ?? current;
  const out: Segment[] = [];
  if (start > windowStart) out.push({ state: "unknown", start: windowStart, end: start });
  let cursor = start;
  for (const e of after) {
    if (e.at > cursor) out.push({ state, start: cursor, end: e.at });
    cursor = Math.max(cursor, e.at);
    state = e.to;
  }
  if (now > cursor) out.push({ state, start: cursor, end: now });
  // Join neighbours in the same state.
  return out.reduce<Segment[]>((acc, seg) => {
    const last = acc.at(-1);
    if (last && last.state === seg.state && last.end === seg.start) last.end = seg.end;
    else acc.push({ ...seg });
    return acc;
  }, []);
}

/** "running 22h 10m, failed 1h 2m, stopped 48m": how long in each state. */
export function timelineSummary(segments: Segment[]): string {
  const total = new Map<string, number>();
  for (const s of segments) {
    const k = s.state === "starting" || s.state === "reconnecting" || s.state === "unhealthy" ? "connecting" : s.state === "unknown" ? "no record" : s.state;
    total.set(k, (total.get(k) ?? 0) + (s.end - s.start));
  }
  return [...total.entries()]
    .filter(([, ms]) => ms >= 60_000)
    .sort((a, b) => b[1] - a[1])
    .map(([k, ms]) => `${k} ${span(ms)}`)
    .join(", ");
}

function span(ms: number): string {
  const m = Math.round(ms / 60_000);
  const h = Math.floor(m / 60);
  return h ? (m % 60 ? `${h}h ${m % 60}m` : `${h}h`) : `${m}m`;
}
