import { ActivityIcon, KeyRoundIcon, SettingsIcon, StethoscopeIcon, WaypointsIcon } from "lucide-react";
import { smallPortal } from "@/art/pixels";
import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { health, summary } from "@/logic";
import { useStore, type View } from "@/store";
import { LAMP, Pixels } from "./Pixels";

const NAV: { value: View; label: string; icon: typeof ActivityIcon }[] = [
  { value: "tunnels", label: "Tunnels", icon: WaypointsIcon },
  { value: "logins", label: "Logins", icon: KeyRoundIcon },
  { value: "activity", label: "Activity", icon: ActivityIcon },
  { value: "doctor", label: "Doctor", icon: StethoscopeIcon },
  { value: "settings", label: "Settings", icon: SettingsIcon },
];

/** The window's left column: the mark and overall state, then the pages. On
 *  macOS the traffic lights sit in its top strip, which drags the window. */
export function Sidebar() {
  const tunnels = useStore((s) => s.tunnels);
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);
  const expired = useStore((s) => s.expired.length);
  const version = useStore((s) => s.version);
  const failure = useStore((s) => s.failure);
  const h = health(tunnels);
  const running = tunnels?.filter((t) => t.state === "running").length ?? 0;
  return (
    <nav aria-label="Pages" className="app-sidebar flex w-[212px] shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground">
      <div data-tauri-drag-region className="h-3 shrink-0 [[data-os=macos][data-frame=native]_&]:h-11" />
      <div data-tauri-drag-region className="flex items-center gap-2.5 px-4 pb-4">
        <Pixels art={smallPortal} scale={2} lamp={LAMP[h]} className="shrink-0" />
        <div className="flex min-w-0 flex-col">
          <span className="text-[14px] font-semibold leading-tight">Tunwarden</span>
          <span className="text-[12px] leading-snug text-muted-foreground" aria-live="polite">
            {tunnels ? summary(tunnels) : failure ? "Not connected" : "Connecting"}
          </span>
        </div>
      </div>
      <ul className="flex flex-col gap-px px-2">
        {NAV.map((n) => {
          const current = view === n.value;
          return (
            <li key={n.value}>
              <button
                type="button"
                onClick={() => setView(n.value)}
                aria-current={current ? "page" : undefined}
                className={cn(
                  "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13px] text-sidebar-foreground/80 transition-colors duration-150",
                  "hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:outline-2 focus-visible:outline-ring",
                  current && "bg-sidebar-accent font-medium text-sidebar-foreground",
                )}
              >
                <n.icon className="size-4 shrink-0 opacity-75" aria-hidden />
                {n.label}
                {n.value === "tunnels" && tunnels && tunnels.length > 0 && (
                  <span className="ml-auto text-[12px] tabular-nums text-muted-foreground" title={`${running} of ${tunnels.length} running`}>
                    {running}/{tunnels.length}
                  </span>
                )}
                {n.value === "logins" && expired > 0 && (
                  <span className="ml-auto flex items-center gap-1 text-[12px] text-failed">
                    <span className="size-1.5 rounded-full bg-failed" aria-hidden />
                    {expired} expired
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      <div className="mt-auto flex items-center gap-2 px-4 py-3 text-[11.5px] text-muted-foreground">
        <span className="truncate">{failure ? "Daemon not connected" : version ? `Daemon ${version}` : ""}</span>
        <span className="ml-auto flex items-center gap-1" title="Commands">
          <Kbd>{navigator.userAgent.includes("Mac") ? "⌘K" : "Ctrl K"}</Kbd>
        </span>
      </div>
    </nav>
  );
}

/** A page's top row: its title, a line about it, and its actions. Empty parts
 *  of the row drag the window. */
export function PageHeader({ title, note, children }: { title: string; note?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div data-tauri-drag-region className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-x-3 gap-y-2 px-5 pt-2 pb-2">
      <h2 className="text-[17px] font-semibold tracking-[-0.01em]">{title}</h2>
      {note && <span className="text-muted-foreground">{note}</span>}
      {children && <span className="ml-auto flex flex-wrap items-center gap-1.5">{children}</span>}
    </div>
  );
}
