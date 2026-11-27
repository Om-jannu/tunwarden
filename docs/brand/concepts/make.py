#!/usr/bin/env python3
"""Draws the Tunwarden logo concepts as pixel-exact SVGs and a review board.

    python3 docs/brand/concepts/make.py

Writes one SVG per mark into this folder and `board.html`, which shows each
mark as an app icon (128, 64, 32 px) and as a 16 px tray glyph in light and
dark menu bars, with the status colours.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))

STATE = {"running": "#3cc27a", "connecting": "#e3a43b", "failed": "#f0645a", "idle": "#8c96a5"}
# The app icon's lamp: a warm neutral light, so a static icon never claims a state.
BRAND_LAMP = ("#ffe8a3", "#6b5f3a")


# --- Pixel painting --------------------------------------------------------------

class Grid:
    def __init__(self, n):
        self.n = n
        self.px = {}

    def set(self, x, y, color):
        if 0 <= x < self.n and 0 <= y < self.n and color:
            self.px[(x, y)] = color

    def sprite(self, x0, y0, rows, palette):
        for dy, row in enumerate(rows):
            for dx, ch in enumerate(row):
                if ch != ".":
                    self.set(x0 + dx, y0 + dy, palette[ch])

    def svg(self, size):
        rects = "".join(
            f'<rect x="{x}" y="{y}" width="1" height="1" fill="{c}"/>'
            for (x, y), c in sorted(self.px.items())
        )
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {self.n} {self.n}" shape-rendering="crispEdges">{rects}</svg>'
        )


def rounded_bg(g, color, radius):
    n = g.n
    for y in range(n):
        for x in range(n):
            cx = min(x, n - 1 - x)
            cy = min(y, n - 1 - y)
            if cx < radius and cy < radius:
                if math.hypot(radius - cx - 0.5, radius - cy - 0.5) > radius:
                    continue
            g.set(x, y, color)


def portal(x, y, cx, top, half, ground):
    """Inside a tunnel mouth: a semicircle of radius `half` on top of a
    rectangle down to `ground`."""
    px, py = x + 0.5, y + 0.5
    if py > ground:
        return False
    cy = top + half
    if py >= cy:
        return abs(px - cx) < half
    return math.hypot(px - cx, py - cy) < half


def tunnel(g, cx, top, outer, inner, ground, stone, interior, depth=True):
    """Stone portal with brick courses and a dark interior that recedes."""
    light, dark, mortar = stone
    for y in range(g.n):
        for x in range(g.n):
            if not portal(x, y, cx, top, outer, ground):
                continue
            if portal(x, y, cx, top + (outer - inner), inner, ground):
                if depth:
                    d = math.hypot(x + 0.5 - cx, y + 0.5 - (top + outer + 3))
                    c = interior[0] if d < inner * 0.55 else interior[1]
                else:
                    c = interior[0]
                g.set(x, y, c)
                continue
            # Brick courses: horizontal rows offset every other row.
            course = y // 2
            brick = (x + (course % 2) * 2) // 4
            if y % 2 == 1 and (x + (course % 2) * 2) % 4 == 0:
                c = mortar
            elif (course + brick) % 3 == 0:
                c = dark
            else:
                c = light
            g.set(x, y, c)


# --- Concept A: the warden at the tunnel mouth ---------------------------------

FRIENDLY = [
    "...KKKKKK...",
    "..KhhHHHHK..",
    ".KhHHHHHHHK.",
    ".KHHHHHHHHK.",
    "KKKKKKKKKKKK",
    ".KVVVVVVVVK.",
    ".KVEEVVEEVK.",
    ".KVEEVVEEVK.",
    "..KVVVVVVK..",
    ".KKBBBBBBKK.",
    "KBBBBYYBBBBK",
    "KBbBBBBBBbBK",
    "KBbBBBBBBbBK",
    ".KBBBBBBBBK.",
    ".KDDK..KDDK.",
]

SERIOUS = [
    ".....KKKK......",
    "....KCCCCK.....",
    "...KCCCCCCK....",
    "...KCFFFFCK....",
    "...KCGGGGCK....",
    "...KCFFFFCK....",
    "..KCCCCCCCCK...",
    ".KCCcCCCCcCCK..",
    ".KCCcCCCCcCCKMK",
    ".KCCcCCCCcCKMGM",
    ".KCCcCCCCcCKMGM",
    ".KCCcCCCCcCCKMK",
    ".KCCcCCCCcCCK..",
    ".KCCcCCCCcCCK..",
    "..KCCCCCCCCK...",
    "..KKK....KKK...",
]


def concept_a(feel):
    g = Grid(32)
    if feel == "friendly":
        rounded_bg(g, "#1d3a6e", 7)
        tunnel(g, 19, 5, 11, 7, 27, ("#a3acbd", "#7c869a", "#5d667a"), ("#0b0f15", "#141b26"))
        for x in range(32):
            for y in (27, 28):
                if (x, y) in g.px:
                    g.set(x, y, "#2b4f8f")
        pal = dict(K="#0b0f15", h="#ff8f9c", H="#ef5b6d", V="#0b0f15", E="#f4f7ff",
                   B="#3d7bea", b="#2a5bb8", Y="#ffe08a", D="#18202c")
        g.sprite(3, 13, FRIENDLY, pal)
    else:
        rounded_bg(g, "#1a222d", 7)
        tunnel(g, 19, 5, 11, 7, 27, ("#4d5869", "#3a4453", "#2c3440"), ("#07090c", "#0d1218"))
        # Warm light from the lantern on the portal stones behind the keeper.
        for (x, y) in [(15, 20), (15, 21), (16, 21), (15, 22)]:
            if (x, y) in g.px:
                g.set(x, y, "#7a6d4c")
        # Lantern light pooled on the ground in front of the tunnel.
        for x, c in [(14, "#3a3526"), (15, "#4a4330"), (16, "#4a4330"), (17, "#3a3526")]:
            g.set(x, 27, c)
        pal = dict(K="#07090c", C="#5a6a80", c="#3e4b5e", F="#07090c", G="#ffe8a3",
                   M="#c3cad6")
        g.sprite(2, 12, SERIOUS, pal)
    return g


def tray_a(ink):
    """16 px: the arch with the warden's helmet in it."""
    g = Grid(16)
    for y in range(16):
        for x in range(16):
            outer = portal(x, y, 8, 1, 7, 15)
            inner = portal(x, y, 8, 3, 5, 15)
            if outer and not inner:
                g.set(x, y, ink)
    head = [
        "..XXXX..",
        ".XXXXXX.",
        "XXXXXXXX",
        ".X.XX.X.",
        ".XXXXXX.",
        "..XXXX..",
        ".XXXXXX.",
    ]
    g.sprite(4, 8, head, {"X": ink})
    return g


