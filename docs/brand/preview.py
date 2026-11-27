#!/usr/bin/env python3
"""Builds docs/brand/preview.html from the shipped icons.

    TUNWARDEN_ICON_PREVIEW=/tmp/tw-tray cargo test -p tunwarden-desktop preview
    python3 docs/brand/preview.py /tmp/tw-tray

The tray images come from the app's own drawing code (raw RGBA written by
that test); the app icons are the files Tauri bundles.
"""

import base64
import os
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ICONS = os.path.join(HERE, "..", "..", "apps", "desktop", "src-tauri", "icons")


def png_from_rgba(raw, w, h):
    rows = b"".join(b"\x00" + raw[y * w * 4:(y + 1) * w * 4] for y in range(h))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def data_url(png):
    return "data:image/png;base64," + base64.b64encode(png).decode()


def main():
    tray_dir = sys.argv[1]
    states = ["running", "connecting", "failed", "idle"]
    tray = {}
    for s in states:
        for theme in ("light", "dark"):
            with open(os.path.join(tray_dir, f"{s}-{theme}.rgba"), "rb") as f:
                tray[(s, theme)] = data_url(png_from_rgba(f.read(), 32, 32))
    icons = {}
    for name in ("icon.png", "128x128@2x.png", "128x128.png", "64x64.png", "32x32.png"):
        with open(os.path.join(ICONS, name), "rb") as f:
            icons[name] = "data:image/png;base64," + base64.b64encode(f.read()).decode()

    def bar(theme):
        bg, fg = ("#2a2d33", "#e6e9ee") if theme == "dark" else ("#e9ebee", "#1d2128")
        cells = "".join(f'<img src="{tray[(s, theme)]}" width="16" height="16" alt="{s}">' for s in states)
        return f'<div class="bar" style="background:{bg};color:{fg}">{cells}<span>9:41</span></div>'

    zoomed = "".join(
        f'<figure><img class="px" src="{tray[(s, t)]}" width="96" height="96" alt="{s} {t}" style="background:{"#2a2d33" if t == "dark" else "#e9ebee"}"><figcaption>{s}, {t}</figcaption></figure>'
        for t in ("light", "dark") for s in states
    )
    sizes = "".join(
        f'<figure><img src="{icons[n]}" width="{w}" height="{w}" alt="{n}"><figcaption>{w} px</figcaption></figure>'
        for n, w in (("icon.png", 256), ("128x128@2x.png", 128), ("64x64.png", 64), ("32x32.png", 32))
    )
    dock = "".join(f'<img src="{icons["128x128@2x.png"]}" width="64" height="64" alt="">' for _ in range(1))
    html = f'''<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Tunwarden icons</title>
<style>
 body {{ margin:0; padding:28px 32px; background:#f4f6f9; color:#17202b; font:13px/1.45 system-ui,-apple-system,sans-serif; }}
 h1 {{ font-size:18px; margin:0 0 16px; }} h2 {{ font-size:14px; margin:20px 0 8px; }}
 .row {{ display:flex; gap:20px; align-items:flex-end; flex-wrap:wrap; }}
 figure {{ margin:0; }} figcaption {{ font-size:12px; color:#5a6576; margin-top:4px; }}
 .bar {{ display:flex; align-items:center; gap:14px; height:26px; padding:0 12px; border-radius:6px; width:260px; margin-bottom:6px; }}
 .bar span {{ margin-left:auto; font-size:12px; }} .px {{ image-rendering:pixelated; border-radius:8px; }}
 .dock {{ display:flex; gap:10px; padding:8px 12px; background:rgba(255,255,255,.55); border:1px solid #dce1e8; border-radius:16px; width:max-content; }}
 .dock.dark {{ background:#2a2d33; border-color:#3a3f47; }}
</style></head><body>
<h1>Tunwarden icons (from the shipped files)</h1>
<h2>App icon</h2><div class="row">{sizes}</div>
<h2>In a dock</h2><div class="row"><div class="dock">{dock}</div><div class="dock dark">{dock}</div></div>
<h2>Tray at 16 pt</h2><div>{bar("light")}{bar("dark")}</div>
<h2>Tray at 6× (running, connecting, failed, idle)</h2><div class="row">{zoomed}</div>
</body></html>'''
    with open(os.path.join(HERE, "preview.html"), "w", encoding="utf-8") as f:
        f.write(html)
    print("wrote docs/brand/preview.html")


if __name__ == "__main__":
    main()
