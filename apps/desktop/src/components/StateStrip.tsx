import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { DAY, timeline, timelineSummary, type Segment } from "@/logic";
import type { TunnelView } from "@/protocol";
import { useStore } from "@/store";

const FILL: Record<Segment["state"], string> = {
  running: "bg-running",
  starting: "bg-busy",
  reconnecting: "bg-busy",
  unhealthy: "bg-busy",
  failed: "bg-failed",
  stopped: "bg-muted-foreground/25",
  unknown: "bg-transparent",
};

function useMinute() {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return now;
}

const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

/** The tunnel's last 24 hours as one bar: green running, amber connecting,
 *  red failed, grey stopped, blank where the log has no record. The bar's
 *  label says the same in words. */
export function StateStrip({ t, size = "row" }: { t: TunnelView; size?: "row" | "pane" }) {
  const history = useStore((s) => s.history);
  const complete = useStore((s) => s.historyComplete);
  const now = useMinute();
  const segments = useMemo(() => timeline(history, t.name, t.state, Math.max(now, Date.now()), complete), [history, t.name, t.state, now, complete]);
  const words = timelineSummary(segments);
  const start = now - DAY;
  return (
    <div className={cn("flex flex-col gap-1", size === "row" ? "w-[132px]" : "w-full")}>
      <div
        role="img"
        aria-label={`Last 24 hours: ${words || "no record"}`}
        title={size === "row" ? `Last 24 hours: ${words || "no record"}` : undefined}
        className={cn("flex overflow-hidden rounded-full bg-muted", size === "row" ? "h-1.5" : "h-2.5")}
      >
        {segments.map((s) => (
          <span
            key={s.start}
            className={cn("h-full transition-[flex-grow] duration-500", FILL[s.state])}
            style={{ flexGrow: s.end - s.start, flexBasis: 0, minWidth: s.state === "failed" || s.state === "reconnecting" ? 2 : 0 }}
            title={size === "pane" ? `${s.state === "unknown" ? "no record" : s.state}, ${clock(s.start)}–${clock(s.end)}` : undefined}
          />
        ))}
      </div>
      {size === "pane" && (
        <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
          <span>{clock(start)}</span>
          <span className="truncate px-2">{words}</span>
          <span>now</span>
        </div>
      )}
    </div>
  );
}
