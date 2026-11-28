import { describe, expect, it, vi } from "vitest";
import { Notifier, byProfile, detail, health, summary, uptime } from "./logic";
import { DAY, timeline, timelineSummary } from "./logic";
import { sampleTunnels } from "./mock";
import { parseLine } from "./components/TunnelSheet";
import type { TunnelView } from "./protocol";

const tunnels = sampleTunnels();
const find = (n: string) => tunnels.find((t) => t.name === n)!;

describe("health and summary", () => {
  it("is the worst state that matters", () => {
    expect(health(null)).toBe("idle");
    expect(health([find("grafana")])).toBe("idle");
    expect(health([find("staging-db")])).toBe("good");
    expect(health([find("staging-db"), find("staging-redis")])).toBe("busy");
    expect(health(tunnels)).toBe("bad");
    expect(summary(tunnels)).toBe("2 running, 1 connecting, 1 failed");
    expect(summary([find("grafana")])).toBe("Nothing running");
  });
});

describe("byProfile", () => {
  it("keeps config order and puts tunnels without a profile last", () => {
    const g = byProfile(tunnels);
    expect(g.map((x) => x.profile)).toEqual(["staging", "prod", null]);
    expect(g[1]!.production).toBe(true);
    expect(g[2]!.tunnels.map((t) => t.name)).toEqual(["bastion-socks", "grafana"]);
  });
  it("filters by name, address, group and profile", () => {
    expect(byProfile(tunnels, "5433").flatMap((g) => g.tunnels.map((t) => t.name))).toEqual(["staging-db"]);
    expect(byProfile(tunnels, "daily").flatMap((g) => g.tunnels).length).toBe(2);
    expect(byProfile(tunnels, "PROD").flatMap((g) => g.tunnels).length).toBe(2);
  });
});

describe("detail and uptime", () => {
  it("says why a tunnel failed and what is pending", () => {
    expect(detail(find("prod-api"))).toBe("auth expired: the google login has expired; run `tunwarden login google`");
    expect(detail(find("bastion-socks"))).toBe("config changed; restart to apply");
    expect(detail(find("grafana"))).toBe("");
  });
  it("reads like 2h 13m", () => {
    const now = Date.parse("2026-11-27T12:00:00Z");
    expect(uptime("2026-11-27T09:47:00Z", now)).toBe("2h 13m");
    expect(uptime("2026-11-27T11:59:15Z", now)).toBe("45s");
    expect(uptime("2026-11-25T11:00:00Z", now)).toBe("2d 1h");
    expect(uptime("not a time", now)).toBe("");
  });
  it("splits stored log lines", () => {
    const p = parseLine("2026-11-27T12:00:01.000Z err Listening on port [5433].");
    expect(p.source).toBe("err");
    expect(p.text).toBe("Listening on port [5433].");
  });
});

describe("Notifier", () => {
  const failed = (name: string): TunnelView => ({ ...find("prod-api"), name });

  it("combines failures with the same reason into one notice", () => {
    vi.useFakeTimers();
    const sent: { title: string; body: string }[] = [];
    const n = new Notifier((x) => sent.push(x), 2000, 300_000, () => Date.now());
    n.observe("running", failed("a"));
    n.observe("running", failed("b"));
    n.observe("running", failed("c"));
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(2000);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.title).toBe("3 tunnels failed");
    expect(sent[0]!.body).toContain("a, b, c: the google login has expired");
    vi.useRealTimers();
  });

  it("stays quiet for 5 minutes per tunnel and event", () => {
    let t = 0;
    const sent: string[] = [];
    const n = new Notifier((x) => sent.push(x.title), 0, 300_000, () => t);
    n.observe("running", failed("a"));
    n.flush();
    t = 60_000;
    n.observe("running", failed("a"));
    n.flush();
    expect(sent).toEqual(["a failed"]);
    t = 301_000;
    n.observe("running", failed("a"));
    n.flush();
    expect(sent).toEqual(["a failed", "a failed"]);
  });

  it("ignores changes that are not news", () => {
    const sent: string[] = [];
    const n = new Notifier((x) => sent.push(x.title), 0);
    n.observe(undefined, failed("first-sight"));
    n.observe("stopped", { ...find("staging-db"), state: "starting" });
    n.flush();
    expect(sent).toEqual([]);
  });
});

describe("timeline", () => {
  const now = Date.parse("2026-11-28T12:00:00Z");
  const at = (h: number) => new Date(now - h * 3600_000).toISOString();
  const ev = (h: number, tunnel: string, from: string, to: string) => ({ time: at(h), tunnel, from, to }) as never;

  it("is the current state for a day with no changes", () => {
    expect(timeline([], "db", "running", now, true)).toEqual([{ state: "running", start: now - DAY, end: now }]);
  });

  it("follows each change and joins equal neighbours", () => {
    const events = [ev(30, "db", "stopped", "running"), ev(5, "db", "running", "failed"), ev(4, "api", "stopped", "running"), ev(2, "db", "failed", "starting"), ev(1.9, "db", "starting", "running")];
    const segs = timeline(events, "db", "running", now, true);
    expect(segs.map((s) => s.state)).toEqual(["running", "failed", "starting", "running"]);
    expect(segs[0]!.start).toBe(now - DAY);
    expect(segs[1]!.end - segs[1]!.start).toBe(3 * 3600_000);
    expect(timelineSummary(segs)).toBe("running 20h 54m, failed 3h, connecting 6m");
  });

  it("marks the time before a cut-off log as no record", () => {
    const events = [ev(6, "api", "stopped", "running"), ev(3, "db", "running", "stopped")];
    const segs = timeline(events, "db", "stopped", now, false);
    expect(segs.map((s) => s.state)).toEqual(["unknown", "running", "stopped"]);
    expect(segs[0]!.end).toBe(Date.parse(at(6)));
  });
});
