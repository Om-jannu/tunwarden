---
name: Tunwarden website
description: A railway signal box for your tunnels, with a pixel warden on duty.
colors:
  box-ground: "#1c2420"
  box-floor: "#141a17"
  diagram-enamel: "#2e3a35"
  enamel-edge: "#415048"
  register-recess: "#232d29"
  rail-cream: "#e9e6dc"
  plate-cream: "#efe9d8"
  cream-dim: "#c4bfae"
  traffolyte-black: "#0f1311"
  terminal-black: "#0b0e0d"
  danger-plate: "#8f2a22"
  aspect-green: "#3fd77f"
  aspect-amber: "#ffbe3d"
  aspect-red: "#ff5a4c"
  rulebook-paper: "#ece5d3"
  rulebook-rule: "#c9c0a8"
  rulebook-ink: "#1d2420"
  rulebook-ink-dim: "#4a524c"
  lever-red: "#c8322a"
  lever-blue: "#2f63c4"
  lever-steel: "#c9ccc6"
  lede-cream: "#ddd8c8"
  terminal-text: "#d8d3c3"
  register-text: "#dcd7c7"
  register-time: "#b9a679"
  cream-hover: "#fff8e4"
  running-plate: "#12301f"
  running-plate-text: "#9ff2c0"
  changing-plate: "#3d2e10"
  changing-plate-text: "#ffd98a"
  lever-slot: "#2b3330"
  lever-black: "#050606"
  lever-black-edge: "#3a423f"
  code-on-paper: "#ddd4bd"
  menubar-dark: "#2a2d33"
typography:
  display:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "5.5rem"
    fontWeight: 800
    lineHeight: 0.9
    letterSpacing: "0.01em"
  headline:
    fontFamily: "Big Shoulders, Arial Narrow, sans-serif"
    fontSize: "3.75rem"
    fontWeight: 800
    lineHeight: 0.95
    letterSpacing: "0.01em"
  plate:
    fontFamily: "Sofia Sans Condensed, Arial Narrow, sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 700
    lineHeight: 1.2
    letterSpacing: "0.1em"
  body:
    fontFamily: "Sofia Sans, system-ui, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.55
  code:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: 1.65
rounded:
  plate: "2px"
  control: "3px"
  terminal: "4px"
  panel: "6px"
spacing:
  gutter: "clamp(1.25rem, 4vw, 3.5rem)"
  section: "clamp(4.5rem, 9vw, 7.5rem)"
components:
  plate:
    backgroundColor: "{colors.traffolyte-black}"
    textColor: "{colors.plate-cream}"
    typography: "{typography.plate}"
    rounded: "{rounded.plate}"
    padding: "0.28em 0.65em 0.24em"
  plate-danger:
    backgroundColor: "{colors.danger-plate}"
    textColor: "#fff3ee"
    typography: "{typography.plate}"
    rounded: "{rounded.plate}"
  button-plate:
    backgroundColor: "{colors.plate-cream}"
    textColor: "{colors.box-floor}"
    rounded: "{rounded.control}"
    padding: "1rem 1.5rem 0.9rem"
  button-plate-hover:
    backgroundColor: "#fff8e4"
  command:
    backgroundColor: "{colors.terminal-black}"
    textColor: "{colors.plate-cream}"
    typography: "{typography.code}"
    rounded: "{rounded.control}"
  terminal:
    backgroundColor: "{colors.terminal-black}"
    textColor: "#d8d3c3"
    typography: "{typography.code}"
    rounded: "{rounded.terminal}"
    padding: "1.1rem 1.3rem"
  nav-link:
    textColor: "{colors.cream-dim}"
    typography: "{typography.plate}"
    rounded: "{rounded.plate}"
    padding: "0.55rem 0.8rem 0.5rem"
  nav-link-current:
    backgroundColor: "{colors.traffolyte-black}"
    textColor: "{colors.plate-cream}"
---

# Design System: Tunwarden website

## Overview

**Creative North Star: "The Signal Box"**

The site is the inside of a railway signal box at night. The page ground is the box itself, dark green-grey; the main surfaces are an enamelled track diagram where each tunnel is a lit section running into the pixel portal from the app icon, and a lever frame below it that works the demo. Names and states are engraved black plates with cream capitals. The documentation is the box's rule book, printed on cream paper and ruled in ink.

The pixel art comes from the brand drawings (`docs/brand/concepts/make.py`) through `scripts/art.py`, so the portal, lamp, tray glyph and warden on the site are the same grids the app ships. The warden is friendly and stands by the portals; he speaks in a plate, never in a cartoon bubble.

**Key Characteristics:**
- One dark box, one lit diagram, one lever frame; everything else stays in the box's materials.
- State lives in three lamp aspects (green, amber, red), always paired with words.
- Real product output (CLI transcripts, config) is the proof, shown as terminal text.
- Motion is mechanical: a lever throw and a lamp changing aspect.

## Colors

A full-palette world: the dark box neutrals carry most of the page, cream carries type and rails, and the three aspect colours are reserved for tunnel state.

### Primary
- **Plate Cream** (plate-cream): headlines, body on dark, the primary action's face, and the rails.
- **Diagram Enamel** (diagram-enamel): the lit diagram panel and the download desk; the brightest dark surface on the page.

