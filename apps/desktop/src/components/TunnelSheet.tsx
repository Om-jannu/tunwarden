import { useEffect, useMemo, useRef, useState } from "react";
import { CopyIcon, PencilIcon, PlayIcon, RotateCwIcon, SquareIcon, Trash2Icon, XIcon } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { toast } from "@/components/ui/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { api } from "@/api";
import { detail, isActive, uptime } from "@/logic";
import { one, useStore } from "@/store";
import { Address, StateLabel, copy } from "./Dashboard";
import { StateStrip } from "./StateStrip";

const SHOWN = 2000;

/** A stored log line (`<UTC time> <out|err|tw> <text>`) split for display. */
export function parseLine(line: string): { time: string; source: string; text: string } {
  const [ts = "", source = "", ...rest] = line.split(" ");
  const d = new Date(ts);
  const time = Number.isNaN(d.getTime())
    ? ts
    : d.toLocaleTimeString(undefined, { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit" });
  return { time, source, text: rest.join(" ") };
}

function Logs({ name }: { name: string }) {
  const [lines, setLines] = useState<string[]>([]);
  const [follow, setFollow] = useState(true);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const r = await (await api()).logsTail(name, SHOWN);
        if (alive) {
          setLines(r.lines);
          setError(null);
        }
      } catch (e) {
        if (alive) setError(String((e as { message?: string }).message ?? e));
      }
    };
    void load();
    if (!follow) return () => void (alive = false);
    const id = setInterval(load, 1000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [name, follow]);

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? lines.filter((l) => l.toLowerCase().includes(q)) : lines;
  }, [lines, filter]);

  useEffect(() => {
    // Scroll the log box only, never the pane around it.
    const el = box.current;
    if (follow && el) el.scrollTop = el.scrollHeight;
  }, [shown, follow]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex items-center gap-3">
        <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter the log" aria-label="Filter the log" className="h-7 flex-1" />
        <Label className="flex items-center gap-1.5 font-normal">
          <Checkbox checked={follow} onCheckedChange={(v) => setFollow(v === true)} />
          Follow
        </Label>
        <Button variant="outline" size="sm" onClick={() => void copy(shown.slice(-100).join("\n"), "the last 100 lines")}>
          <CopyIcon data-icon="inline-start" />
          Copy last 100
        </Button>
      </div>
      <div ref={box} className="min-h-[160px] flex-1 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-[11.5px] leading-[1.55]" role="log" aria-live="off">
        {error && <p className="text-failed">{error}</p>}
        {shown.length === 0 && !error && <p className="text-muted-foreground">{filter ? "No lines match." : "Nothing logged yet."}</p>}
        {shown.map((l, i) => {
          const p = parseLine(l);
          return (
            <div key={`${i}-${l.slice(0, 24)}`} className="flex gap-2 whitespace-pre-wrap break-all">
              <span className="shrink-0 text-muted-foreground">{p.time}</span>
              <span className={cn("w-[3ch] shrink-0", p.source === "err" ? "text-busy" : p.source === "tw" ? "text-primary" : "text-muted-foreground")}>
                {p.source}
              </span>
              <span>{p.text}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function RemoveTunnel({ name, running }: { name: string; running: boolean }) {
  const [open, setOpen] = useState(false);
  const remove = async () => {
    try {
      await (await api()).removeEntry("tunnels", name);
      useStore.getState().setOpen(null);
      toast.add({ title: `Removed ${name}`, timeout: 2000 });
    } catch (e) {
      toast.add({ title: `Cannot remove ${name}`, description: String((e as { message?: string }).message ?? e) });
    }
  };
  return (
    <>
      <Button size="sm" variant="ghost" className="ml-auto" onClick={() => setOpen(true)}>
        <Trash2Icon data-icon="inline-start" />
        Remove
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {name}?</AlertDialogTitle>
            <AlertDialogDescription>
              {running ? "It stops now and " : "It "}is removed from your config file. Its log stays on disk.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void remove()}>
              Remove {name}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

/** The selected tunnel, beside the list: its state, command facts, actions
 *  and live log. Esc or the close button deselects it. */
export function TunnelPane() {
  const t = useStore((s) => s.tunnels?.find((x) => x.name === s.open) ?? null);
  const setOpen = useStore((s) => s.setOpen);
  const act = useStore((s) => s.act);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector("[role=dialog],[role=alertdialog]")) setOpen(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);
  if (!t) return null;
  const facts: [string, React.ReactNode][] = [
    ["Type", t.type],
    ["Profile", t.profile ?? "none"],
    ["Groups", t.groups.join(", ") || "none"],
    ["PID", t.pid ?? "—"],
    ["Uptime", t.state === "running" ? uptime(t.since) : "—"],
    ["Restarts", t.restarts],
  ];
  return (
    <aside
      aria-label={`${t.name} details`}
      className="flex min-h-0 w-[42%] max-w-[600px] min-w-[340px] shrink-0 flex-col gap-4 overflow-y-auto border-l bg-card px-5 pt-1 pb-4 motion-safe:animate-in motion-safe:slide-in-from-right-4 motion-safe:fade-in-0 motion-safe:duration-200"
    >
      <header className="flex items-start gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h3 className="flex items-center gap-3 text-[15px] font-semibold">
            <span className="truncate">{t.name}</span>
            <StateLabel state={t.state} />
          </h3>
          <span className="flex flex-wrap gap-3">
            {t.local.map((a) => (
              <Address key={a} value={a} />
            ))}
          </span>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto shrink-0" aria-label="Close details" title="Close (Esc)" onClick={() => setOpen(null)}>
          <XIcon />
        </Button>
      </header>
      <div className="flex flex-wrap gap-1.5">
        {isActive(t) ? (
          <Button size="sm" variant="outline" onClick={() => void act("stop", one(t.name))}>
            <SquareIcon data-icon="inline-start" />
            Stop
          </Button>
        ) : (
          <Button size="sm" onClick={() => void act("start", one(t.name))}>
            <PlayIcon data-icon="inline-start" />
            Start
          </Button>
        )}
        <Button size="sm" variant="outline" onClick={() => void act("restart", one(t.name))}>
          <RotateCwIcon data-icon="inline-start" />
          {t.pending_restart ? "Restart to apply changes" : "Restart"}
        </Button>
        <Button size="sm" variant="outline" onClick={() => useStore.getState().edit({ table: "tunnels", name: t.name })}>
          <PencilIcon data-icon="inline-start" />
          Edit
        </Button>
        <RemoveTunnel name={t.name} running={isActive(t)} />
      </div>
      {detail(t) && <p className={cn(t.state === "failed" ? "text-failed" : "text-muted-foreground")}>{detail(t)}</p>}
      <section aria-label="Last 24 hours" className="flex flex-col gap-1.5">
        <h4 className="text-[12px] font-semibold tracking-[0.02em] text-muted-foreground uppercase">Last 24 hours</h4>
        <StateStrip t={t} size="pane" />
      </section>
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
        {facts.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-muted-foreground">{k}</dt>
            <dd className="tabular-nums">{v}</dd>
          </div>
        ))}
      </dl>
      <Separator />
      <Logs name={t.name} />
    </aside>
  );
}