# --- Concept B: tunnel portal with a signal lamp --------------------------------

def concept_b(state):
    g = Grid(32)
    rounded_bg(g, "#12171e", 7)
    tunnel(g, 16, 4, 13, 9, 28, ("#56627a", "#434e63", "#323b4c"), ("#05070a", "#0b0f15"))
    if state == "brand":
        lamp, halo = BRAND_LAMP
    else:
        lamp = STATE[state]
        # The light at the end of the tunnel: a lamp with a halo ring.
        halo = {"running": "#1f5b3f", "connecting": "#6b4f1f", "failed": "#6e2f2b", "idle": "#2f3640"}[state]
    for dy in range(-3, 4):
        for dx in range(-3, 4):
            d = math.hypot(dx, dy)
            if d <= 1.6:
                g.set(16 + dx, 18 + dy, lamp)
            elif d <= 3.1:
                g.set(16 + dx, 18 + dy, halo)
    g.set(15, 17, "#ffffff")
    # A track running into the tunnel: two rails and sleepers, wider as it nears.
    for y in range(23, 29):
        spread = (y - 23) // 2
        left, right = 13 - spread, 18 + spread
        if y % 2 == 0:
            for x in range(left, right + 1):
                g.set(x, y, "#3b3328")
        g.set(left, y, "#9aa3b5")
        g.set(right, y, "#9aa3b5")
    return g


def tray_portal(ink, dot):
    """16 px: an arch outline and the lamp inside, which carries the state."""
    g = Grid(16)
    for y in range(16):
        for x in range(16):
            outer = portal(x, y, 8, 1, 7, 15)
            inner = portal(x, y, 8, 3, 5, 15)
            if outer and not inner:
                g.set(x, y, ink)
    for dy in range(-2, 3):
        for dx in range(-2, 3):
            if dx * dx + dy * dy <= 5:
                g.set(8 + dx, 10 + dy, dot)
    return g


