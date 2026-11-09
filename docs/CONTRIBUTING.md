# Contributing

## Development environment

The toolchain comes from the Nix flake. See [INSTALL.md](INSTALL.md) for the
full setup; in short:

```sh
nix develop     # or `direnv allow` with direnv
```

## Command runner

Run `just` for the list. Common recipes:

| Recipe | Runs |
| --- | --- |
| `just build` / `just test` | Release build / workspace tests |
| `just format` | `cargo fmt --all` plus `nix fmt` |
| `just check` | The full required check suite |
| `just check-tui-nodiff` | Build and test `tui` without the diff plugin |
| `just check-tui-core-boundary` | Assert `tui-core` links no plugin crate |
| `just bench-tui` | The terminal frontend frame benchmark |
| `just test-nvim` | The Neovim plugin's headless suite |
| `just coverage` | `cargo llvm-cov --workspace` |
| `just showcase` | Regenerate the `docs/showcase/` images |

`just showcase` rasterizes the CLI figures (`scripts/showcase.py`) and the
terminal-frontend frames (`crates/tui/examples/frames.rs`, rendered by
`scripts/showcase_tui.py`) from this repository. It needs `delta` and, for PNG
rendering, the macOS-only `qlmanage`.

## Required checks

Run every required development check with one command:

```sh
just check
```

This is equivalent to:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p tui --features bench --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo tree -e features -p git
cargo build -p cli --no-default-features
cargo build -p tui --no-default-features
cargo test -p tui --no-default-features
cargo tree -p tui-core
just test-dsh
```

Three checks encode architectural invariants:

- The `cargo tree` command audits the enabled `gix` features. It must show
  nothing beyond the approved list in
  [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) section 4.1 and their unavoidable
  transitive implications.
- The `cli` no-default-features build proves the terminal frontend is optional:
  with `--no-default-features`, `ratatui`, `crossterm`, `terminal-colorsaurus`,
  `syntect`, and `two-face` must not appear in `cli`'s dependency tree.
  `just check-workspace-nodefault` also asserts this with `cargo tree`.
- `just check-tui-nodiff` proves `tui` builds and passes its tests without the
  diff plugin and that `tui-diff` is absent from that tree;
  `just check-tui-core-boundary` proves `tui-core` links neither `tui` nor
  `tui-diff`.

The Neovim plugin has a separate headless suite (`just test-nvim`) and a Nix
flake check (`nix flake check`). The DSH plugin has its own Node suite
(`just test-dsh`). Pinned Diffview integration is run separately with
`CODECT_DIFFVIEW_RTP` pointing at the checked-out revision. Run each view
separately; move coverage uses the same entry points as the baseline cases:

```sh
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=history \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=range \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=local \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=move \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=move-split \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=move-paths \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=move-tree-labels \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=move-source-cycle \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=root \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_TEST_DIFFVIEW_CASE=branch \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua

# Independent move acceptance suite
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
CODECT_BIN="$PWD/target/debug/codect" \
nvim --headless -u NONE -l editors/nvim/tests/test_move_acceptance.lua
```

The move cases cover exact and edited moves in those views, Tests mode,
one-to-one entry reconciliation, and no duplicate or null-pane rows. Record them
as passing only after the pinned run has completed.

## Tests

- Unit tests live beside the code they cover.
- Integration tests live under `crates/<crate>/tests/` and may share helpers
  through a `tests/support/` module.
- Repository-level language fixtures live under `fixtures/<language>/`. Each
  case holds an input file and its expected `types.txt` / `signatures.txt`.
- Invariance tests (`crates/lang-*/tests/invariance.rs`) prove that body,
  comment, and whitespace edits leave projections unchanged, and that type and
  signature edits change the right mode.
- Git tests create real temporary repositories with the `git` executable. This
  is the only place tests may invoke Git; library and CLI code never do.
- CLI tests assert stdout, stderr, and exit status for every command form,
  including a case that redirected output contains no escape bytes.
- Path-display tests cover literal backslash versus invalid-byte escapes,
  LF/CR/TAB, other C0 and DEL with uppercase `\xNN`, ordinary Unicode unchanged,
  JSON escaping as an additional layer, raw-byte ordering independent of display,
  and both show and diff renderers.
- The terminal frontend is tested with `ratatui::TestBackend`, pure `update`
  tests, and an injected terminal driver; a PTY smoke test covers the real
  crossterm path where supported.
- Move tests cover exact and edited moves, scope independence, raw path
  ordering, ambiguous identical sources, copy-as-addition, Tests-mode
  visibility, index/worktree/untracked and merge-base comparisons, the
  deterministic fuzzy caps, and the pure-move TUI empty state with no hunk
  rows.
- The kernel is tested over fake plugins from `tui_core::testing`; the plugin
  crates keep their own unit tests. `crates/tui/tests/golden/*.txt` are
  byte-stable frames generated by the scenarios in `crates/tui/src/golden.rs`.
  Run them without regeneration after any refactor; only a task that lists an
  intentional change may commit rewritten frames
  (`UPDATE_GOLDEN=1 cargo test -p tui golden`).

See [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) section 16 for the full case
lists, and [INVARIANTS.md](agents/INVARIANTS.md) for the properties they guard.

## Performance

Correctness and stable output take priority over concurrency. The measured
baseline for `show` and `diff` is recorded in
[TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) section 17; it is a comparison
baseline, not a target. Do not add threads, persistent caches, or broader `gix`
features without benchmarks.

For the terminal frontend, `just bench-tui` reports per-frame totals and the
seams they are made of (`highlight`, `layout_diff`, ratatui's surface diff scan)
and writes an HTML report under `target/criterion/`.

File-move detection has a developer benchmark script:

```sh
python3 scripts/bench_move_detection.py
python3 scripts/bench_move_detection.py --scenario overlap --json
python3 scripts/bench_move_detection.py --scenario mixed --json
```

It builds throwaway repositories and times `codect diff` through the public CLI.
The scenarios cover the bounded cap fallback, fuzzy scalar and line Myers work
that the frequency bound cannot prune, a line-overlap case with roughly 55%
shared source near the threshold, a mixed-alphabet case with unique random
permutations and no duplicate file contents (so score caching cannot hide the
worst case), and exact streaming identity above the fuzzy byte cap. Use it to record before/after wall-clock evidence for the completion
gate. Exact worktree identity may stream a large source; the fuzzy path reads at
most 256 KiB per candidate. Safe frequency and LCS-length-derived byte upper
bounds may skip only pairs provably below 50% and never replace the final
deterministic Myers score for survivors.

Benchmarks are a developer-run check. CI does not run them; its suite covers
correctness, feature boundaries, and the plugin suites only.

## Security and robustness

Repositories and source files are untrusted input. Do not execute repository
configuration, hooks, filters, attributes, macros, build scripts, compilers, or
formatters; do not follow repository symlinks; bound configurable caches; and
report allocation or parser failures rather than panicking. Reserve `panic!`,
`unwrap`, and `expect` for tests and statically guaranteed initialization.

## Releasing

Releases are tagged `vX.Y.Z` and published by the release workflow. See
[RELEASING.md](RELEASING.md) for the version bump and tag process.

## Dependency policy

- Declare shared versions under `[workspace.dependencies]` and commit
  `Cargo.lock`.
- Keep `gix` at `default-features = false` and enable only the approved
  features. Review the list on every upgrade, because `gix` is pre-1.0.
- Do not add an async runtime. All work is local and synchronous.
- Terminal dependencies stay behind the default-on `tui` feature.
