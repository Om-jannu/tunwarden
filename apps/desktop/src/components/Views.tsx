import { useEffect, useRef, useState } from "react";
import { wardenLantern, wardenRest } from "@/art/pixels";
import { LAMP, Pixels } from "@/components/Pixels";
import { LogInIcon, PencilIcon, PlusIcon, RefreshCwIcon, TerminalIcon, Trash2Icon } from "lucide-react";
import { cn } from "@/lib/utils";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { toast } from "@/components/ui/toast";
import { api, asFailure, type AutostartInfo, type ConfigEntries } from "@/api";
import type { TableName } from "@/forms";
import type { Mark, Report } from "@/protocol";
import { useStore } from "@/store";
import { Banners, StateLabel } from "./Dashboard";
import { PageHeader } from "./Shell";

const RESULT: Record<string, string> = {
  ok: "text-running",
  expired: "text-failed",
  unknown: "text-busy",
  "not checked": "text-muted-foreground",
};

function ago(iso: string | null): string {
  if (!iso) return "";
  const s = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 1000));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${Math.round(s / 3600)} h ago`;
}

/** Remove a profile or login after asking; the daemon refuses one still in use. */
function RemoveButton({ table, name }: { table: TableName; name: string }) {
  const [open, setOpen] = useState(false);
  const remove = async () => {
    try {
      await (await api()).removeEntry(table, name);
      toast.add({ title: `Removed ${name}`, timeout: 2000 });
      void useStore.getState().loadLogins();
    } catch (e) {
      toast.add({ title: `Cannot remove ${name}`, description: asFailure(e).message });
    }
  };
  return (
    <>
      <Button variant="ghost" size="icon" aria-label={`Remove ${name}`} title="Remove" onClick={() => setOpen(true)}>
        <Trash2Icon />
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {name}?</AlertDialogTitle>
            <AlertDialogDescription>It is removed from your config file. Nothing else changes.</AlertDialogDescription>
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

/** Runs a login's command inside the app and shows what it prints (FR-25). */
export function LoginRunDialog() {
  const name = useStore((s) => s.loggingIn);
  const [lines, setLines] = useState<string[]>([]);
  const [state, setState] = useState<"running" | "done" | "failed">("running");
  const [result, setResult] = useState("");
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!name) return;
    setLines([]);
    setState("running");
    setResult("");
    let stop: (() => void) | null = null;
    void (async () => {
      const a = await api();
      stop = await a.onLoginOutput((l) => l.login === name && setLines((x) => [...x, l.line]));
      try {
        const r = await a.loginRun(name);
        setState("done");
        setResult(r.restarted.length ? `Logged in. Restarting ${r.restarted.join(", ")}.` : "Logged in. No tunnels were waiting for this login.");
        void useStore.getState().loadLogins();
      } catch (e) {
        const f = asFailure(e);
        setState("failed");
        setResult(f.hint ? `${f.message}. ${f.hint}.` : f.message);
      }
    })();
    return () => stop?.();
  }, [name]);

  useEffect(() => end.current?.scrollIntoView?.({ block: "end" }), [lines]);

  const close = () => useStore.setState({ loggingIn: null });
  return (
    <Dialog open={name !== null} onOpenChange={(o) => !o && state !== "running" && close()}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>Log in to {name}</DialogTitle>
          <DialogDescription>
            {state === "running" ? "Finish the login in your browser if one opened. Its output appears below." : result}
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-64 min-h-24 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-[11.5px] leading-[1.55] whitespace-pre-wrap break-all" role="log">
          {lines.length === 0 && state === "running" && <span className="text-muted-foreground">Waiting for output</span>}
          {lines.map((l, i) => (
            <div key={i}>{l}</div>
          ))}
          <div ref={end} />
        </div>
        <DialogFooter>
          {state !== "done" && name && (
            <Button variant="outline" onClick={() => void api().then((a) => a.loginInTerminal(name))}>
              <TerminalIcon data-icon="inline-start" />
              Run in Terminal instead
            </Button>
          )}
          <Button onClick={close} disabled={state === "running"}>
            {state === "running" ? <Spinner data-icon="inline-start" /> : null}
            {state === "running" ? "Logging in" : "Close"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function LoginsView() {
  const logins = useStore((s) => s.logins);
  const loadLogins = useStore((s) => s.loadLogins);
  const edit = useStore((s) => s.edit);
  const [entries, setEntries] = useState<ConfigEntries | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    void loadLogins();
    void api().then(async (a) => setEntries(await a.configEntries()));
  }, [loadLogins, logins.length]);

  const checkNow = async () => {
    setChecking(true);
    try {
      const r = await (await api()).authCheck();
      useStore.setState({ logins: r.logins, expired: r.logins.filter((l) => l.result === "expired") });
    } catch (e) {
      toast.add({ title: "The check did not run", description: asFailure(e).message });
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="flex flex-col">
      <PageHeader title="Logins" note="Checked every few minutes while a tunnel uses them.">
        <Button variant="outline" size="sm" disabled={checking} onClick={() => void checkNow()}>
          {checking ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
          Check now
        </Button>
        <Button size="sm" onClick={() => edit({ table: "logins", name: null })}>
          <PlusIcon data-icon="inline-start" />
          Add login
        </Button>
      </PageHeader>
      <Banners logins={false} />
    <div className="flex flex-col gap-8 px-5 pb-5">
      <section className="flex flex-col gap-2">
        {logins.length === 0 ? (
          <Empty className="border">
            <EmptyHeader>
              <Pixels art={wardenLantern} scale={3} lamp={LAMP.idle} className="mb-2" />
              <EmptyTitle>No logins</EmptyTitle>
              <EmptyDescription>Add one for a tool whose sign-in expires, such as gcloud or AWS SSO, so expired tunnels stop with a clear reason and restart after you log in.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>Login</TableHead>
                <TableHead>Used by</TableHead>
                <TableHead>Result</TableHead>
                <TableHead>Detail</TableHead>
                <TableHead>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {logins.map((l) => (
                <TableRow key={l.name}>
                  <TableCell className="font-medium">{l.name}</TableCell>
                  <TableCell className="text-muted-foreground">{l.profiles.join(", ") || "no profile"}</TableCell>
                  <TableCell className={cn("font-medium", RESULT[l.result])}>{l.checkable ? l.result : "no check"}</TableCell>
                  <TableCell className="w-full max-w-0">
                    <span className="block truncate text-muted-foreground" title={l.detail ?? ""}>
                      {l.detail}
                      {l.checked_at ? ` (${ago(l.checked_at)})` : ""}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="flex justify-end gap-1">
                      <Button size="sm" variant={l.result === "expired" ? "default" : "outline"} onClick={() => useStore.setState({ loggingIn: l.name })}>
                        <LogInIcon data-icon="inline-start" />
                        Log in
                      </Button>
                      <Button variant="ghost" size="icon" aria-label={`Run the ${l.name} login in Terminal`} title="Run in Terminal" onClick={() => void api().then((a) => a.loginInTerminal(l.name))}>
                        <TerminalIcon />
                      </Button>
                      <Button variant="ghost" size="icon" aria-label={`Edit ${l.name}`} title="Edit" onClick={() => edit({ table: "logins", name: l.name })}>
                        <PencilIcon />
                      </Button>
                      <RemoveButton table="logins" name={l.name} />
                    </span>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-[12px] font-semibold tracking-[0.02em] text-muted-foreground uppercase">Profiles</h3>
          <span className="text-muted-foreground">Environment, a login and the production flag, shared by tunnels.</span>
          <Button size="sm" className="ml-auto" onClick={() => edit({ table: "profiles", name: null })}>
            <PlusIcon data-icon="inline-start" />
            Add profile
          </Button>
        </div>
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>Profile</TableHead>
              <TableHead>Login</TableHead>
              <TableHead>Environment</TableHead>
              <TableHead>
                <span className="sr-only">Actions</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {(entries?.profiles ?? []).map((p) => {
              const name = String(p.name);
              const env = p.env && typeof p.env === "object" ? Object.keys(p.env as object) : [];
              return (
                <TableRow key={name}>
                  <TableCell className="font-medium">
                    <span className="flex items-center gap-2">
                      {name}
                      {p.production === true && (
                        <Badge variant="outline" className="border-production text-production">
                          production
                        </Badge>
                      )}
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground">{typeof p.login === "string" ? p.login : "none"}</TableCell>
                  <TableCell className="w-full max-w-0">
                    <span className="block truncate font-mono text-[12px] text-muted-foreground">{env.join(", ")}</span>
                  </TableCell>
                  <TableCell>
                    <span className="flex justify-end gap-1">
                      <Button variant="ghost" size="icon" aria-label={`Edit ${name}`} title="Edit" onClick={() => edit({ table: "profiles", name })}>
                        <PencilIcon />
                      </Button>
                      <RemoveButton table="profiles" name={name} />
                    </span>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </section>
    </div>
    </div>
  );
}

export function ActivityView() {
  const live = useStore((s) => s.activity);
  const [past, setPast] = useState<typeof live>([]);
  useEffect(() => {
    void api()
      .then((a) => a.activity(200))
      .then((r) => setPast(r.events))
      .catch(() => {});
  }, []);
  const seen = new Set(past.map((e) => `${e.time}${e.tunnel}`));
  const events = [...past, ...live.filter((e) => !seen.has(`${e.time}${e.tunnel}`))].reverse();
  return (
    <div className="flex flex-col">
      <PageHeader title="Activity" note="Starts, stops, failures and reconnects, newest first." />
      <Banners logins={false} />
    <div className="px-5 pb-5">
      {events.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <Pixels art={wardenRest} scale={3} className="mb-2" />
            <EmptyTitle>Nothing has happened yet</EmptyTitle>
            <EmptyDescription>Starts, stops, failures and reconnects of every tunnel appear here.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>Time</TableHead>
              <TableHead>Tunnel</TableHead>
              <TableHead>Change</TableHead>
              <TableHead>Detail</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {events.map((e, i) => (
              <TableRow key={`${e.time}-${e.tunnel}-${i}`}>
                <TableCell className="text-muted-foreground tabular-nums">
                  {new Date(e.time).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })}
                </TableCell>
                <TableCell className="font-medium">{e.tunnel}</TableCell>
                <TableCell>
                  <span className="flex items-center gap-1.5">
                    <span className="text-muted-foreground">{e.from}</span>
                    <span className="text-muted-foreground" aria-hidden>
                      to
                    </span>
                    <StateLabel state={e.to} />
                  </span>
                </TableCell>
                <TableCell className="w-full max-w-0">
                  <span className={cn("block truncate", e.to === "failed" ? "text-failed" : "text-muted-foreground")} title={e.message ?? ""}>
                    {[e.reason, e.message].filter(Boolean).join(": ")}
                  </span>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
    </div>
  );
}

const MARK: Record<Mark, string> = { ok: "text-running", warning: "text-busy", problem: "text-failed" };

export function DoctorView() {
  const [report, setReport] = useState<Report | null>(null);
  const [running, setRunning] = useState(false);
  const run = async () => {
    setRunning(true);
    try {
      setReport(await (await api()).doctor());
    } finally {
      setRunning(false);
    }
  };
  useEffect(() => void run(), []);
  const sections = [...new Set((report?.rows ?? []).map((r) => r.section))];
  const problems = (report?.rows ?? []).filter((r) => r.mark === "problem").length;
  return (
    <div className="flex flex-col">
      <PageHeader
        title="Doctor"
        note={report === null ? "Checking the config, daemon, tools, logins and ports" : problems === 0 ? "No problems found." : `${problems} ${problems === 1 ? "problem" : "problems"} to fix.`}
      >
        <Button variant="outline" size="sm" disabled={running} onClick={() => void run()}>
          {running ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
          Check again
        </Button>
      </PageHeader>
      <Banners logins={false} />
    <div className="flex flex-col gap-4 px-5 pb-5">
      {sections.map((s) => (
        <section key={s} className="flex flex-col">
          <h3 className="pb-1 font-semibold">{s}</h3>
          <ul className="flex flex-col divide-y rounded-md border bg-card">
            {report!.rows
              .filter((r) => r.section === s)
              .map((r, i) => (
                <li key={i} className="grid grid-cols-[5rem_minmax(14rem,auto)_1fr] gap-3 px-3 py-1.5">
                  <span className={cn("font-medium", MARK[r.mark])}>{r.mark}</span>
                  <span className="break-all">{r.item}</span>
                  <span className="text-muted-foreground">{r.detail}</span>
                </li>
              ))}
          </ul>
        </section>
      ))}
    </div>
    </div>
  );
}

export function SettingsView() {
  const config = useStore((s) => s.config);
  const [auto, setAuto] = useState<AutostartInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void api()
      .then((a) => a.autostartStatus())
      .then(setAuto)
      .catch((e) => setError(asFailure(e).message));
  }, []);
  const toggle = async (enabled: boolean) => {
    setError(null);
    try {
      setAuto(await (await api()).autostartSet(enabled));
    } catch (e) {
      setError(asFailure(e).message);
    }
  };
  return (
    <div className="flex flex-col">
      <PageHeader title="Settings" />
      <Banners logins={false} />
    <div className="flex max-w-[640px] flex-col gap-6 px-5 pb-5">
      <Field orientation="horizontal">
        <Switch id="autostart" checked={auto?.enabled ?? false} disabled={auto === null} onCheckedChange={(v) => void toggle(v)} />
        <div className="flex flex-col gap-0.5">
          <FieldLabel htmlFor="autostart">Start the daemon when you log in</FieldLabel>
          <FieldDescription>
            Tunnels that were running come back after a restart, without opening the app. It takes effect at your next login.
            {auto?.enabled ? ` Entry: ${auto.location}` : ""}
          </FieldDescription>
        </div>
      </Field>
      {auto?.problem && (
        <Alert variant="destructive">
          <AlertTitle>Autostart will not work</AlertTitle>
          <AlertDescription>{auto.problem}</AlertDescription>
        </Alert>
      )}
      {auto?.other_binary && (
        <Alert>
          <AlertTitle>Autostart uses another copy of Tunwarden</AlertTitle>
          <AlertDescription>It starts {auto.other_binary}. Turn it off and on again to use the one that comes with this app.</AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      <div className="flex flex-col gap-1">
        <h3 className="font-semibold">Config file</h3>
        <code className="font-mono text-[12px]">{config?.path}</code>
        <p className="text-muted-foreground">Edit it by hand any time; the daemon picks up changes while it runs.</p>
        <Button variant="outline" size="sm" className="self-start" onClick={() => void api().then((a) => a.openConfig())}>
          Open in editor
        </Button>
      </div>
    </div>
    </div>
  );
}