# --- Concept C: metro-map monogram (geometric, not pixel) ------------------------

def concept_c(size, state="running"):
    lamp = STATE[state]
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 128 128">
  <rect width="128" height="128" rx="28" fill="#2350b8"/>
  <path d="M30 104 V60 a34 34 0 0 1 68 0 V104" fill="none" stroke="#f4f6f9" stroke-width="12" stroke-linecap="round"/>
  <path d="M64 120 V72" fill="none" stroke="#ffd45c" stroke-width="10" stroke-linecap="round"/>
  <circle cx="64" cy="64" r="12" fill="#2350b8" stroke="#f4f6f9" stroke-width="6"/>
  <circle cx="64" cy="64" r="5.5" fill="{lamp}"/>
</svg>'''


def tray_c(ink, dot):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  <path d="M3 15 V8 a5 5 0 0 1 10 0 V15" fill="none" stroke="{ink}" stroke-width="2"/>
  <circle cx="8" cy="9" r="2.4" fill="{dot}"/>
</svg>'''


# --- Board -------------------------------------------------------------------------

def png(g, scale, canvas, path):
    """Writes `g` as a PNG: each grid pixel becomes `scale`×`scale` pixels,
    centred on a transparent `canvas`×`canvas` image."""
    size = g.n * scale
    off = (canvas - size) // 2
    rows = []
    for y in range(canvas):
        row = bytearray([0])
        for x in range(canvas):
            gx, gy = (x - off) // scale, (y - off) // scale
            c = g.px.get((gx, gy)) if 0 <= x - off < size and 0 <= y - off < size else None
            if c:
                row += bytes(int(c[i:i + 2], 16) for i in (1, 3, 5)) + b"\xff"
            else:
                row += b"\x00\x00\x00\x00"
        rows.append(bytes(row))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", canvas, canvas, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(data)


def write(name, text):
    with open(os.path.join(HERE, name), "w", encoding="utf-8") as f:
        f.write(text)


def bar(svgs, dark):
    bg, fg = ("#2a2d33", "#e6e9ee") if dark else ("#e9ebee", "#1d2128")
    cells = "".join(f'<span class="glyph">{s}</span>' for s in svgs)
    return f'<div class="bar" style="background:{bg};color:{fg}">{cells}<span class="clock">9:41</span></div>'


def zoom(svg):
    return svg.replace('width="16" height="16"', 'width="64" height="64"')


def section(title, line, feel, icons, trays_light, trays_dark, notes):
    icon_cells = "".join(
        f'<figure><div class="icons">{i(128)}{i(64)}{i(32)}</div><figcaption>{cap}</figcaption></figure>'
        for i, cap in icons
    )
    return f'''
<section>
  <header><h2>{title}</h2><p>{line}</p><span class="feel">{feel}</span></header>
  <div class="row">{icon_cells}</div>
  <div class="row trays">
    <figure><div>{bar(trays_light, False)}{bar(trays_dark, True)}</div><figcaption>Tray at 16 px, light and dark menu bar</figcaption></figure>
    <figure><div class="zoomed">{"".join(zoom(t) for t in trays_light[:2])}</div><figcaption>Tray glyph at 4× for detail</figcaption></figure>
  </div>
  <ul class="notes">{"".join(f"<li>{n}</li>" for n in notes)}</ul>
</section>'''


