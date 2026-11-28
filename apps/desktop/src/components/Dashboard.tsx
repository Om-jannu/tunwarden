import { useEffect, useState } from "react";
import { wardenLantern } from "@/art/pixels";
import { LAMP, Pixels } from "@/components/Pixels";
import { CopyIcon, FileCodeIcon, LogInIcon, PlayIcon, PlusIcon, RotateCwIcon, SearchIcon, SquareIcon, TerminalIcon, UploadIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { toast } from "@/components/ui/toast";
import { api, selector } from "@/api";
import { byProfile, detail, isActive, uptime } from "@/logic";
import type { State, TunnelView } from "@/protocol";
import { one, useStore } from "@/store";
import { PageHeader } from "./Shell";
import { StateStrip } from "./StateStrip";
import { TunnelPane } from "./TunnelSheet";

const STATE_TEXT: Record<State, string> = {
  running: "text-running",
  starting: "text-busy",
  reconnecting: "text-busy",
  unhealthy: "text-busy",
  failed: "text-failed",
  stopped: "text-stopped",
};

export function StateLabel({ state }: { state: State }) {
  const moving = state === "starting" || state === "reconnecting";
  return (
    <span className={cn("inline-flex items-center gap-1.5 font-medium transition-colors duration-300 ease-out", STATE_TEXT[state])}>
      <span className={cn("size-1.5 rounded-full bg-current", moving && "motion-safe:animate-pulse")} />
      {state}
    </span>
  );
}

export async function copy(text: string, what = text) {
  await (await api()).copy(text);
  toast.add({ title: `Copied ${what}`, timeout: 1500 });
}

/** A local address you can copy, in monospace because you paste it elsewhere. */
export function Address({ value }: { value: string }) {
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        void copy(value);
      }}
      className="group inline-flex items-center gap-1 rounded-sm font-mono text-[12px] text-foreground hover:text-primary focus-visible:outline-2"
      title={`Copy ${value}`}
    >
      {value}
      <CopyIcon className="size-3 opacity-0 group-hover:opacity-60 group-focus-visible:opacity-60" />
    </button>
  );
}

function Toolbar({ filter, setFilter }: { filter: string; setFilter: (s: string) => void }) {
  const tunnels = useStore((s) => s.tunnels);
  const groups = [...new Set((tunnels ?? []).flatMap((t) => t.groups))].sort();
  const act = useStore((s) => s.act);
  const edit = useStore((s) => s.edit);
  return (
    <PageHeader title="Tunnels">
      <span className="relative">
        <SearchIcon className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter" aria-label="Filter tunnels" className="h-7 w-44 pl-7" />
      </span>
      {groups.map((g) => (
        <Button key={g} variant="outline" size="sm" onClick={() => void act("start", selector({ group: g }))}>
          <PlayIcon data-icon="inline-start" />
          Start {g}
        </Button>
      ))}
      <Button variant="ghost" size="sm" onClick={() => useStore.setState({ phase: "first-run" })}>
        <UploadIcon data-icon="inline-start" />
        Import
      </Button>
      <Button variant="ghost" size="sm" onClick={() => void api().then((a) => a.openConfig())}>
        <FileCodeIcon data-icon="inline-start" />
        Edit config
      </Button>
      <Button size="sm" onClick={() => edit({ table: "tunnels", name: null })}>
        <PlusIcon data-icon="inline-start" />
        Add tunnel
      </Button>
    </PageHeader>
  );
}

