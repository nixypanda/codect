#!/usr/bin/env python3
"""Render terminal-frontend frames into docs/showcase PNGs.

Reads the `.ansi` frames produced by `crates/tui/examples/frames.rs`, wraps each
in a terminal-window page, and rasterizes it with the macOS-only `qlmanage`.

`just showcase` builds the example and then calls this script.

Usage:
    python3 scripts/showcase_tui.py [--frames DIR] [--out DIR]
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import tempfile

from ansi_html import rasterize, terminal_page

DEFAULT_FRAMES = "target/showcase-frames"
DEFAULT_OUT = "docs/showcase"
RENDER_SIZE = 1800

# (frame file stem, page title, command shown in the window bar, caption)
FRAMES = [
    ("tui-show", "Terminal frontend — show", "codect tui show --mode types",
     "A file tree beside the canonical projection. `m` switches Types/Signatures, "
     "`s` changes scope, and the projection is searchable with `/`."),
    ("tui-diff-range", "Terminal frontend — diff range", "codect tui diff range --mode types",
     "A side-by-side projection diff with full-width hunk bands and brighter "
     "intra-line emphasis on the bytes that changed."),
    ("tui-diff-commits", "Terminal frontend — diff commits",
     "codect tui diff commits --mode types",
     "A scrollable commit picker above the changed-file tree. Selecting a commit "
     "diffs it against its first parent."),
    ("tui-overlay", "Terminal frontend — command palette",
     "codect tui show --mode types  ·  Ctrl-P",
     "Every action is reachable from the command palette, so bindings need not be "
     "memorized; keys and palette entries dispatch the same actions."),
]

SUBTITLE = ("<p class=\"sub\">Rendered from this repository &middot; "
            "read-only</p>")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--frames", default=DEFAULT_FRAMES)
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()

    if not shutil.which("qlmanage"):
        print("error: `qlmanage` not found (macOS-only PNG rendering)", file=sys.stderr)
        return 1

    here = os.path.dirname(os.path.abspath(__file__))
    frames_dir = os.path.abspath(os.path.join(here, "..", args.frames))
    out_dir = os.path.abspath(os.path.join(here, "..", args.out))
    os.makedirs(out_dir, exist_ok=True)

    missing = [name for name, *_ in FRAMES if not os.path.exists(os.path.join(frames_dir, name + ".ansi"))]
    if missing:
        print(f"error: missing frames in {frames_dir}: {', '.join(missing)}\n"
              f"run `cargo run -q -p tui --features bench --example frames -- --out {args.frames}`",
              file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="codect-showcase-tui-") as tmp:
        for name, title, command, lede in FRAMES:
            with open(os.path.join(frames_dir, name + ".ansi")) as f:
                ansi = f.read()
            content = terminal_page(title, SUBTITLE, ansi, command, lede)
            page_path = os.path.join(tmp, name + ".html")
            with open(page_path, "w") as f:
                f.write(content)
            output = rasterize(page_path, out_dir, name, RENDER_SIZE)
            print(f"wrote {os.path.relpath(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
