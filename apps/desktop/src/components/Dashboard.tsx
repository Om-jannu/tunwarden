import { useEffect, useState } from "react";
import { smallPortal, wardenLantern } from "@/art/pixels";
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
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { toast } from "@/components/ui/toast";
import { api, selector } from "@/api";
import { byProfile, detail, health, isActive, summary, uptime } from "@/logic";
import type { State, TunnelView } from "@/protocol";
import { one, useStore, type View } from "@/store";

const STATE_TEXT: Record<State, string> = {
  running: "text-running",
  starting: "text-busy",
  reconnecting: "text-busy",
  unhealthy: "text-busy",
  failed: "text-failed",
  stopped: "text-stopped",
};

const RAIL: Record<State, string> = {
  running: "border-l-running",
  starting: "border-l-busy",
  reconnecting: "border-l-busy",
  unhealthy: "border-l-busy",
  failed: "border-l-failed",
  stopped: "border-l-transparent",
};

export function StateLabel({ state }: { state: State }) {
  const moving = state === "starting" || state === "reconnecting";
  return (
    <span className={cn("inline-flex items-center gap-1.5 font-medium", STATE_TEXT[state])}>
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

const VIEWS: { value: View; label: string }[] = [
  { value: "tunnels", label: "Tunnels" },
  { value: "logins", label: "Logins" },
  { value: "activity", label: "Activity" },
  { value: "doctor", label: "Doctor" },
  { value: "settings", label: "Settings" },
];

export function Header() {
  const tunnels = useStore((s) => s.tunnels);
  const view = useStore((s) => s.view);
  const expired = useStore((s) => s.expired.length);
  const setView = useStore((s) => s.setView);
  const h = health(tunnels);
  return (
    <header className="flex items-center gap-4 border-b bg-card px-4 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <Pixels art={smallPortal} scale={1} lamp={LAMP[h]} className="shrink-0" />
        <h1 className="text-[15px] font-semibold">Tunwarden</h1>
        <span className="truncate text-muted-foreground">{tunnels ? summary(tunnels) : "Connecting to the daemon"}</span>
      </div>
      <Tabs value={view} onValueChange={(v) => setView(v as View)} className="ml-auto">
        <TabsList>
          {VIEWS.map((v) => (
            <TabsTrigger key={v.value} value={v.value}>
              {v.label}
              {v.value === "logins" && expired > 0 && <span className="size-1.5 rounded-full bg-failed" aria-label={`${expired} expired`} />}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    </header>
  );
}

function Toolbar({ filter, setFilter }: { filter: string; setFilter: (s: string) => void }) {
  const tunnels = useStore((s) => s.tunnels);
  const groups = [...new Set((tunnels ?? []).flatMap((t) => t.groups))].sort();
  const act = useStore((s) => s.act);
  const edit = useStore((s) => s.edit);
  return (
    <div className="flex items-center gap-2 px-4 py-2">
      <div className="relative">
        <SearchIcon className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter tunnels" aria-label="Filter tunnels" className="h-7 w-56 pl-7" />
      </div>
      {groups.map((g) => (
        <Button key={g} variant="outline" size="sm" onClick={() => void act("start", selector({ group: g }))}>
          <PlayIcon data-icon="inline-start" />
          Start {g}
        </Button>
      ))}
      <span className="ml-auto flex gap-1">
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
      </span>
    </div>
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
        <Alert variant="destructive" className="rounded-none border-x-0 border-t-0">
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
        <Alert className="rounded-none border-x-0 border-t-0">
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
        <Alert key={l.name} className="rounded-none border-x-0 border-t-0">
          <AlertTitle>The {l.name} login has expired</AlertTitle>
          <AlertDescription>
            Tunnels using {l.profiles.length ? l.profiles.join(", ") : "it"} stopped. They start again on their own after you log in.
          </AlertDescription>
          <AlertAction className="flex gap-1">
            <Button size="sm" onClick={() => useStore.setState({ loggingIn: l.name })}>
              <LogInIcon data-icon="inline-start" />
              Log in
            </Button>
            <Button size="sm" variant="outline" onClick={() => void api().then((a) => a.loginInTerminal(l.name))}>
              <TerminalIcon data-icon="inline-start" />
              In Terminal
            </Button>
          </AlertAction>
        </Alert>
      ))}
    </>
  );
}

function Row({ t, now }: { t: TunnelView; now: number }) {
  const act = useStore((s) => s.act);
  const busy = useStore((s) => s.busy[t.name] ?? false);
  const setOpen = useStore((s) => s.setOpen);
  const active = isActive(t);
  const text = detail(t);
  return (
    <TableRow onClick={() => setOpen(t.name)} className="cursor-default">
      <TableCell className={cn("border-l-3 pl-3 font-medium", RAIL[t.state])}>{t.name}</TableCell>
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
      <TableCell className="text-muted-foreground tabular-nums">{t.restarts || ""}</TableCell>
      <TableCell className="max-w-0 w-full">
        <span className={cn("block truncate", t.state === "failed" ? "text-failed" : "text-muted-foreground")} title={text}>
          {text}
        </span>
      </TableCell>
      <TableCell className="pr-3" onClick={(e) => e.stopPropagation()}>
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
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Toolbar filter={filter} setFilter={setFilter} />
      <div className="min-h-0 flex-1 overflow-auto">
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
                <TableHead className="border-l-3 border-l-transparent pl-3">Name</TableHead>
                <TableHead>State</TableHead>
                <TableHead>Local</TableHead>
                <TableHead>Uptime</TableHead>
                <TableHead>Restarts</TableHead>
                <TableHead>Detail</TableHead>
                <TableHead className="pr-3">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            {groups.map((g) => (
              <TableBody key={g.profile ?? "none"}>
                <TableRow className="hover:bg-transparent">
                  <TableCell colSpan={6} className={cn("border-l-3 pt-4 pb-1 pl-3", g.production ? "border-l-production" : "border-l-transparent")}>
                    <span className="flex items-center gap-2">
                      <span className="font-semibold">{g.profile ?? "No profile"}</span>
                      {g.production && (
                        <Badge variant="outline" className="border-production text-production">
                          production
                        </Badge>
                      )}
                    </span>
                  </TableCell>
                  <TableCell className="pt-4 pr-3 pb-1">
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
                  <Row key={t.name} t={t} now={now} />
                ))}
              </TableBody>
            ))}
          </Table>
        )}
      </div>
    </div>
  );
}