export function Banners() {
  const failure = useStore((s) => s.failure);
  const configErrors = useStore((s) => s.configErrors);
  const expired = useStore((s) => s.expired);
  const restartDaemon = useStore((s) => s.restartDaemon);
  return (
    <>
      {failure && (
        <Alert variant="destructive" className="mx-5 mt-1 mb-2 w-auto motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-200">
          <AlertTitle>
            {failure.kind === "protocol_mismatch"
              ? "The running daemon is a different version"
              : failure.kind === "start_failed"
                ? "The daemon could not start"
                : "Not connected to the daemon"}
          </AlertTitle>
          <AlertDescription>
            <span className="whitespace-pre-wrap">{failure.message}</span>
            {failure.hint ? ` ${failure.hint}.` : failure.kind === "protocol_mismatch" ? "" : " Retrying every 2 seconds."}
          </AlertDescription>
          {failure.kind === "protocol_mismatch" && (
            <AlertAction>
              <Button size="sm" onClick={() => void restartDaemon()}>
                Restart daemon
              </Button>
            </AlertAction>
          )}
        </Alert>
      )}
      {configErrors.length > 0 && (
        <Alert className="mx-5 mt-1 mb-2 w-auto motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-200">
          <AlertTitle>The config file has problems; the last valid version is in use</AlertTitle>
          <AlertDescription>
            <ul className="font-mono text-[12px]">
              {configErrors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          </AlertDescription>
          <AlertAction>
            <Button size="sm" variant="outline" onClick={() => void api().then((a) => a.openConfig())}>
              Edit config
            </Button>
          </AlertAction>
        </Alert>
      )}
      {expired.map((l) => (
        <section
          key={l.name}
          aria-label={`The ${l.name} login has expired`}
          className="mx-5 mt-1 mb-2 flex items-center gap-4 rounded-lg border bg-card px-4 py-3 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-200"
        >
          <Pixels art={wardenLantern} scale={2} lamp={LAMP.bad} className="shrink-0" />
          <div className="flex min-w-0 flex-col gap-0.5">
            <p className="font-semibold">The {l.name} login has expired</p>
            <p className="text-muted-foreground">
              {l.profiles.length ? `Tunnels in ${l.profiles.join(", ")}` : "Its tunnels"} stopped. They start again on their own after you log in.
            </p>
          </div>
          <span className="ml-auto flex shrink-0 gap-1.5">
            <Button size="sm" variant="outline" onClick={() => void api().then((a) => a.loginInTerminal(l.name))}>
              <TerminalIcon data-icon="inline-start" />
              In Terminal
            </Button>
            <Button size="sm" onClick={() => useStore.setState({ loggingIn: l.name })}>
              <LogInIcon data-icon="inline-start" />
              Log in
            </Button>
          </span>
        </section>
      ))}
    </>
  );
}

function Row({ t, now, compact }: { t: TunnelView; now: number; compact: boolean }) {
  const act = useStore((s) => s.act);
  const busy = useStore((s) => s.busy[t.name] ?? false);
  const setOpen = useStore((s) => s.setOpen);
  const selected = useStore((s) => s.open === t.name);
  const active = isActive(t);
  const text = detail(t);
  return (
    <TableRow
      onClick={() => setOpen(selected ? null : t.name)}
      aria-selected={selected}
      data-state={selected ? "selected" : undefined}
      className="cursor-default transition-colors duration-150 data-[state=selected]:bg-accent"
    >
      <TableCell className="pl-5 font-medium">
        <button
          type="button"
          className="rounded-sm text-left focus-visible:outline-2 focus-visible:outline-ring"
          onClick={(e) => {
            e.stopPropagation();
            setOpen(selected ? null : t.name);
          }}
        >
          {t.name}
        </button>
      </TableCell>
      <TableCell>
        <StateLabel state={t.state} />
      </TableCell>
      <TableCell>
        <span className="flex flex-col">
          {t.local.length ? t.local.map((a) => <Address key={a} value={a} />) : <span className="text-muted-foreground">—</span>}
        </span>
      </TableCell>
      <TableCell className="text-muted-foreground tabular-nums">
        {t.state === "running" || t.state === "unhealthy" ? uptime(t.since, now) : ""}
      </TableCell>
      {!compact && (
        <>
          <TableCell>
            <StateStrip t={t} />
          </TableCell>
          <TableCell className="text-muted-foreground tabular-nums">{t.restarts || ""}</TableCell>
          <TableCell className="max-w-0 w-full">
            <span className={cn("block truncate", t.state === "failed" ? "text-failed" : "text-muted-foreground")} title={text}>
              {text}
            </span>
          </TableCell>
        </>
      )}
      <TableCell className="pr-5" onClick={(e) => e.stopPropagation()}>
        <span className="flex justify-end gap-1">
          {busy ? (
            <Button variant="ghost" size="icon" disabled aria-label="Working">
              <Spinner />
            </Button>
          ) : active ? (
            <Button variant="ghost" size="icon" aria-label={`Stop ${t.name}`} title="Stop" onClick={() => void act("stop", one(t.name))}>
              <SquareIcon />
            </Button>
          ) : (
            <Button variant="ghost" size="icon" aria-label={`Start ${t.name}`} title="Start" onClick={() => void act("start", one(t.name))}>
              <PlayIcon />
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Restart ${t.name}`}
            title={t.pending_restart ? "Restart to apply the config change" : "Restart"}
            disabled={busy}
            className={cn(t.pending_restart && "text-primary")}
            onClick={() => void act("restart", one(t.name))}
          >
            <RotateCwIcon />
          </Button>
        </span>
      </TableCell>
    </TableRow>
  );
}

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}

export function TunnelsView() {
  const tunnels = useStore((s) => s.tunnels);
  const act = useStore((s) => s.act);
  const [filter, setFilter] = useState("");
  const now = useNow(1000);
  const groups = byProfile(tunnels ?? [], filter);
  const open = useStore((s) => s.open);
  // j / k move through the list (opening each tunnel beside it), space starts
  // or stops the selected one. Not while typing or with a dialog open.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const el = e.target;
      if (el instanceof Element && el.closest("input, textarea, select, [contenteditable=true], [role=dialog], [role=alertdialog]")) return;
      if (document.querySelector("[role=dialog], [role=alertdialog]")) return;
      const order = groups.flatMap((g) => g.tunnels);
      if (!order.length) return;
      const s = useStore.getState();
      const i = order.findIndex((t) => t.name === s.open);
      const next = e.key === "j" || e.key === "ArrowDown" ? order[Math.min(i + 1, order.length - 1)] : e.key === "k" || e.key === "ArrowUp" ? order[Math.max(i - 1, 0)] : undefined;
      const selected = order[i];
      if (next) {
        e.preventDefault();
        s.setOpen(next.name);
      } else if (e.key === " " && selected) {
        e.preventDefault();
        void s.act(isActive(selected) ? "stop" : "start", one(selected.name));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [groups]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Toolbar filter={filter} setFilter={setFilter} />
      <Banners />
      <div className="flex min-h-0 flex-1">
      <div className="min-h-0 min-w-0 flex-1 overflow-auto">
        {tunnels === null ? (
          <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
            <Spinner /> Waiting for the daemon
          </div>
        ) : tunnels.length === 0 ? (
          <Empty className="h-full">
            <EmptyHeader>
              <Pixels art={wardenLantern} scale={3} lamp={LAMP.idle} className="mb-2" />
              <EmptyTitle>No tunnels yet</EmptyTitle>
              <EmptyDescription>Import them from your shell aliases, or add one.</EmptyDescription>
            </EmptyHeader>
            <EmptyContent className="flex-row justify-center">
              <Button onClick={() => useStore.setState({ phase: "first-run" })}>Import tunnels</Button>
              <Button variant="outline" onClick={() => useStore.getState().edit({ table: "tunnels", name: null })}>
                Add tunnel
              </Button>
              <Button variant="outline" onClick={() => void api().then((a) => a.openConfig())}>
                Edit config
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <Table>
            <TableHeader className="sticky top-0 z-10 bg-background">
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5">Name</TableHead>
                <TableHead>State</TableHead>
                <TableHead>Local</TableHead>
                <TableHead>Uptime</TableHead>
                {!open && (
                  <>
                    <TableHead>Last 24 h</TableHead>
                    <TableHead>Restarts</TableHead>
                    <TableHead>Detail</TableHead>
                  </>
                )}
                <TableHead className="pr-5">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            {groups.map((g) => (
              <TableBody key={g.profile ?? "none"}>
                <TableRow className="hover:bg-transparent">
                  <TableCell colSpan={open ? 4 : 7} className="pt-4 pb-1 pl-5">
                    <span className="flex items-center gap-2">
                      <span className="text-[12px] font-semibold tracking-[0.02em] text-muted-foreground uppercase">{g.profile ?? "No profile"}</span>
                      {g.production && (
                        <Badge variant="outline" className="border-production text-production">
                          production
                        </Badge>
                      )}
                    </span>
                  </TableCell>
                  <TableCell className="pt-4 pr-5 pb-1">
                    {g.profile && (
                      <span className="flex justify-end gap-1">
                        <Button variant="ghost" size="xs" onClick={() => void act("start", selector({ profile: g.profile }))}>
                          Start all
                        </Button>
                        <Button variant="ghost" size="xs" onClick={() => void act("stop", selector({ profile: g.profile }))}>
                          Stop all
                        </Button>
                      </span>
                    )}
                  </TableCell>
                </TableRow>
                {g.tunnels.map((t) => (
                  <Row key={t.name} t={t} now={now} compact={open !== null} />
                ))}
              </TableBody>
            ))}
          </Table>
        )}
      </div>
      {open && <TunnelPane />}
      </div>
    </div>
  );
}
