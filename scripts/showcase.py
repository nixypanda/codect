#!/usr/bin/env python3
"""Regenerate the CLI docs/showcase images from a focused diff of a PR range.

Captures `codect show`/`codect diff` output in native color and piped through
`delta`, renders each as a terminal-window HTML page, and rasterizes it to PNG.

PNG rendering uses the macOS-only `qlmanage`; `delta` and `python3` are also
required. Run `just showcase` from the development shell.

Usage:
    python3 scripts/showcase.py [--base REV] [--target REV] [--out DIR]
"""

from __future__ import annotations

import argparse
import html
import os
import shutil
import subprocess
import sys
import tempfile

from ansi_html import page, pre, rasterize, truncate, window

# PR #16 refactor/base-doc-and-keys: extracts shared Doc/KeyAllocator types.
DEFAULT_BASE = "4e09c9c^"
DEFAULT_TARGET = "4e09c9c"
DEFAULT_BINARY = "target/release/codect"
DEFAULT_OUT = "docs/showcase"
RENDER_SIZE = 1800
DELTA_WIDTH = 110  # no wrapping, no delta `↴` markers; max content is ~117 cols


def run(argv: list[str], **kw) -> str:
    return subprocess.run(argv, capture_output=True, text=True, check=True, **kw).stdout


def capture(binary: str, args: list[str], *, color: bool, delta: bool) -> str:
    cmd = [binary]
    if color:
        cmd.append("--color=always")
    cmd += args
    if not delta:
        return run(cmd)
    first = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if first.returncode != 0:
        sys.stderr.write(first.stderr.decode("utf-8", "replace"))
        raise SystemExit(f"error: {' '.join(cmd)} exited {first.returncode}")
    delta_cmd = ["delta", "--paging=never", "--true-color=always", f"--width={DELTA_WIDTH}"]
    out = subprocess.run(delta_cmd, input=first.stdout, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL)
    return out.stdout.decode("utf-8", "replace")


def build_html(binary: str, base: str, target: str, workdir: str) -> dict[str, str]:
    rng = [f"{base}", f"{target}"]
    types_raw = capture(binary, ["diff", "--mode", "types", *rng], color=True, delta=False)
    types_delta = capture(binary, ["diff", "--mode", "types", *rng], color=False, delta=True)
    sig_raw = capture(binary, ["diff", "--mode", "signatures", *rng], color=True, delta=False)
    show = capture(binary, ["show", "--mode", "types", target], color=True, delta=False)

    sub = f'<p class="sub">PR showcase &middot; base {html.escape(base)} → {html.escape(target)}</p>'

    pages = {}
    pages["types-diff-native"] = page(
        "codect — focused types diff (native color)", sub,
        window(pre(types_raw), "codect diff --mode types"),
        "Codect's own --color=always output: declarations only, no function bodies.")
    pages["types-diff-delta"] = page(
        "codect piped through delta", sub,
        window(pre(types_delta), "codect diff --mode types … | delta"),
        "The same output piped to delta: line numbers, backgrounds, intra-line emphasis.")
    pages["show-types"] = page(
        "codect show --mode types", sub,
        window(pre(truncate(show, 150)), "codect show --mode types"),
        "The whole repository as an outline: types and signatures, no bodies, no imports.")

    left = ('<span class="tag">--mode types</span>'
            + window(pre(truncate(types_raw, 260)), "codect diff --mode types"))
    right = ('<span class="tag sig">--mode signatures</span>'
             + window(pre(truncate(sig_raw, 260)), "codect diff --mode signatures"))
    pages["types-vs-signatures"] = page(
        "Types vs Signatures — the same change", sub,
        f'<div class="cols compact"><div class="col">{left}</div><div class="col">{right}</div></div>',
        "One change, two focused views. Types shows declaration shape; Signatures adds "
        "every signature and grows substantially. Both hide bodies.")

    paths = {}
    for name, content in pages.items():
        path = os.path.join(workdir, name + ".html")
        with open(path, "w") as f:
            f.write(content)
        paths[name] = path
    return paths


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default=DEFAULT_BASE)
    ap.add_argument("--target", default=DEFAULT_TARGET)
    ap.add_argument("--binary", default=DEFAULT_BINARY)
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()

    binary = os.path.abspath(args.binary)
    if not os.path.exists(binary):
        print(f"error: {args.binary} not found; run `cargo build -p cli --release`", file=sys.stderr)
        return 1
    for tool in ("delta", "qlmanage"):
        if not shutil.which(tool):
            print(f"error: `{tool}` not found (delta is required; qlmanage is macOS-only)",
                  file=sys.stderr)
            return 1

    out_dir = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", args.out))
    os.makedirs(out_dir, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="codect-showcase-") as tmp:
        pages = build_html(binary, args.base, args.target, tmp)
        for name, page_path in pages.items():
            output = rasterize(page_path, out_dir, name, RENDER_SIZE)
            print(f"wrote {os.path.relpath(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