### Secondary
- **Traffolyte Black** (traffolyte-black): every engraved plate: tunnel names, nav, lever badges, the warden's speech.
- **Danger Plate** (danger-plate): production markers and failed-state plates only.

### Tertiary
- **Aspect Green, Amber, Red** (aspect-green, aspect-amber, aspect-red): lamp cores, lit rails and state words in terminals. Amber also marks focus rings and selection.

### State plates
- **Running plate** (running-plate, running-plate-text) and **Changing plate** (changing-plate, changing-plate-text) tint state plates; failed uses the danger plate.

### Neutral
- **Box Ground** (box-ground): the page. **Box Floor** (box-floor): nameboard and lever frame. **Register Recess** (register-recess): the sunk strip that holds the register. **Rule-book Paper** and **Ink** for the docs and the Box rules sheet.

### Named Rules
**The Aspect Rule.** Green, amber and red mean running, changing and failed. They never decorate content, and each one always sits next to the word it means. Amber doubles as the focus ring and text selection, where it marks the one thing you are acting on.

**The Danger Plate Rule.** Red plates mark production or failure. A red plate that marks neither is a lie.

## Typography

**Display Font:** Big Shoulders (800), condensed industrial capitals for headlines.
**Plate Font:** Sofia Sans Condensed (700), tracked capitals, the engraved-plate voice for labels, nav, levers and buttons.
**Body Font:** Sofia Sans (400 and 600) for sentences.
**Code Font:** JetBrains Mono, for commands, ports, transcripts and the register only.

### Hierarchy
- **Display** (800, clamp(3.5rem, 6.2vw, 5.5rem), 0.9): the hero headline and page titles.
- **Headline** (800, clamp(2.25rem, 4.4vw, 3.75rem), 0.95): section headings, uppercase, balanced.
- **Plate** (700, 0.8125rem to 1.0625rem, tracked 0.08em to 0.16em, uppercase): every label.
- **Body** (400, 1.0625rem, 1.55; ledes 1.125rem to 1.25rem): prose, max 62ch.
- **Code** (400, 0.75rem to 0.8125rem, 1.55 to 1.65): ligatures off, tabular numerals in tables.

### Named Rules
**The Engraving Rule.** Anything that names a thing is set in the plate face on a plate. Sentences are never set in capitals.

**The Real Output Rule.** Monospace appears only where the product's own text appears: commands, config, logs, ports.

## Layout

The page runs in a single centred column (max 80rem) with a fluid gutter (clamp(1.25rem, 4vw, 3.5rem)) and large section spacing (clamp(4.5rem, 9vw, 7.5rem)). The hero is one object: the diagram panel (copy and register on the left, tracks on the right) joined to the lever frame below it, which holds the levers and the primary action. Below 1100px the panel stacks copy, diagram, register; below 640px the install action moves under the copy so it stays in the first viewport, track labels sit above their rails and portals drop to 3× scale. Sections alternate dense (terminals) and quiet (the rule book sheet) passages.

**The Whole-Pixel Rule.** Pixel art renders only at whole multiples of its grid (4× on desktop, 3× on phones), never at fractional sizes.

## Elevation & Depth

Flat and tonal. Depth comes from the box's materials: the enamel panel is lighter than the box, the register is a recess, the lever frame is the floor. The diagram panel carries a 2px inset edge (enamel-edge). There are no drop shadows.

**The No-Glow Rule.** Lamps glow only through their pixel halo ring drawn on the grid; no CSS glows or blurs.

## Shapes

Small, machined corners: plates 2px, controls 3px, terminals 4px, the panel 6px. Levers are straight bars with a steel catch handle and a floor slot. No pills.

## Components

### Buttons
- **Plate button:** the primary action. Cream face, floor-coloured plate capitals at 1.375rem, 3px corners; hover lightens the cream. One per view region.
- **Copy button:** sits flush at the end of a command strip, plate capitals on a slightly raised dark, "Copy" then "Copied".

### Chips (plates)
- **Plate:** black traffolyte, cream capitals, 2px corners. State plates tint by aspect; the production plate is danger red.

### Navigation
- **Nameboard:** the floor-coloured strip with the pixel icon and the name in tracked plate capitals; nav items are plate capitals in cream-dim, the current page sits on a black plate.

### Lever
- **Lever:** a button whose arm throws from −14° to 14° around its base (520ms, ease cubic-bezier(0.16, 1, 0.3, 1)). Black, red and blue arms like a real frame. A locked lever is desaturated and its badge says why it is locked (interlocking); an unlocked one says "Pull" in amber.

### Track
- **Track:** a name plate, the address and tool in code, a state plate, a cream rail that turns amber or red with the state, and the pixel portal whose lamp core takes the aspect colour.

### Terminal
- **Terminal:** near-black block of real CLI output, with state words coloured by aspect.

## Do's and Don'ts

### Do:
- **Do** pair every aspect colour with its word.
- **Do** show real CLI output and label demo data as a demo.
- **Do** draw new pixel art in `docs/brand/concepts/make.py` and regenerate with `scripts/art.py`.
- **Do** keep levers interlocked: a lever that cannot act says why.

### Don't:
- **Don't** add testimonials, user counts or benchmarks; there are none (PRODUCT.md, Evidence on Hand).
- **Don't** scale pixel art by fractions or smooth it.
- **Don't** use the aspect colours for decoration, links or headings.
- **Don't** use monospace for anything that is not the product's own text.
