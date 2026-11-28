---
name: Tunwarden desktop app
description: The signal box as a working panel. Quiet when healthy, clear when not.
colors:
  paper: "#f6f5f1"
  ink: "#1d2420"
  card: "#ffffff"
  muted-paper: "#efeee8"
  ink-dim: "#5b635d"
  rule: "#e1dfd7"
  sidebar-paper: "#ecebe4"
  box: "#171e1b"
  box-card: "#1c2420"
  box-sidebar: "#131916"
  plate-cream: "#efe9d8"
  cream-dim: "#939d96"
  box-rule: "#29332e"
  focus-amber: "#c7851a"
  focus-amber-dark: "#ffbe3d"
  running: "#1e9e5a"
  running-dark: "#3cc27a"
  busy: "#b9770f"
  busy-dark: "#e3a43b"
  failed: "#d5483e"
  failed-dark: "#f0645a"
  stopped: "#8b938d"
  stopped-dark: "#6f7a73"
  production: "#8b45d2"
  production-dark: "#b07bf0"
typography:
  page-title:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "17px"
    fontWeight: 600
    letterSpacing: "-0.01em"
  first-run-title:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "20px"
    fontWeight: 600
  pane-title:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "15px"
    fontWeight: 600
  mark:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "14px"
    fontWeight: 600
  body:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
  group-label:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 600
    letterSpacing: "0.02em"
  small:
    fontFamily: "system-ui, -apple-system, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "11px"
    fontWeight: 400
  code:
    fontFamily: "ui-monospace, SF Mono, Cascadia Mono, JetBrains Mono, Menlo, monospace"
    fontSize: "12px"
    fontWeight: 400
rounded:
  sm: "5px"
  md: "6.5px"
  lg: "8px"
spacing:
  page-x: "20px"
  sidebar-width: "212px"
  row-height: "40px"
components:
  button-primary:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.paper}"
    rounded: "{rounded.md}"
    height: "28px"
  button-primary-dark:
    backgroundColor: "{colors.plate-cream}"
    textColor: "{colors.box}"
    rounded: "{rounded.md}"
  sidebar-item:
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    height: "32px"
  state-label:
    typography: "{typography.body}"
  address:
    typography: "{typography.code}"
---

# Design System: Tunwarden desktop app

## Overview

**Creative North Star: "The Working Panel"**

The website (apps/web/DESIGN.md) is the signal box seen from outside; the app is the panel the warden actually works at. It keeps the box's green-grey and cream, its plate-like primary buttons and its three lamp aspects, and drops everything decorative: no textures, no display type, no pixel art on working screens. It is glanced at many times a day, so a healthy screen is calm and a failed tunnel is the only loud thing on it.

The window is native where the OS offers it: a translucent sidebar with the traffic lights on macOS, Mica on Windows, a plain frame on Linux. A menu-bar popover (an NSPanel on macOS) covers most daily use; the window is for detail.

**Key Characteristics:**
- Sidebar with counts instead of tabs; one title row per page.
- Tunnels are dense rows grouped by profile; selecting one opens its log beside the list, never over it.
- Green, amber and red mean tunnel state and nothing else, always beside the state word.
- The pixel portal and warden appear only in the sidebar mark, the popover, first-run, empty states and the login-expired card.

## Colors

Restrained: neutrals tinted from the signal box's green-grey, one ink or cream for primary actions, and state colours.

### Primary
- **Ink** (ink) on light, **Plate Cream** (plate-cream) on dark: primary buttons and headings. Primary buttons are plates, never blue.

### Neutral
- **Paper** (paper) and **Box** (box): the content ground. **Card** (card, box-card): the detail pane, cards and the login notice. **Sidebar** (sidebar-paper, box-sidebar): replaced by the native material when the frame is native.

### State
- **Running, Busy, Failed, Stopped** and their dark variants: state words, dots, lamp, the 24-hour strip, failure text.
- **Production** (production): the production badge only.

### Named Rules
**The Aspect Rule.** Green, amber and red mean running, connecting and failed. They never decorate, and each one sits next to the word it means.

**The Amber Focus Rule.** Focus rings and text selection are amber, the one colour that marks what you are acting on.

## Typography

**UI face:** the system stack (SF Pro, Segoe UI Variable), at 13px, the density of a native tool.
**Code face:** the system monospace, for addresses, commands, environment names and logs only.

### Hierarchy
- **First-run title** (600, 20px): the setup screen only.
- **Page title** (600, 17px, -0.01em): one per page, in the title row.
- **Pane title** (600, 15px): the selected tunnel's name in the detail pane.
- **Mark** (600, 14px): "Tunwarden" in the sidebar, and the palette's input.
- **Body** (400, 13px): everything else.
- **Group label** (600, 12px, uppercase, 0.02em, dim): profile groups, "Profiles", "Last 24 hours".
- **Small** (400, 11px): the popover's addresses and group labels, the strip's time axis.
- **Code** (400, 11.5–12px): ports, commands, logs; digits are tabular in tables.

### Named Rules
**The No Display Type Rule.** Working screens use the system face only; the brand's display and plate faces stay on the website.

## Layout

A 212px sidebar and the content column; the window never scrolls, each region scrolls inside itself. Pages have a title row (title, a dim note, actions on the right) that also drags the window, then 20px side margins. The tunnel list shows Name, State, Local, Uptime, Last 24 h, Restarts and Detail; with a tunnel selected, the detail pane takes the right 42% (340–600px) and the list keeps Name, State, Local, Uptime and actions. Minimum window 760×480.

**The Beside, Not Over Rule.** Details open next to what they describe. Dialogs are for decisions (production start, remove, quit) and forms only.

## Elevation & Depth

Flat and tonal. Depth comes from the sidebar's material, card surfaces one step lighter than the ground and 1px rules. Shadows belong only to floating things: dialogs, the palette, the popover.

## Shapes

Gently rounded: 5–8px on controls and cards, full round on state dots and the 24-hour strip. No side stripes on rows or cards.

## Components

### Buttons
- **Primary:** ink (light) or cream (dark) plate, 28px high, 6.5px corners; one per region (Add tunnel, Log in, Add login).
- **Outline and ghost:** everything else. Icon-only buttons carry an aria-label and a title.

### Navigation
- **Sidebar item:** 32px, icon and label, current page on a tinted fill; counts on the right (running/total, "n expired" in red).

### Tunnel row
- **Row:** name, state word with dot, copyable address, uptime, 24-hour strip, restarts, reason, start/stop and restart. Selected rows take the accent fill. j/k move, space starts or stops.

### 24-hour strip
- **Strip:** one bar per tunnel; running, connecting, failed and stopped stretches, blank where the log has no record; labelled in words for screen readers and on hover.

### Login card
- **Card:** the red-lantern warden, the login's name, what stopped, In Terminal, and Log in as the primary.

### Command palette
- **Palette:** ⌘K or Ctrl+K; every action, filtered by all words typed; Enter runs, Esc closes.

### Popover
- **Popover:** 340×460, the mark and summary, an expired-login line, every tunnel with state and a switch, Open Tunwarden.

## Do's and Don'ts

### Do:
- **Do** pair every state colour with its word.
- **Do** keep the window still: scroll inside regions.
- **Do** open details beside the list.
- **Do** keep production starts behind a confirmation everywhere (rows, palette, popover, tray menu).

### Don't:
- **Don't** use blue or any accent for primary buttons; they are ink or cream plates.
- **Don't** put coloured stripes down a row or card edge.
- **Don't** put pixel art on working screens beyond the sidebar mark.
- **Don't** repeat the login-expired card outside the Tunnels page; the sidebar count and the Logins table carry it there.
