import { useEffect } from "react";
import { AppWindowIcon, LogInIcon } from "lucide-react";
import { smallPortal } from "@/art/pixels";
import { api } from "@/api";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { ConfirmProduction } from "@/components/Dialogs";
import { cn } from "@/lib/utils";
import { byProfile, detail, health, isActive, summary } from "@/logic";
import type { TunnelView } from "@/protocol";
import { one, useStore } from "@/store";
import { StateLabel } from "./Dashboard";
import { LAMP, Pixels } from "./Pixels";

/** The window that opens from the tray icon: every tunnel with its lamp and a
 *  switch, an expired login, and the way into the full window. Esc closes it. */
export function TrayPopover() {
  const phase = useStore((s) => s.phase);
  const tunnels = useStore((s) => s.tunnels);
  const expired = useStore((s) => s.expired);
  const failure = useStore((s) => s.failure);
  useEffect(() => {
    void useStore.getState().start();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector("[role=alertdialog]")) void api().then((a) => a.hidePopover());
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const h = health(tunnels);
  const open = (login?: string) => void api().then((a) => a.openMain(login));
  const groups = byProfile(tunnels ?? [], "");
  return (
    <div className="popover-shell flex h-dvh flex-col text-[13px]">
      <header className="flex items-center gap-2.5 px-3.5 pt-3 pb-2.5">
        <Pixels art={smallPortal} scale={2} lamp={LAMP[h]} className="shrink-0" />
        <div className="flex min-w-0 flex-col">
          <span className="font-semibold leading-tight">Tunwarden</span>
          <span className="truncate text-[12px] text-muted-foreground" aria-live="polite">
            {tunnels ? summary(tunnels) : failure ? "Not connected to the daemon" : "Connecting"}
          </span>
        </div>
        <Button variant="ghost" size="icon" className="ml-auto" aria-label="Open Tunwarden" title="Open Tunwarden" onClick={() => open()}>
          <AppWindowIcon />
        </Button>
      </header>
      {expired.map((l) => (
        <div key={l.name} className="mx-2 mb-1.5 flex items-center gap-2 rounded-md bg-failed/10 px-2.5 py-2 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-200">
          <span className="size-1.5 shrink-0 rounded-full bg-failed" aria-hidden />
          <span className="min-w-0 flex-1 truncate">
            The <span className="font-medium">{l.name}</span> login has expired
          </span>
          <Button size="xs" onClick={() => open(l.name)}>
            <LogInIcon data-icon="inline-start" />
            Log in
          </Button>
        </div>
      ))}
      <div className="min-h-0 flex-1 overflow-auto border-t px-1.5 py-1.5">
        {phase === "loading" || tunnels === null ? (
          <p className="flex items-center gap-2 px-2 py-3 text-muted-foreground">
            <Spinner /> {failure ? failure.message : "Waiting for the daemon"}
          </p>
        ) : tunnels.length === 0 ? (
          <p className="px-2 py-3 text-muted-foreground">No tunnels yet. Open Tunwarden to add or import them.</p>
        ) : (
          groups.map((g) => (
            <section key={g.profile ?? "none"} aria-label={g.profile ?? "No profile"} className="pb-1">
              <h3 className="flex items-center gap-1.5 px-2 pt-1.5 pb-0.5 text-[11px] font-semibold tracking-[0.02em] text-muted-foreground uppercase">
                {g.profile ?? "No profile"}
                {g.production && <span className="text-production normal-case">production</span>}
              </h3>
              <ul>
                {g.tunnels.map((t) => (
                  <Item key={t.name} t={t} />
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
      <footer className="border-t px-1.5 py-1.5">
        <button
          type="button"
          onClick={() => open()}
          className="flex h-7 w-full items-center rounded-md px-2 text-left hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
        >
          Open Tunwarden
        </button>
      </footer>
      <ConfirmProduction />
    </div>
  );
}

function Item({ t }: { t: TunnelView }) {
  const act = useStore((s) => s.act);
  const busy = useStore((s) => s.busy[t.name] ?? false);
  const on = isActive(t);
  const why = t.state === "failed" ? detail(t) : "";
  return (
    <li className="flex items-center gap-2 rounded-md px-2 py-1.5 transition-colors duration-150 hover:bg-accent/60">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex items-center gap-2">
          <span className="truncate font-medium">{t.name}</span>
          <span className="ml-auto shrink-0 text-[12px]">
            <StateLabel state={t.state} />
          </span>
        </span>
        <span className={cn("truncate font-mono text-[11px]", why ? "text-failed" : "text-muted-foreground")} title={why || undefined}>
          {why || t.local.join(", ") || "no local port"}
        </span>
      </div>
      {busy ? (
        <Spinner className="mx-2" />
      ) : (
        <Switch
          checked={on}
          aria-label={`${on ? "Stop" : "Start"} ${t.name}`}
          onCheckedChange={(v) => void act(v ? "start" : "stop", one(t.name))}
        />
      )}
    </li>
  );
}