def main():
    a_friendly = concept_a("friendly")
    a_serious = concept_a("serious")
    write("a-warden-friendly.svg", a_friendly.svg(128))
    write("a-warden-serious.svg", a_serious.svg(128))
    b = {s: concept_b(s) for s in STATE}
    for s, g in b.items():
        write(f"b-signal-{s}.svg", g.svg(128))
    write("c-route.svg", concept_c(128))
    icon = concept_b("brand")
    write("../icon.svg", icon.svg(1024))
    # Apple's icon grid puts the rounded square at about 824 of 1024 px; 26 px
    # per grid pixel gives 832, close enough and pixel-exact.
    png(icon, 26, 1024, os.path.join(HERE, "..", "icon-1024.png"))

    light_ink, dark_ink = "#1d2128", "#e6e9ee"
    sections = [
        section(
            "A · Warden at the tunnel mouth",
            "Your idea: a pixel guard standing at a stone tunnel portal.",
            "friendly and serious",
            [(lambda s: a_friendly.svg(s), "Friendly: a round-helmet guard with a visor and kind eyes, like a game sprite"),
             (lambda s: a_serious.svg(s), "Serious: a hooded keeper with a lit visor slit and a lantern")],
            [tray_a(light_ink).svg(16), tray_a(light_ink).svg(16)],
            [tray_a(dark_ink).svg(16), tray_a(dark_ink).svg(16)],
            ["Most character of the three; strongest for the website, installer art and empty states.",
             "At 16 px the warden shrinks to a helmet in an arch; it reads, but the tray cannot show state in the mark itself, so the state needs a separate dot.",
             "The figure needs both variants drawn well at every size; more art to maintain."],
        ),
        section(
            "B · Signal at the end of the tunnel",
            "A tunnel portal with a railway signal lamp inside. The lamp is the status: green running, amber connecting, red failed, grey idle.",
            "calm and trustworthy, pixel-built",
            [(lambda s, k=k: b[k].svg(s), cap) for k, cap in
             [("running", "Running"), ("failed", "Failed")]],
            [tray_portal(light_ink, STATE[s]).svg(16) for s in STATE],
            [tray_portal(dark_ink, STATE[s]).svg(16) for s in STATE],
            ["The logo and the status indicator are the same thing, so the tray icon needs no extra dot.",
             "Keeps your pixel theme; a warden character can still appear in illustrations, guarding the portal, without being the icon.",
             "macOS menu bar icons are usually single-colour template images; a coloured lamp breaks that convention (Docker and Tailscale do the same)."],
        ),
        section(
            "C · Route into the tunnel",
            "A geometric monogram from transit maps: a route line runs into an arch and ends at a station, which lights in the status colour.",
            "neutral, clean",
            [(lambda s: concept_c(s), "App icon"), (lambda s: concept_c(s, "failed"), "Failed state")],
            [tray_c(light_ink, STATE[s]) for s in STATE],
            [tray_c(dark_ink, STATE[s]) for s in STATE],
            ["Sharpest at small sizes and easiest to reproduce anywhere (favicon, README badge, CLI banner).",
             "Leaves the pixel idea behind; the warden would live only in copy and illustrations.",
             "Closest to what other developer tools look like, so the least memorable."],
        ),
    ]
    board = f'''<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Tunwarden logo concepts</title>
<style>
  body {{ margin:0; padding:28px 32px; background:#f4f6f9; color:#17202b; font:13px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif; }}
  h1 {{ font-size:18px; margin:0 0 4px; }} .lede {{ color:#5a6576; margin:0 0 20px; max-width:72ch; }}
  section {{ background:#fff; border:1px solid #dce1e8; border-radius:10px; padding:16px 18px; margin-bottom:16px; }}
  section header {{ display:flex; align-items:baseline; gap:10px; flex-wrap:wrap; }}
  h2 {{ font-size:15px; margin:0; }} section header p {{ margin:0; color:#5a6576; }}
  .feel {{ margin-left:auto; font-size:12px; color:#5a6576; border:1px solid #dce1e8; border-radius:999px; padding:1px 8px; }}
  .row {{ display:flex; gap:28px; margin-top:12px; flex-wrap:wrap; }}
  figure {{ margin:0; }} figcaption {{ color:#5a6576; font-size:12px; margin-top:6px; max-width:44ch; }}
  .icons {{ display:flex; align-items:flex-end; gap:14px; }} .icons svg {{ image-rendering:pixelated; }}
  .bar {{ display:flex; align-items:center; gap:14px; height:26px; padding:0 12px; border-radius:6px; margin-bottom:6px; width:300px; }}
  .glyph svg {{ display:block; }} .clock {{ margin-left:auto; font-size:12px; }}
  .zoomed {{ display:flex; gap:10px; }} .zoomed svg {{ image-rendering:pixelated; background:#e9ebee; border-radius:6px; }}
  .notes {{ margin:12px 0 0; padding-left:18px; color:#323b4c; }} .notes li {{ margin:2px 0; }}
</style></head><body>
<h1>Tunwarden logo concepts</h1>
<p class="lede">Three directions, drawn at the sizes they ship at. Pixel marks are exact grids (32 px for the icon, 16 px for the tray) scaled by whole numbers, so they stay crisp. Colours are working values, not final.</p>
{"".join(sections)}
</body></html>'''
    write("board.html", board)
    print("wrote", sorted(f for f in os.listdir(HERE) if f.endswith((".svg", ".html"))))


if __name__ == "__main__":
    main()
