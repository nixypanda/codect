# Codect

Codect provides selectable views of a codebase at different levels of detail.
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

## Install with Nix

The flake provides native packages for `x86_64` and `aarch64` on both macOS
and Linux. From a checkout, install the package into your user profile with:

```sh
nix profile install .
```

Or run it without installing:

```sh
nix run . -- --help
```

When installing from GitHub, replace `.` with `github:nixypanda/codect`. Nix
automatically selects the package matching the current system.
To update an installation made from this checkout, run
`nix profile upgrade codect`.

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

The `cargo tree` command audits the enabled `gix` features. It must show
nothing beyond the approved feature list in TECHNICAL_DESIGN.md section 4.1 and
their unavoidable transitive implications.

The final `cargo build` proves the terminal frontend is optional: with
`--no-default-features`, `ratatui`, `crossterm`, `terminal-colorsaurus`,
`syntect`, and `two-face` must not appear in `cli`'s dependency tree.
`just check-workspace-nodefault` also checks this dependency tree.

The Neovim plugin has a separate headless suite (`just test-nvim`) and a Nix
flake check (`nix flake check`).

Individual recipes are available as `just build`, `just test`, `just format`,
`just check-workspace-clippy`, `just check-workspace-features`, and
`just check-workspace-nodefault`.

## Usage

```text
codect show --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
codect show --format json --mode <types|signatures> --stdin    --path <FILE>
codect show --format json --mode <types|signatures> --worktree --path <FILE>
codect diff --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`--format` defaults to `text`, the canonical projection. `--format json` emits
the versioned `codect.show.v1` document: the requested mode's projection plus a
mode-independent declaration outline that an editor can turn into semantic
folds. `--stdin` projects a buffer's bytes and `--worktree` projects the file at
`--path` on disk; both require exactly one `--path` naming a file, and the
result carries the same stable keys as a committed revision. See
[docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) section 14.2 and
[docs/schema/codect.show.v1.json](./docs/schema/codect.show.v1.json) for the
contract.

`codect diff --format json` emits `codect.diff.v1` for commits and the reserved
snapshot names `:index`, `:worktree`, and `:empty`. The canonical Git empty-tree
object ID is also accepted, so the initial commit can be compared directly.
For example, compare staged changes with `codect diff --format json --mode types HEAD :index`
and unstaged changes with `codect diff --format json --mode types :index :worktree`.
It includes the resolved commit IDs (or mutable snapshot names) and each changed file's projected base and
target panes with declaration items and outlines. An absent side is `null`.
Files with equal projected text are omitted, so an empty `files` array means
there are no focused changes. The JSON contract is specified in
[docs/schema/codect.diff.v1.json](./docs/schema/codect.diff.v1.json). Index reads
the stage-zero blob bytes; worktree reads tracked regular files on disk and
does not follow symlinks. Mutable snapshot IDs are labels, so integrations
must refresh after staging or file writes. Unsaved editor buffer bytes are not
part of the worktree snapshot.

The Neovim plugin provides semantic folds over editable source buffers and an
optional, version-pinned Diffview integration for focused history and local
changes. See [editors/nvim/README.md](./editors/nvim/README.md) for setup,
commands, and compatibility details.

`--path`/`-p` is repeatable and scopes a command to a file or directory (a
directory includes everything beneath it); a path that names nothing in the
projected revision — or in either side of a diff — is an error. Paths are
relative to the current directory and must stay inside the repository.

`--area`/`-a` is repeatable and selects named path groups defined in
`.codect.toml` at the repository root. Area paths are relative to the repository
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

`codect` also ships an interactive terminal browser for the same projections. It
is a default-on feature of the CLI.

```text
codect tui show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
codect tui diff range --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
codect tui diff commits --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`tui show` opens a file tree beside the canonical projection of the selected
file. `tui diff range` compares the two revisions directly. `tui diff commits`
lists the commits after BASE through TARGET on TARGET's first-parent chain,
newest first; selecting a commit compares it with its first parent. BASE must
be on that chain. The commit picker sits above the changed-file tree. Both
views show a side-by-side projection diff. The diff panes use Tokyo Night
night/day syntax colors and added/deleted line backgrounds, brighter intra-line
emphasis on the bytes that changed, and full-width `@@` hunk bands. `NO_COLOR`
disables all styling.

A command palette (`Ctrl-P`), a fuzzy file finder (`Ctrl-F`), and in-pane search
(`/`) make the frontend navigable without memorizing keys. The UI is themed:
`CODECT_THEME=light` switches to a light palette, and colors degrade gracefully on
terminals that only support 256 or 16 colors.

The file tree can draw Nerd Font folder and file-type glyphs with `--icons=nerd`
(or `CODECT_ICONS=nerd`). This requires a Nerd Font installed in your terminal;
without one the glyphs render as boxes, so the default is `--icons=none`.

Keybindings:

| Key | Action |
|---|---|
| `q`, `Ctrl-C` | Quit |
| `↑`/`↓` or `k`/`j` | Move in the focused commit list or tree, or scroll the content |
| `←`/`→` or `h`/`l` | Fold/unfold the tree, or scroll the projection sideways |
| `g` / `G` | Jump to the top or bottom |
| `PageUp`/`PageDown`, `Ctrl-U`/`Ctrl-D` | Move or scroll a half page |
| `Tab`, `Shift-Tab` | Cycle through Commits, Files, and Diff in commits view; switch between tree and content elsewhere |
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
| Click a commit | Select it and show its parent-to-commit diff |
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
cargo build -p cli --no-default-features
```

`tui` then becomes an unknown command, and no terminal dependency is linked.

## Performance baseline

Wall-clock measurements for representative synthetic and real repositories are
recorded in [docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) section 17.
They are a baseline for comparison, not a target.
