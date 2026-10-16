# Terminal UI

`codect` ships an interactive terminal browser for the same focused projections.
It is a default-on feature of the CLI.

## Launch

```text
codect tui show --mode <types|signatures|tests> [--path <PATH> | --area <AREA>]... [REVISION]
codect tui diff range --mode <types|signatures|tests> [--merge-base] [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
codect tui diff commits --mode <types|signatures|tests> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

- `tui show` opens a source explorer: a file tree beside the real source of the
  selected file, folded to the requested mode.
- `tui diff range` compares the two endpoint snapshots directly; either side
  may be a commit, `:index`, `:worktree`, or `:empty`, as in `codect diff`.
  `--merge-base` replaces the base with the merge base of the two commits and
  labels the base pane, matching `codect diff`.
- `tui diff commits` lists the commits after `BASE` through `TARGET` on
  `TARGET`'s first-parent chain, newest first; selecting a commit compares it
  with its first parent. `BASE` must be on that chain. The commit picker is
  pinned as a full-width band at the bottom of the body, above the status bar.
  It needs commit sides, so `:index`, `:worktree`, and `:empty` are a usage
  error, and it does not accept `--merge-base`.
- Both views show a side-by-side projection diff.

![codect tui show — a file tree beside the mode-folded source](showcase/tui-show.png)

*`codect tui show --mode types`: the file tree, the source folded to the mode, and the status bar.*

The diff panes use Tokyo Night night/day syntax colors and added/deleted line
backgrounds, brighter intra-line emphasis on the bytes that changed, and
full-width `@@` hunk bands. `NO_COLOR` disables all styling.

![codect tui diff range — side-by-side projection diff](showcase/tui-diff-range.png)

*`codect tui diff range`: a side-by-side projection diff with hunk bands.*

![codect tui diff commits — commit picker pinned as a band below the diff](showcase/tui-diff-commits.png)

*`codect tui diff commits`: the commit picker, the changed-file tree, and the selected step's diff.*

## Keybindings

| Key | Action |
|---|---|
| `q`, `Ctrl-C` | Quit |
| `↑`/`↓` or `k`/`j` | Move in the focused commit list or tree, move the source cursor, or scroll the content |
| `←`/`→` or `h`/`l` | Fold/unfold the tree, or pan the source sideways |
| `g` / `G` | Jump to the top or bottom |
| `PageUp`/`PageDown`, `Ctrl-U`/`Ctrl-D` | Move or scroll a half page |
| `Tab`, `Shift-Tab` | Cycle through Commits, Files, and Diff in commits view; switch between tree and content elsewhere |
| `Enter` | Open a file or fold a directory; in the source body, toggle the fold at the cursor |
| `za` | Toggle the fold at the cursor (same as `Enter` in the body) |
| `zo` / `zc` | Open or close the fold at the cursor |
| `zO` / `zC` | Open or close the fold and its nested regions |
| `zR` / `zM` | Unfold every region / fold every region |
| `Ctrl-P` | Open the command palette |
| `Ctrl-F` | Find a file by name |
| `/`, then `n`/`N` | Search the current view and step through matches |
| `m` | Choose types, signatures, or tests |
| `s` | Choose a scope: everything, a named area, or a literal path |
| `r` | Edit the show revision |
| `b`, `t` | Edit the diff base and target revisions |
| `[`, `]` | Shrink or grow the file tree; `\` resets it |
| `?` | Toggle help |
| `Esc` | Close an overlay, clear a search, or dismiss a diagnostic |

Every overlay action is also reachable from the command palette, so bindings
need not be memorized:

![codect command palette](showcase/tui-overlay.png)

*`Ctrl-P` command palette; keys and palette entries dispatch the same actions.*

## Folds

`tui show` is a source explorer: the body renders the real source of the
selected file with mode-driven folds.

- A retained container is open and a retained signature is folded, so the body
  shows the shape of the code without the implementations.
- `tests` mode is a filter rather than a marker: a declaration the mode drops is
  removed entirely — no row, its lines gone — and lines outside every retained
  declaration or container are hidden too, so only the retained tests and the
  containers that hold them remain. Exclusion is independent of folding, so
  `za`/`zR` cannot reveal a dropped declaration, and a file with nothing
  retained shows an empty state.
- In `types` and `signatures` a declaration the requested mode drops is folded
  whatever its kind, because the mode keeps nothing of it; its row is marked
  `hidden in <mode>`. This is what makes `types` filter out a method-only
  inherent `impl`, say, instead of leaving its body in the source.
- A collapsed row shows the declaration's own signature line — the line the
  declaration node starts on, excluding leading attributes, decorators, and doc
  comments — followed by the fold suffix. The leading annotations stay hidden
  while the fold is closed and reappear when it opens, so a `#[test]` function
  summarizes as its `fn …` line, not as `#[test]`.
- `Enter` and `za` toggle the fold at the cursor; `zo`/`zc` open or close it,
  `zO`/`zC` do so recursively, `zR` is the local "unfold all" view, and `zM`
  folds everything again.
- Opening or closing a fold is an override that is sticky for the session: it
  survives a mode reload and a selection change. Stepping to a search match
  opens every region around the match so the highlighted text is visible, and a
  later `zc`/`zM` closes it again.
- The modes are `types`, `signatures`, and `tests`.
- Folding uses declaration spans, so a body that is part of the declaration's
  span (Rust, Python, and Elm) hides fully. A Haskell top-level binding is
  separate from its signature, so the binding is not inside the outlined
  declaration and remains visible.

## Mouse

| Input | Action |
|---|---|
| Click a file | Select it; clicking a directory folds or unfolds it |
| Click a commit | Select it and show its parent-to-commit diff |
| Click the `+`/`-` gutter | Fold or unfold the declaration on that row |
| Click a source line | Move the source cursor there |
| Wheel over a pane | Scroll that pane and focus it; over the file tree it scrolls the view without moving the selection |

Mouse capture is enabled so clicks and the wheel work. That takes over the
terminal's own text selection, so to copy text hold the terminal's selection
override — usually `Shift` while dragging — or turn mouse capture off in your
terminal. Overlays stay keyboard-driven while they are open.

## Theming and icons

- `CODECT_THEME=light` switches to the light palette; `dark` is explicit, and
  anything else asks the terminal for its background color and falls back to
  dark. Colors degrade gracefully on terminals that only support 256 or 16
  colors.
- The file tree can draw Nerd Font folder and file-type glyphs with
  `--icons=nerd` (or `CODECT_ICONS=nerd`). This requires a Nerd Font installed
  in your terminal; without one the glyphs render as boxes, so the default is
  `--icons=none`.

## Requirements

- Both standard input and standard output must be terminals; a redirected
  invocation exits `1` with empty stdout and one diagnostic on stderr.
- The frontend is read-only: it reads committed blobs and never writes the
  repository, worktree, or index.
- Terminals narrower than 40 columns or shorter than 8 rows show a minimal
  "terminal too small" view; between 40 and 79 columns one focused region is
  shown at a time, and at 80 columns or wider the tree and content share the
  screen. The commits view keeps its picker band pinned at the bottom of the
  body on every width; above the band, narrow terminals show the focused pane,
  or the two stacked diff sides when Diff or Commits is focused.

## Building without the frontend

```sh
cargo build -p cli --no-default-features
```

`tui` then becomes an unknown command, and no terminal dependency is linked.

## See also

- [Using the CLI](CLI.md) — the same projections without a UI.
- [`docs/TECHNICAL_DESIGN.md`](TECHNICAL_DESIGN.md) section 21 — frontend
  architecture.
