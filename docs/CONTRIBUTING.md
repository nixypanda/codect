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
```

Two checks encode architectural invariants:

- The `cargo tree` command audits the enabled `gix` features. It must show
  nothing beyond the approved list in
  [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) section 4.1 and their unavoidable
  transitive implications.
- The final build proves the terminal frontend is optional: with
  `--no-default-features`, `ratatui`, `crossterm`, `terminal-colorsaurus`,
  `syntect`, and `two-face` must not appear in `cli`'s dependency tree.
  `just check-workspace-nodefault` also asserts this with `cargo tree`.

The Neovim plugin has a separate headless suite (`just test-nvim`) and a Nix
flake check (`nix flake check`).

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
- The terminal frontend is tested with `ratatui::TestBackend`, pure `update`
  tests, and an injected terminal driver; a PTY smoke test covers the real
  crossterm path where supported.

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

Benchmarks are a developer-run check. CI does not run them; its suite covers
correctness, feature boundaries, and the Neovim plugin only.

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
