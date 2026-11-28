import { useEffect, useMemo, useRef, useState } from "react";
import { SearchIcon } from "lucide-react";
import { api, selector } from "@/api";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { isActive } from "@/logic";
import { one, useStore, type View } from "@/store";

type Command = { id: string; label: string; group: string; hint?: string; run: () => void };

const PAGES: [View, string][] = [
  ["tunnels", "Tunnels"],
  ["logins", "Logins"],
  ["activity", "Activity"],
  ["doctor", "Doctor"],
  ["settings", "Settings"],
];

/** ⌘K (Ctrl+K elsewhere): every action in one list, filtered as you type.
 *  ↑ ↓ choose, Enter runs, Esc closes. */
export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const tunnels = useStore((s) => s.tunnels);
  const logins = useStore((s) => s.logins);
  const expired = useStore((s) => s.expired);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (open) {
      setQuery("");
      setIndex(0);
      void useStore.getState().loadLogins();
    }
  }, [open]);

  const commands = useMemo<Command[]>(() => {
    const s = useStore.getState();
    const go = (v: View) => s.setView(v);
    const out: Command[] = [];
    const expiredNames = new Set(expired.map((l) => l.name));
    for (const l of [...logins].sort((a, b) => Number(expiredNames.has(b.name)) - Number(expiredNames.has(a.name)))) {
      out.push({
        id: `login:${l.name}`,
        label: `Log in to ${l.name}`,
        group: "Logins",
        hint: expiredNames.has(l.name) ? "expired" : undefined,
        run: () => {
          go("logins");
          useStore.setState({ loggingIn: l.name });
        },
      });
    }
    for (const t of tunnels ?? []) {
      const on = isActive(t);
      out.push({ id: `toggle:${t.name}`, label: `${on ? "Stop" : "Start"} ${t.name}`, group: "Tunnels", hint: t.state, run: () => void s.act(on ? "stop" : "start", one(t.name)) });
      out.push({ id: `restart:${t.name}`, label: `Restart ${t.name}`, group: "Tunnels", run: () => void s.act("restart", one(t.name)) });
      out.push({
        id: `show:${t.name}`,
        label: `Show ${t.name}`,
        group: "Tunnels",
        hint: "details and log",
        run: () => {
          go("tunnels");
          s.setOpen(t.name);
        },
      });
    }
    for (const g of [...new Set((tunnels ?? []).flatMap((t) => t.groups))].sort()) {
      out.push({ id: `group-start:${g}`, label: `Start group ${g}`, group: "Tunnels", run: () => void s.act("start", selector({ group: g })) });
      out.push({ id: `group-stop:${g}`, label: `Stop group ${g}`, group: "Tunnels", run: () => void s.act("stop", selector({ group: g })) });
    }
    out.push({ id: "add", label: "Add tunnel", group: "Config", run: () => s.edit({ table: "tunnels", name: null }) });
    out.push({ id: "import", label: "Import tunnels", group: "Config", run: () => useStore.setState({ phase: "first-run" }) });
    out.push({ id: "edit-config", label: "Edit config file", group: "Config", run: () => void api().then((a) => a.openConfig()) });
    for (const [v, label] of PAGES) out.push({ id: `go:${v}`, label: `Go to ${label}`, group: "Pages", run: () => go(v) });
    return out;
  }, [tunnels, logins, expired]);

  const shown = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return commands;
    return commands
      .filter((c) => words.every((w) => `${c.label} ${c.group} ${c.hint ?? ""}`.toLowerCase().includes(w)))
      .sort((a, b) => {
        const first = words[0] ?? "";
        return Number(!a.label.toLowerCase().startsWith(first)) - Number(!b.label.toLowerCase().startsWith(first));
      });
  }, [commands, query]);

  const current = Math.min(index, Math.max(0, shown.length - 1));
  useEffect(() => {
    list.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [current]);

  const run = (c: Command | undefined) => {
    if (!c) return;
    setOpen(false);
    c.run();
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent showCloseButton={false} className="top-[18%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-[560px]">
        <DialogTitle className="sr-only">Commands</DialogTitle>
        <div className="flex items-center gap-2 border-b px-3">
          <SearchIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <input
            autoFocus
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setIndex(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(i + 1, shown.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(i - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                run(shown[current]);
              }
            }}
            placeholder="Start, stop, log in, go to…"
            aria-label="Search commands"
            role="combobox"
            aria-expanded
            aria-controls="palette-list"
            aria-activedescendant={shown[current] ? `cmd-${current}` : undefined}
            className="h-11 flex-1 bg-transparent text-[14px] outline-none placeholder:text-muted-foreground"
          />
          <Kbd>esc</Kbd>
        </div>
        <ul ref={list} id="palette-list" role="listbox" aria-label="Commands" className="max-h-[340px] overflow-auto p-1.5">
          {shown.length === 0 && <li className="px-2.5 py-6 text-center text-muted-foreground">No command matches “{query}”.</li>}
          {shown.map((c, i) => (
            <li
              key={c.id}
              id={`cmd-${i}`}
              data-index={i}
              role="option"
              aria-selected={i === current}
              onMouseMove={() => setIndex(i)}
              onClick={() => run(c)}
              className={cn("flex h-8 cursor-default items-center gap-3 rounded-md px-2.5", i === current && "bg-accent")}
            >
              <span className="truncate">{c.label}</span>
              {c.hint && <span className={cn("text-[12px]", c.hint === "expired" ? "text-failed" : "text-muted-foreground")}>{c.hint}</span>}
              <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">{c.group}</span>
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
