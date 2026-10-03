#!/usr/bin/env python3
"""Shared ANSI-to-SVG rendering for the docs-showcase generators.

`showcase.py` renders CLI output and `showcase_tui.py` renders terminal-frontend
frames. Both parse the captured ANSI into a cell grid and draw it as an SVG:

* backgrounds are rectangles, so they tile with no seams;
* box-drawing and block characters are drawn as shapes, so lines connect;
* every other character is a centred `<text>`, so columns stay aligned.

The SVG is embedded in a terminal-window page and rasterized with the
macOS-only `qlmanage`.
"""

from __future__ import annotations

import html
import os
import re
import shutil
import subprocess

SGR = re.compile(r"\x1b\[([0-9;]*)m")
OTHER_CSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
OSC = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

XTERM16 = [
    (0, 0, 0), (205, 49, 49), (13, 188, 121), (122, 120, 86),
    (36, 87, 247), (188, 19, 209), (26, 161, 180), (203, 204, 205),
    (103, 110, 114), (229, 83, 83), (45, 208, 145), (166, 164, 110),
    (73, 120, 255), (211, 60, 230), (54, 195, 216), (255, 255, 255),
]

CELL_W = 8.4
CELL_H = 17.0
FONT_SIZE = 14.0
LINE_T = 1.8
DEFAULT_FG = "#c8ccd4"
BOX_CHARS = set("─│║├┤┬┴┼┌┐└┘╭╮╰╯█▌▐▏▀▄")


