# OwnAI

OwnAI provides selectable views of a codebase at different levels of detail.
Focused views remove implementation bodies so that a person or tool can study the
shape of the code without reading how it works.

See [docs/PRODUCT.md](./docs/PRODUCT.md) for product behavior and scope, and
[docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) for the implementation contract.

## Development environment

The toolchain is provided by Nix (flakes). Rust itself comes from
[`rust-overlay`](https://github.com/oxalica/rust-overlay) pinned to current
stable; nixpkgs is pinned to the `nixos-26.05` release.

```sh
nix develop
```

With [direnv](https://direnv.net/) installed, `direnv allow` activates the same
shell automatically via `.envrc`.

## Required checks

Run every required development check with one command:

```sh
just check
```

This is equivalent to:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo tree -e features -p ownai-git
cargo build -p ownai-cli --no-default-features
```

The `cargo tree` command audits the enabled `gix` features. It must show
nothing beyond the approved feature list in TECHNICAL_DESIGN.md section 4.1 and
their unavoidable transitive implications.

The final `cargo build` proves the terminal frontend is optional: with
`--no-default-features`, `ratatui`, `crossterm`, `syntect`, and `two-face` must
not appear in `ownai-cli`'s dependency tree.

Individual recipes are available as `just build`, `just test`, `just format`,
`just check-workspace-clippy`, `just check-workspace-features`, and
`just check-workspace-nodefault`.

## Usage

```text
ownai show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai diff --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`--path`/`-p` is repeatable and scopes a command to a file or directory (a
directory includes everything beneath it); a path that names nothing in the
projected revision — or in either side of a diff — is an error. Paths are
relative to the current directory and must stay inside the repository.

`--area`/`-a` is repeatable and selects named path groups defined in
`.ownai.toml` at the repository root. Area paths are relative to the repository
root and are read only when `--area` is used; `--area` and `--path` cannot be
combined.

```toml
[areas]
frontend = ["apps/web", "packages/ui"]
backend  = ["services/api"]
```

Focused diffs intentionally hide implementation-only changes: if a function body
changes while its projected declaration is unchanged, the focused diff shows no
change for that function.

## Terminal frontend

`ownai` also ships an interactive terminal browser for the same projections. It
is a default-on feature of the CLI.

```text
ownai tui show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai tui diff --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`tui show` opens a file tree beside the canonical projection of the selected
file. `tui diff` opens a changed-file tree beside a side-by-side projection
diff. The diff panes are syntax-highlighted and styled like
[`delta`](https://github.com/dandavison/delta): per-token syntax colors,
full-line add/delete backgrounds, brighter intra-line emphasis on the bytes that
changed, and `@@` hunk headers. `NO_COLOR` disables all styling.

A command palette (`Ctrl-P`), a fuzzy file finder (`Ctrl-F`), and in-pane search
(`/`) make the frontend navigable without memorizing keys. The UI is themed:
`OWNAI_THEME=light` switches to a light palette, and colors degrade gracefully on
terminals that only support 256 or 16 colors.

The file tree can draw Nerd Font folder and file-type glyphs with `--icons=nerd`
(or `OWNAI_ICONS=nerd`). This requires a Nerd Font installed in your terminal;
without one the glyphs render as boxes, so the default is `--icons=none`.

Keybindings:

| Key | Action |
|---|---|
| `q`, `Ctrl-C` | Quit |
| `↑`/`↓` or `k`/`j` | Move in the tree, or scroll the focused content |
| `←`/`→` or `h`/`l` | Fold/unfold the tree, or scroll the projection sideways |
| `g` / `G` | Jump to the top or bottom |
| `PageUp`/`PageDown`, `Ctrl-U`/`Ctrl-D` | Move or scroll a half page |
| `Tab`, `Shift-Tab` | Switch between the tree and the content |
| `Enter` | Open a file or fold a directory |
| `Ctrl-P` | Open the command palette |
| `Ctrl-F` | Find a file by name |
| `/`, then `n`/`N` | Search the current view and step through matches |
| `m` | Choose Types or Signatures |
| `s` | Choose a scope: everything, a named area, or a literal path |
| `r` | Edit the show revision |
| `b`, `t` | Edit the diff base and target revisions |
| `[`, `]` | Shrink or grow the file tree; `\` resets it |
| `?` | Toggle help |
| `Esc` | Close an overlay, clear a search, or dismiss a diagnostic |

Mouse:

| Input | Action |
|---|---|
| Click a file | Select it; clicking a directory folds or unfolds it |
| Wheel over a pane | Scroll that pane and focus it |

Mouse capture is enabled so clicks and the wheel work. That takes over the
terminal's own text selection, so to copy text hold the terminal's selection
override — usually `Shift` while dragging — or turn mouse capture off in your
terminal. Overlays stay keyboard-driven while they are open.

Requirements:

- Both standard input and standard output must be terminals; a redirected
  invocation exits `1` with empty stdout and one diagnostic on stderr.
- The frontend is read-only: it reads committed blobs and never writes the
  repository, worktree, or index.
- Terminals narrower than 40 columns or shorter than 8 rows show a minimal
  "terminal too small" view; between 40 and 79 columns one region is shown at a
  time, and at 80 columns or wider the tree and content share the screen.

Building without the frontend:

```sh
cargo build -p ownai-cli --no-default-features
```

`tui` then becomes an unknown command, and no terminal dependency is linked.

## Performance baseline

Wall-clock measurements for representative synthetic and real repositories are
recorded in [docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) section 17.
They are a baseline for comparison, not a target.
