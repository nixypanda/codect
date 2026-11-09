#!/usr/bin/env python3
"""Developer benchmark for file move detection.

Builds throwaway Git repositories and times `codect diff` through the public
CLI. The scenarios cover the bounded fallback, fuzzy Myers work that a token
frequency bound cannot prune, and exact streaming hashing above the fuzzy
byte cap.

Usage:
    python3 scripts/bench_move_detection.py
    python3 scripts/bench_move_detection.py --binary target/release/codect
    python3 scripts/bench_move_detection.py --scenario cap --scenario exact
    python3 scripts/bench_move_detection.py --scalar-pairs 32 --line-pairs 8
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BINARY = ROOT / "target" / "release" / "codect"


def git(repo: Path, *args: str) -> None:
    env = {
        **os.environ,
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_AUTHOR_NAME": "Codect Bench",
        "GIT_AUTHOR_EMAIL": "bench@example.invalid",
        "GIT_COMMITTER_NAME": "Codect Bench",
        "GIT_COMMITTER_EMAIL": "bench@example.invalid",
        "GIT_AUTHOR_DATE": "2020-01-01T00:00:00+0000",
        "GIT_COMMITTER_DATE": "2020-01-01T00:00:00+0000",
    }
    subprocess.run(
        ["git", *args],
        cwd=repo,
        env=env,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )


def write(repo: Path, relative: str, text: str) -> None:
    path = repo / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def run_diff(binary: Path, repo: Path, base: str, target: str) -> tuple[float, dict]:
    started = time.perf_counter()
    output = subprocess.run(
        [
            str(binary),
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            base,
            target,
        ],
        cwd=repo,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    elapsed = time.perf_counter() - started
    return elapsed, json.loads(output.stdout)


def status_counts(document: dict) -> dict[str, int]:
    counts: dict[str, int] = {}
    for file in document.get("files", []):
        status = file["status"]
        counts[status] = counts.get(status, 0) + 1
    return counts


def make_repo() -> Path:
    root = Path(tempfile.mkdtemp(prefix="codect-move-bench-"))
    git(root, "init", "-q")
    return root


def commit(repo: Path, message: str) -> None:
    git(repo, "add", "--all")
    git(repo, "commit", "-q", "--allow-empty", "-m", message)


def cap_fallback(binary: Path, count: int) -> dict:
    repo = make_repo()
    try:
        for index in range(count):
            write(
                repo,
                f"src/old_{index:04}.rs",
                f"pub struct Old{index:04} {{ pub value: u32 }}\n",
            )
        commit(repo, "base")
        for index in range(count):
            (repo / f"src/old_{index:04}.rs").unlink()
            write(
                repo,
                f"src/new_{index:04}.rs",
                f"pub struct Old{index:04} {{ pub value: u64 }}\n",
            )
        commit(repo, "replace")
        elapsed, document = run_diff(binary, repo, "HEAD~1", "HEAD")
        return {
            "scenario": f"cap-fallback-{count}",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def scalar_shuffled(binary: Path, pairs: int, size: int) -> dict:
    repo = make_repo()
    try:
        token = "abcdefghij"
        for index in range(pairs):
            body = token * (size // len(token))
            write(
                repo,
                f"src/old_{index:02}.rs",
                f"pub struct Marker;\npub const OLD_{index:02}: u8 = 0;\n// {body}\n",
            )
        commit(repo, "base")
        for index in range(pairs):
            (repo / f"src/old_{index:02}.rs").unlink()
            body = (token[::-1]) * (size // len(token))
            write(
                repo,
                f"src/new_{index:02}.rs",
                f"pub struct Marker;\npub const NEW_{index:02}: u8 = 0;\n// {body}\n",
            )
        commit(repo, "replace")
        elapsed, document = run_diff(binary, repo, "HEAD~1", "HEAD")
        return {
            "scenario": f"scalar-shuffled-{pairs}x{pairs}",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def line_shuffled(binary: Path, pairs: int, lines: int) -> dict:
    repo = make_repo()
    try:
        def source(direction: range) -> str:
            return "pub struct Marker;\n" + "".join(
                f"// token_{index:05}\n" for index in direction
            )

        ascending = source(range(lines))
        descending = source(range(lines - 1, -1, -1))
        for index in range(pairs):
            write(repo, f"src/old_{index:02}.rs", ascending)
        commit(repo, "base")
        for index in range(pairs):
            (repo / f"src/old_{index:02}.rs").unlink()
            write(repo, f"src/new_{index:02}.rs", descending)
        commit(repo, "replace")
        elapsed, document = run_diff(binary, repo, "HEAD~1", "HEAD")
        return {
            "scenario": f"line-shuffled-{pairs}x{pairs}",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def mixed_alphabet(binary: Path, pairs: int, comments: int, blanks: int) -> dict:
    repo = make_repo()
    try:
        alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
        comment_lines = []
        for index in range(comments):
            value = index
            name = ["0", "0", "0"]
            for slot in (2, 1, 0):
                name[slot] = alphabet[value % len(alphabet)]
                value //= len(alphabet)
            comment_lines.append(f"//{''.join(name)}\n")
        blank_text = "\n" * blanks

        for index in range(pairs):
            old_comments = list(comment_lines)
            new_comments = list(comment_lines)
            random.Random(1_000 + index).shuffle(old_comments)
            random.Random(10_000 + index).shuffle(new_comments)
            old_text = "".join(old_comments) + blank_text
            new_text = blank_text + "".join(new_comments)
            assert len(old_text.encode()) <= 256 * 1024
            assert len(new_text.encode()) <= 256 * 1024
            write(repo, f"old_{index:02}.rs", old_text)
        commit(repo, "base")
        for index in range(pairs):
            (repo / f"old_{index:02}.rs").unlink()
            write(repo, f"new_{index:02}.rs", new_text)
        commit(repo, "replace")
        elapsed, document = run_diff(binary, repo, "HEAD~1", "HEAD")
        return {
            "scenario": f"mixed-alphabet-unique-{pairs}x{pairs}",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)



def line_overlap(binary: Path, pairs: int, shared: int, unique: int) -> dict:
    repo = make_repo()
    try:
        shared_text = "".join(f"// shared_{index:05}\n" for index in range(shared))
        old_text = "pub struct Marker;\n" + shared_text + "".join(
            f"// old_{index:05}\n" for index in range(unique)
        )
        new_text = "pub struct Marker;\n" + shared_text + "".join(
            f"// new_{index:05}\n" for index in range(unique)
        )
        for index in range(pairs):
            write(repo, f"src/old_{index:02}.rs", old_text)
        commit(repo, "base")
        for index in range(pairs):
            (repo / f"src/old_{index:02}.rs").unlink()
            write(repo, f"src/new_{index:02}.rs", new_text)
        commit(repo, "replace")
        elapsed, document = run_diff(binary, repo, "HEAD~1", "HEAD")
        return {
            "scenario": f"line-overlap-{pairs}x{pairs}",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def exact_large_worktree(binary: Path, size: int) -> dict:
    repo = make_repo()
    try:
        padding = "// padding padding padding padding padding padding\n"
        source = padding * (size // len(padding) + 1) + "pub struct Large;\n"
        write(repo, "src/old.rs", source)
        commit(repo, "base")
        (repo / "src/old.rs").rename(repo / "src/new.rs")
        elapsed, document = run_diff(binary, repo, "HEAD", ":worktree")
        return {
            "scenario": f"exact-large-worktree-{size // 1024}KiB",
            "seconds": round(elapsed, 3),
            "files": len(document.get("files", [])),
            "statuses": status_counts(document),
        }
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument(
        "--scenario",
        action="append",
        choices=["cap", "scalar", "line", "overlap", "mixed", "exact"],
        help="run only the named scenario (repeatable)",
    )
    parser.add_argument("--cap-count", type=int, default=257)
    parser.add_argument("--scalar-pairs", type=int, default=16)
    parser.add_argument("--scalar-size", type=int, default=3800)
    parser.add_argument("--line-pairs", type=int, default=8)
    parser.add_argument("--line-count", type=int, default=4000)
    parser.add_argument("--exact-size", type=int, default=300 * 1024)
    parser.add_argument("--overlap-pairs", type=int, default=64)
    parser.add_argument("--overlap-shared", type=int, default=9_000)
    parser.add_argument("--overlap-unique", type=int, default=9_000)
    parser.add_argument("--mixed-pairs", type=int, default=64)
    parser.add_argument("--mixed-comments", type=int, default=38_000)
    parser.add_argument("--mixed-blanks", type=int, default=20_000)
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit one JSON object per scenario",
    )
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.exists():
        subprocess.run(
            ["cargo", "build", "--release", "-p", "cli"],
            cwd=ROOT,
            check=True,
        )
    selected = set(args.scenario or ["cap", "scalar", "line", "overlap", "mixed", "exact"])

    results = []
    if "cap" in selected:
        results.append(cap_fallback(binary, args.cap_count))
    if "scalar" in selected:
        results.append(
            scalar_shuffled(binary, args.scalar_pairs, args.scalar_size)
        )
    if "line" in selected:
        results.append(line_shuffled(binary, args.line_pairs, args.line_count))
    if "overlap" in selected:
        results.append(
            line_overlap(
                binary,
                args.overlap_pairs,
                args.overlap_shared,
                args.overlap_unique,
            )
        )
    if "mixed" in selected:
        results.append(
            mixed_alphabet(
                binary,
                args.mixed_pairs,
                args.mixed_comments,
                args.mixed_blanks,
            )
        )
    if "exact" in selected:
        results.append(exact_large_worktree(binary, args.exact_size))

    for result in results:
        if args.json:
            print(json.dumps(result, sort_keys=True))
        else:
            print(
                f"{result['scenario']}: {result['seconds']}s "
                f"files={result['files']} statuses={result['statuses']}"
            )


if __name__ == "__main__":
    main()
