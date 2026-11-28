---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: []
---

# Desktop app: dashboard, logins, activity, doctor, settings, tray popover

Scope: the Tauri window and the menu-bar popover. Mode: Operate. The look extends the brand world of apps/web/DESIGN.md (the signal box) into a working tool: plain and dense where people scan, brand only in the header mark, first-run and empty states (PRODUCT.md, Brand Commitments).

- Audience: backend and DevOps engineers glancing at tunnel state many times a day.
- Task: see which tunnels run, act on one (start, stop, restart, log in), find why one failed.
- States: running, starting, reconnecting, unhealthy, failed, stopped; login expired; daemon down; config errors; empty.
- Constraints: state never by colour alone; light and dark follow the OS; production must be hard to start by mistake; macOS and Windows (Linux keeps a plain frame).

## Direction contract

THESIS: The app is the signal box's working panel: one quiet line per tunnel with its lamp, the cause and the one action. It refuses the web dashboard in a browser frame: top tabs, a colour stripe per row, a slide-over for details.

OWN-WORLD: A native translucent sidebar (macOS vibrancy, Windows Mica) over neutrals tinted from the box's green-grey; dark uses box #161d1a / panel #1e2723, light uses paper #f4f3ee / white. Primary actions are plates: cream on ink in dark, ink on paper in light. Focus and selection are amber. Green, amber and red appear only for tunnel state, always beside the state word. The pixel portal and warden appear in the sidebar mark, first-run and empty states only.

STORY: Open the window or the popover; see at once what runs and what failed. Click a tunnel to read its live log and command beside the list. Act from the row, the command palette (⌘K / Ctrl+K) or the popover. An expired login shows one card with Log in.

FIRST VIEWPORT: Left, a 220 px sidebar: the traffic lights, the portal mark with Tunwarden and the overall state, then Tunnels, Logins, Activity, Doctor, Settings with counts. Right, a toolbar (filter, group starts, Add tunnel as the primary plate), the tunnel list grouped by profile (lamp and state word, name, local address, uptime, a 24-hour state strip, actions), and, when a tunnel is selected, a detail pane on the right third with its log.

FORM: The user's own structure. seed: none (pinned) "native window frame, sidebar instead of tabs, a split view for tunnels, a menu-bar popover, a command palette, a state timeline per tunnel, brand colours instead of the library's, purposeful motion" (chosen in conversation, "0.1.0, everything"). Signature interaction: selecting a tunnel opens its live log beside the list without covering it; motion is the lamp changing aspect and the pane sliding in, nothing else.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