def xterm256(n: int) -> tuple[int, int, int]:
    if n < 16:
        return XTERM16[n]
    if n < 232:
        n -= 16
        levels = (0, 95, 135, 175, 215, 255)
        return (levels[n // 36], levels[(n % 36) // 6], levels[n % 6])
    v = 8 + (n - 232) * 10
    return (v, v, v)


def rgb(r: int, g: int, b: int) -> str:
    return f"#{r:02x}{g:02x}{b:02x}"


def _parse_grid(text: str):
    """Parse ANSI text into rows of cells `(char, fg, bg, bold, dim, italic,
    underline, rev)`. `fg`/`bg` are hex strings or `None`."""
    state = {"fg": None, "bg": None, "bold": False, "dim": False,
             "italic": False, "underline": False, "strike": False, "rev": False}
    rows: list[list[tuple]] = [[]]
    pos = 0

    def put(chunk: str) -> None:
        for ch in chunk:
            if ch == "\n":
                rows.append([])
                continue
            fg, bg = state["fg"], state["bg"]
            if state["rev"]:
                fg, bg = bg, fg
            rows[-1].append((ch, fg, bg, state["bold"], state["dim"],
                             state["italic"], state["underline"], state["strike"]))

    for m in SGR.finditer(text):
        put(text[pos:m.start()])
        pos = m.end()
        params = [int(p) if p else 0 for p in m.group(1).split(";")] or [0]
        i = 0
        while i < len(params):
            p = params[i]
            if p == 0:
                state.update(fg=None, bg=None, bold=False, dim=False,
                             italic=False, underline=False, strike=False, rev=False)
            elif p == 1:
                state["bold"] = True
            elif p == 2:
                state["dim"] = True
            elif p == 3:
                state["italic"] = True
            elif p == 4:
                state["underline"] = True
            elif p == 7:
                state["rev"] = True
            elif p == 9:
                state["strike"] = True
            elif p == 22:
                state["bold"] = state["dim"] = False
            elif p in (23, 24, 27, 29):
                state[{23: "italic", 24: "underline", 27: "rev", 29: "strike"}[p]] = False
            elif 30 <= p <= 37:
                state["fg"] = rgb(*xterm256(p - 30))
            elif 90 <= p <= 97:
                state["fg"] = rgb(*xterm256(p - 90 + 8))
            elif 40 <= p <= 47:
                state["bg"] = rgb(*xterm256(p - 40))
            elif 100 <= p <= 107:
                state["bg"] = rgb(*xterm256(p - 100 + 8))
            elif p == 39:
                state["fg"] = None
            elif p == 49:
                state["bg"] = None
            elif p in (38, 48):
                key = "fg" if p == 38 else "bg"
                if i + 1 < len(params) and params[i + 1] == 5 and i + 2 < len(params):
                    state[key] = rgb(*xterm256(params[i + 2]))
                    i += 2
                elif i + 1 < len(params) and params[i + 1] == 2 and i + 4 < len(params):
                    state[key] = rgb(params[i + 2], params[i + 3], params[i + 4])
                    i += 4
            i += 1

    put(OTHER_CSI.sub("", OSC.sub("", text[pos:])))
    if text.endswith("\n"):
        rows.pop()
    return rows


def _rect(x, y, w, h, color):
    if w <= 0 or h <= 0:
        return ""
    return f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{color}"/>'


def _box_shape(ch, x, y, color):
    """Shape markup for a box-drawing or block character, or None for text."""
    w, h, t = CELL_W, CELL_H, LINE_T
    cx, cy = x + w / 2, y + h / 2
    half_t = t / 2
    radius = min(w, h) * 0.42
    up = _rect(cx - half_t, y, t, h / 2 + half_t, color)
    down = _rect(cx - half_t, cy - half_t, t, h / 2 + half_t, color)
    left = _rect(x, cy - half_t, w / 2 + half_t, t, color)
    right = _rect(cx, cy - half_t, w / 2, t, color)
    vert = _rect(cx - half_t, y, t, h, color)
    horiz = _rect(x, cy - half_t, w, t, color)

    # Corner paths: vertical arm and horizontal arm meet at the cell centre.
    corners = {
        "┌": f"M {cx} {y + h} L {cx} {cy} L {x + w} {cy}",
        "┐": f"M {cx} {y + h} L {cx} {cy} L {x} {cy}",
        "└": f"M {cx} {y} L {cx} {cy} L {x + w} {cy}",
        "┘": f"M {cx} {y} L {cx} {cy} L {x} {cy}",
        "╭": f"M {cx} {y + h} L {cx} {cy + radius} Q {cx} {cy} {cx + radius} {cy} L {x + w} {cy}",
        "╮": f"M {cx} {y + h} L {cx} {cy + radius} Q {cx} {cy} {cx - radius} {cy} L {x} {cy}",
        "╰": f"M {cx} {y} L {cx} {cy - radius} Q {cx} {cy} {cx + radius} {cy} L {x + w} {cy}",
        "╯": f"M {cx} {y} L {cx} {cy - radius} Q {cx} {cy} {cx - radius} {cy} L {x} {cy}",
    }
    if ch in corners:
        return (f'<path d="{corners[ch]}" fill="none" stroke="{color}" '
                f'stroke-width="{t:.2f}" stroke-linecap="square"/>')

    shapes = {
        "├": vert + right,
        "┤": vert + left,
        "┬": horiz + down,
        "┴": horiz + up,
        "┼": vert + horiz,
        "█": _rect(x, y, w, h, color),
        "▌": _rect(x, y, w / 2, h, color),
        "▐": _rect(x + w / 2, y, w / 2, h, color),
        "▏": _rect(x, y, w / 8, h, color),
        "▀": _rect(x, y, w, h / 2, color),
        "▄": _rect(x, y + h / 2, w, h / 2, color),
    }
    return shapes.get(ch)


def svg_from_ansi(text: str, default_bg: str = "#0b0e14") -> str:
    rows = _parse_grid(text)
    cols = max((len(r) for r in rows), default=0)
    if cols == 0:
        return '<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"></svg>'
    width = cols * CELL_W
    height = len(rows) * CELL_H

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" '
           f'height="{height:.0f}" viewBox="0 0 {width:.0f} {height:.0f}" '
           f'font-family="Menlo, \'SF Mono\', monospace" '
           f'text-rendering="geometricPrecision" shape-rendering="crispEdges">']
    out.append(_rect(0, 0, width, height, default_bg))

    # Backgrounds, merging horizontal runs.
    for r, row in enumerate(rows):
        y = r * CELL_H
        c = 0
        while c < len(row):
            bg = row[c][2]
            if bg is None or bg == default_bg:
                c += 1
                continue
            start = c
            while c < len(row) and row[c][2] == bg:
                c += 1
            out.append(_rect(start * CELL_W, y, (c - start) * CELL_W, CELL_H, bg))

    # Straight box runs are one rect, so they do not break into dashes at cell
    # boundaries.
    for r, row in enumerate(rows):
        c = 0
        while c < len(row):
            if row[c][0] != "─":
                c += 1
                continue
            start = c
            color = row[c][1] or DEFAULT_FG
            while c < len(row) and row[c][0] == "─" and (row[c][1] or DEFAULT_FG) == color:
                c += 1
            out.append(_rect(start * CELL_W, r * CELL_H + CELL_H / 2 - LINE_T / 2,
                             (c - start) * CELL_W, LINE_T, color))
    for c in range(cols):
        r = 0
        while r < len(rows):
            row = rows[r]
            ch = row[c][0] if c < len(row) else None
            if ch not in ("│", "║"):
                r += 1
                continue
            start = r
            color = row[c][1] or DEFAULT_FG
            while r < len(rows):
                rr = rows[r]
                if c < len(rr) and rr[c][0] == ch and (rr[c][1] or DEFAULT_FG) == color:
                    r += 1
                else:
                    break
            top, hgt = start * CELL_H, (r - start) * CELL_H
            if ch == "│":
                out.append(_rect(c * CELL_W + CELL_W / 2 - LINE_T / 2, top, LINE_T, hgt, color))
            else:
                out.append(_rect(c * CELL_W + CELL_W * 0.32, top, LINE_T, hgt, color))
                out.append(_rect(c * CELL_W + CELL_W * 0.62, top, LINE_T, hgt, color))

    # Corners, junctions, blocks, and text runs.
    baseline = CELL_H - 4.5
    for r, row in enumerate(rows):
        y = r * CELL_H
        n = len(row)
        c = 0
        while c < n:
            ch, fg, _bg, bold, dim, italic, underline, strike = row[c]
            if ch in ("│", "║", "─"):
                c += 1
                continue
            if ch in BOX_CHARS:
                out.append(_box_shape(ch, c * CELL_W, y, fg or DEFAULT_FG))
                c += 1
                continue
            if ch == " ":
                c += 1
                continue
            start = c
            key = (fg, bold, dim, italic, underline, strike)
            run = []
            while c < n:
                ch2, fg2, _bg2, b2, d2, i2, u2, s2 = row[c]
                if (fg2, b2, d2, i2, u2, s2) != key or ch2 in BOX_CHARS:
                    break
                run.append(ch2)
                c += 1
            text = "".join(run)
            if not text.strip():
                continue
            style = ""
            if bold:
                style += ' font-weight="700"'
            if italic:
                style += ' font-style="italic"'
            if dim:
                style += ' opacity="0.65"'
            if underline or strike:
                deco = " ".join(k for k, v in (("underline", underline), ("line-through", strike)) if v)
                style += f' text-decoration="{deco}"'
            out.append(
                f'<text x="{start * CELL_W:.2f}" y="{y + baseline:.2f}" '
                f'font-size="{FONT_SIZE}" fill="{key[0] or DEFAULT_FG}" '
                f'textLength="{len(text) * CELL_W:.2f}" lengthAdjust="spacingAndGlyphs" '
                f'xml:space="preserve"{style}>{html.escape(text, quote=False)}</text>')

    out.append("</svg>")
    return "".join(out)


CSS = """
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body { margin:0; background:#07080d; color:#c8ccd4;
  font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; }
.stage { max-width:1500px; margin:0 auto; padding:28px 24px 48px; }
.stage.wide { max-width:1700px; }
h1 { font-size:20px; margin:0 0 4px; color:#e6e9ef; font-weight:650; }
p.lede { margin:0 0 22px; color:#8a93a6; font-size:13.5px; line-height:1.5; }
.sub { color:#6d7688; font-size:12.5px; margin:0 0 18px; }
.window { border-radius:10px; overflow:hidden; border:1px solid #232734;
  box-shadow:0 18px 48px rgba(0,0,0,.55); background:#0b0e14; }
.bar { display:flex; align-items:center; gap:8px; padding:9px 12px;
  background:linear-gradient(#232734,#1a1e29); border-bottom:1px solid #232734; }
.dot { width:11px; height:11px; border-radius:50%; }
.dot.r{background:#ff5f57}.dot.y{background:#febc2e}.dot.g{background:#28c840}
.bar .title { font:12px ui-monospace,SFMono-Regular,Menlo,monospace;
  color:#9aa3b5; margin-left:8px; }
.screen { padding:14px 16px; overflow:auto; }
.screen svg { display:block; max-width:100%; height:auto; }
.cols { display:flex; gap:20px; align-items:flex-start; }
.col { flex:1 1 0; min-width:0; }
.tag { display:inline-block; font:11.5px ui-monospace,Menlo,monospace;
  color:#7ee787; background:#12261a; border:1px solid #1f4d2e;
  padding:2px 8px; border-radius:999px; margin-bottom:8px; }
.tag.sig { color:#ffa657; background:#2a1c0d; border-color:#5c3a16; }
.tall { max-height:820px; }
"""


def page(title: str, subtitle: str, body: str, lede: str = "", wide: bool = False) -> str:
    lede_html = f'<p class="lede">{html.escape(lede)}</p>' if lede else ""
    stage = "stage wide" if wide else "stage"
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(title)}</title><style>{CSS}</style></head>
<body><div class="{stage}">
<h1>{html.escape(title)}</h1>
{subtitle}
{lede_html}
{body}
</div></body></html>"""


def window(inner: str, title: str, extra: str = "") -> str:
    return (f'<div class="window {extra}">'
            '<div class="bar"><span class="dot r"></span><span class="dot y"></span>'
            '<span class="dot g"></span>'
            f'<span class="title">{html.escape(title)}</span></div>{inner}</div>')


def pre(ansi_text: str, cls: str = "screen tall", default_bg: str = "#0b0e14") -> str:
    return f'<div class="{cls}">{svg_from_ansi(ansi_text, default_bg)}</div>'


def truncate(text: str, max_lines: int) -> str:
    lines = text.split("\n")
    if len(lines) > max_lines:
        return "\n".join(lines[:max_lines]) + f"\n… ({len(lines) - max_lines} more lines)"
    return text


def terminal_page(title: str, subtitle: str, ansi_text: str, command: str,
                  lede: str = "", max_lines: int = 0) -> str:
    """A one-window terminal page for a single captured output."""
    body_text = truncate(ansi_text, max_lines) if max_lines else ansi_text
    return page(title, subtitle, window(pre(body_text, "screen tall", "#1a1b26"), command),
                lede, wide=True)


def rasterize(page_path: str, out_dir: str, name: str, size: int = 1800) -> str:
    """Rasterize `page_path` with qlmanage and copy the PNG to `out_dir/name.png`."""
    work = os.path.dirname(page_path)
    subprocess.run(["qlmanage", "-t", "-s", str(size), "-o", work, page_path],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    output = os.path.join(out_dir, name + ".png")
    shutil.copyfile(page_path + ".png", output)
    return output
