# Planning: `ownai tui`, delivered in three usable phases

> **Status: forward-looking plan. No part of this is implemented.**
>
> `PRODUCT.md`, `TECHNICAL_DESIGN.md`, and `README.md` describe the current
> system until this work is ready. Updating them is part of the final phase and
> a merge gate, not a separate cleanup change.

## 1. Delivery principle

Build the smallest useful terminal experience first:

1. **Show MVP:** choose a projected file from a tree and read its projection.
2. **Diff MVP:** choose a changed projected file from a tree and read a usable
   side-by-side diff, including long lines that stay within their panes.
3. **Improvements:** add navigation, filtering, source views, interactive scope
   changes, polish, and optimizations only after the first two workflows work.

The Show MVP must be usable on its own before Diff MVP begins. Diff MVP must be
usable on its own before improvement work begins. Architecture is pulled into
the earliest phase that needs it; there is no standalone refactor phase.

The frontend is read-only. It reads committed blobs and never writes the
repository, worktree, or index.

## 2. Current state

- `ownai` is one binary backed by the `ownai-core`, `ownai-git`,
  `ownai-language-elm`, and `ownai-language-rust` libraries.
- The Git-aware pipeline is in `crates/ownai-cli/src/command.rs`.
- `.ownai.toml` loading is in `crates/ownai-cli/src/config.rs`.
- `ownai-core` owns projection models, canonical rendering, `unified_hunks`,
  `PathScope`, and `AreaSet`.
- Projection and blob caches are per command and are not persistent.
- There is no `ownai-engine` or `ownai-tui` crate.
- `unified_hunks` emits text; it does not expose the aligned rows needed for a
  side-by-side diff.

## 3. Decisions shared by all phases

### 3.1 Crate boundary

Add two library crates:

```text
crates/ownai-engine/
  src/
    lib.rs
    engine.rs
    config.rs
    error.rs
    selection.rs

crates/ownai-tui/
  src/
    lib.rs       // run(), TuiOptions, run loop, terminal lifecycle, driver seam
    app.rs       // Model, Msg, Cmd, update, view - the colocated TEA core
```

`ownai-engine` becomes the shared application layer over Git, core, and the
language adapters. `ownai-tui` uses the engine and core but does not parse CLI
arguments or render `miette` reports.

The `ownai-tui` TEA core — Model, Msg, Cmd, `update`, and `view` — is colocated
in `app.rs` and is pure. Only `lib.rs` is imperative: it owns the run loop,
terminal lifecycle, and event/effect interpretation. Split the crate into more
files only when readability demands it; do not pre-split.

Dependency direction:

```text
ownai-cli ──→ ownai-tui (optional) ──→ ownai-engine ──→ ownai-git
     │                 │                    │          language adapters
     ├─────────────────┴────────────────────┴────────→ ownai-core
     └───────────────────────────────────────────────→ CLI-only dependencies
```

`ownai-engine` must not depend on `clap`, `miette`, `ratatui`, or `crossterm`.
`ownai-tui` must not depend on `clap` or `miette`, discover repositories, parse
arguments, or read `.ownai.toml` directly.

### 3.2 Feature and command surface

The TUI is a default-on optional feature:

```toml
[features]
default = ["tui"]
tui = ["dep:ownai-tui"]
```

Use explicit command forms so startup state is unambiguous:

```text
ownai tui show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai tui diff --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`show` defaults to `HEAD`; both diff revisions are required. When the feature is
off, `tui` is an unknown command and `clap` exits `2`.

The CLI owns `argv` OS-path conversion and constructs the initial selection. It
discovers an `Engine`, then passes the owned engine and a plain options value to
`ownai_tui::run`, which starts the TEA runtime (section 3.6).

### 3.3 Engine contract

Names may be refined during implementation, but ownership and result behavior
must stay explicit:

```rust
pub struct Engine { /* GitRepository; no unbounded session cache */ }
pub struct Selection { /* private normalized scope and groups */ }

pub enum SelectionGroup {
    Path { label: String, path: RepoPath },
    Area { name: String, paths: Vec<RepoPath> },
}

pub struct FileDiff {
    pub path: RepoPath,
    pub old: Option<ProjectedFile>,
    pub new: Option<ProjectedFile>,
}

impl Selection {
    pub fn all() -> Self;
    pub fn new(groups: Vec<SelectionGroup>) -> Result<Self, SelectionError>;
}

impl Engine {
    pub fn discover(start: &Path) -> Result<Self, EngineError>;
    pub fn root(&self) -> &Path;
    pub fn load_areas(&self) -> Result<AreaSet, EngineError>;

    pub fn show(
        &self,
        revision: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<ProjectedFile>, EngineError>;

    pub fn diff(
        &self,
        base: &str,
        target: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<FileDiff>, EngineError>;
}
```

`Selection` fields stay private so its normalized scope cannot disagree with
its existence-check groups. `load_areas` returns an owned current snapshot;
callers decide how long to retain it.

`show` returns supported selected files in raw path-byte order, including files
with empty canonical projections. `diff` returns only paths whose canonical old
and new projections differ, also in raw path-byte order. Consequently,
implementation-only changes do not appear in the diff tree.

Selection existence is validated before any selected blob read. Each engine
operation owns and drops its blob/projection caches. Persistent caches are not
part of either MVP; they require profiling, explicit byte bounds, eviction, and
invalidation tests.

### 3.4 Errors and UI state

`SelectionError` covers invalid selection construction. `EngineError` has typed
Git, configuration, projection, and unsatisfied-selection variants retaining
applicable repository, revision(s), path, language, range, and source error.
Consumers never recover context by parsing error strings.

Initial startup failure restores the terminal and returns an error to the CLI.
After startup, state changes are transactional (section 3.6): calculate a
replacement model first, then install it. A failure preserves the last valid
screen and displays a dismissible diagnostic.

### 3.5 Terminal safety

- Verify stdin and stdout are terminals before emitting control sequences.
- Stage setup and record which steps succeeded.
- On normal return, error, or unwinding panic, show the cursor, leave the
  alternate screen, and disable raw mode in reverse order.
- Teardown is best-effort and never panics from `Drop`.
- Resize events clamp all selections and offsets, including zero-sized layouts.
- Release packaging retains panic unwinding unless another restoration design is
  added; panic-abort cannot promise cleanup.

### 3.6 Architecture: The Elm Architecture (TEA)

`ownai-tui` follows The Elm Architecture (TEA). This is a hard rule for the
frontend, not a style preference:

- **Model** — one plain data structure holds the entire UI state. It is the only
  state the runtime mutates, and it is replaced wholesale, never edited in place.
- **Msg** — every input is a value: a key, a resize, a terminal event, or the
  completion of an effect.
- **update** — a pure function `(Msg, &Model) -> (Model, Vec<Cmd<Msg>>)`. It
  performs no I/O and reads no terminal, engine, clock, or environment.
- **Cmd** — I/O is described as data. The runtime interprets each `Cmd`, and a
  completed effect produces a new `Msg`.
- **view** — a pure function `&Model -> widgets`. It never mutates the model or
  performs I/O.
- **runtime** — the only imperative layer: it reads terminal events, turns them
  into `Msg` values, calls `update`, interprets `Cmd`s, and draws `view`.

Consequences:

- Engine calls (`show`, `diff`, `load_areas`, `source`) are effects, never
  invoked from `update` or `view`.
- `update` and `view` are unit-testable without a terminal, an engine, or a
  clock.
- Because effects are data, tests assert which effect a `Msg` produced instead
  of observing side effects.
- The transactional rule of section 3.4 is expressed in TEA terms: a fallible
  effect returns a failure `Msg`, and `update` keeps the previous `Model` when a
  replacement cannot be produced.

The TEA core is colocated in `app.rs` rather than split into separate `model`,
`msg`, `update`, `cmd`, and `view` modules. Splitting it is deliberately
deferred until the file stops being readable.

Any deviation from this structure requires the project owner's explicit approval
before implementation. If a change seems to need shared mutable state, direct
engine or terminal calls inside `update`/`view`, or side effects outside a
`Cmd`, stop and ask rather than breaking the rule.

## 4. Phase 1 — Show MVP

### 4.1 Outcome

`ownai tui show` opens a useful file browser for one projected revision. A user
can move through a file tree, open a file, read its complete projection, and
quit safely.

This phase includes the minimum engine extraction because both terminal modes
need the same Git-aware projection pipeline. CLI `show` and `diff` behavior must
remain byte-for-byte unchanged during the extraction.

### 4.2 Required layout

- **Left pane:** directory/file tree derived from projected file paths.
- **Right pane:** canonical projection of the selected file.
- **Status bar:** escaped repository path, projection mode, revision, initial
  scope, selected path, and any transient diagnostic.

Files whose canonical projection is empty are hidden from the visible tree. If
the whole result is empty, show a clear empty state rather than an empty frame.
The selected file's projection supports vertical scrolling. Long lines are
clipped within the content pane and support horizontal scrolling; drawing must
never overflow into the tree or status bar.

### 4.3 Required interactions

| Key | Action |
|---|---|
| `q`, `Ctrl-C` | Quit |
| arrows or `h/j/k/l` | Move/fold tree or scroll the focused pane |
| `Tab`, `Shift-Tab` | Switch focus between tree and projection |
| `Enter` | Open a file or toggle a directory |
| `m` | Switch Types/Signatures and re-project |
| `?` | Toggle a compact help overlay |
| `Esc` | Close help or dismiss a diagnostic |

Changing mode preserves the selected path when it still has a non-empty
projection; otherwise select the nearest visible file. Interactive revision
entry is deliberately deferred. The startup `REVISION` argument is sufficient
for Show MVP.

### 4.4 Responsive behavior

- At 80 columns or wider, show tree and projection side by side.
- From 40 to 79 columns, show one focused pane at a time; `Tab` switches panes.
- Below 40 columns or 8 rows, show a minimal “terminal too small” view that still
  accepts resize, help, and quit.
- Truncate labels by display width, never by UTF-8 byte index.
- `RepoPath` remains the identity. Escape it only for display and never parse an
  escaped label back into a path.

### 4.5 Show MVP gate

- `ownai tui show` works in normal, bare, Elm-only, Rust-only, and mixed
  repositories.
- Tree ordering is raw path-byte ordering and non-UTF-8 paths render safely.
- Tests cover empty results, empty projected files, long lines, horizontal and
  vertical scrolling, mode switching, selection preservation, all responsive
  width boundaries, and zero-sized rectangles.
- A non-TTY invocation exits `1` with empty stdout and one stderr diagnostic.
- `update` and `view` are pure: tests drive `update` with `Msg` sequences and
  assert the resulting model and effects, and assert that an engine call is
  produced as a `Cmd` rather than performed during `update`.
- Injected terminal-driver tests cover failure after every setup step and all
  matching cleanup calls. A PTY smoke test runs where supported.
- Every existing CLI test retains identical stdout, stderr, and exit behavior.
- `just check` passes.
- `cargo build -p ownai-cli --no-default-features` builds and CLI `show`/`diff`
  still work.
- `cargo tree -p ownai-cli --no-default-features` contains neither `ratatui` nor
  `crossterm`, enforced by an automated check.

Do not begin Diff MVP until this gate passes.

## 5. Phase 2 — Diff MVP

### 5.1 Outcome

`ownai tui diff` opens a useful focused-diff browser. A user can choose a changed
projected file from a tree and inspect its old and new projection side by side.
The changed-file tree and readable side-by-side diff are the entire priority of
this phase.

### 5.2 Required layout

- **Left pane:** directory/file tree containing only `FileDiff` paths returned by
  the engine.
- **Right region:** old projection on the left and new projection on the right,
  with a clear divider and old/new revision labels.
- **Status bar:** repository, mode, base, target, scope, selected path, and any
  diagnostic.

Added files have an empty old side; deleted files have an empty new side. If no
focused changes exist, show a clear “no projected changes” state. The tree must
remain usable while either diff pane is focused.

### 5.3 Structured side-by-side rows

Add a structured alignment API to `ownai-core` alongside `unified_hunks`. It
returns ordered rows containing optional old/new line numbers, old/new text, and
an equal/add/delete/change classification. Structured rows and unified rendering
must share the same `similar` patience-diff configuration.

The TUI must not derive a side-by-side view by parsing unified-diff text.

### 5.4 Long-line wrapping and bounds

Long lines must never overwrite the neighboring pane, divider, tree, or status
bar. Side-by-side wrapping follows these rules:

1. Wrap old and new text independently to their pane's available content width.
2. One aligned diff row occupies the greater wrapped height of its two sides.
3. Pad the shorter side with blank visual rows so subsequent diff rows stay
   vertically aligned.
4. Repeat neither source line number on continuation rows; show a continuation
   marker instead.
5. Recompute wrapping after every resize using display width, including tabs and
   wide Unicode characters.
6. Never split a UTF-8 code point or render outside the pane buffer.

Vertical scrolling moves through visual wrapped rows while preserving the
logical aligned-row identity. Horizontal scrolling is not required in Diff MVP
because wrapping is always enabled; a wrap toggle may be added later.

### 5.5 Required interactions

Show MVP navigation remains available, with these diff-specific rules:

- Tree selection replaces the side-by-side content without rereading Git.
- `Tab` cycles tree, old pane, and new pane focus.
- Vertical movement in either diff pane is synchronized by aligned visual row.
- `m` re-runs the focused diff in the other projection mode transactionally.
- Help explains that body-only changes are intentionally absent.

Interactive base/target entry, unified view, filtering, source navigation, and
scope changes are deferred to Improvements. Startup arguments provide revisions
and scope for Diff MVP.

### 5.6 Diff MVP gate

- Added, deleted, and modified projected files appear correctly; unchanged and
  body-only changes do not appear in the tree.
- Side-by-side rows and `unified_hunks` identify the same changed lines for
  additions, deletions, replacements, empty content, and missing sides.
- Snapshot tests cover wrapping on neither/old/new/both sides, tabs, combining
  characters, wide Unicode, very narrow panes, resize reflow, and continuation
  markers.
- Tests prove no rendered cell crosses pane bounds.
- Scrolling remains synchronized across unequal wrapped heights.
- Switching files uses the already returned `FileDiff` and performs no Git read.
- Mode-change failure preserves the last valid diff and displays a diagnostic.
- Normal, bare, Elm-only, Rust-only, mixed, and no-projected-change repositories
  work end to end.
- All Show MVP gates continue to pass and `just check` passes.

Do not begin improvement work until this gate passes.

## 6. Phase 3 — Improvements

Improvements are ordered by user value. Each item should be implemented and
tested independently; none may weaken the Show or Diff MVP gates.

### 6.1 Revision controls

- Enter a new show revision.
- Enter new diff base and target revisions.
- Preserve the current file when it exists in the replacement model.
- Invalid or ambiguous revisions leave the visible model unchanged and display
  a dismissible engine diagnostic.

### 6.2 Interactive scope controls

Offer no scope, literal files/directories, UTF-8 literal path entry, and named
areas from `Engine::load_areas`. Tree-selected non-UTF-8 paths retain their
underlying `RepoPath`; they are never reconstructed from display strings.

Opening or explicitly refreshing the area chooser reloads `.ownai.toml`. A
missing or malformed config leaves the current scope unchanged. Config errors
never affect literal-path or unscoped sessions.

### 6.3 File filtering

Add a fuzzy filter over escaped display labels while retaining `RepoPath`
identity. Clearing it restores previous expansion and selection when possible.
Filtering never changes engine scope or reads Git.

### 6.4 Declaration navigation and committed source

Add next/previous declaration navigation and an internal source pane. This
requires an exact displayed row range for every selectable `stable_key`,
including nested declarations. If the current projection assembly cannot
produce these ranges structurally, extend the core model; never locate items by
substring search.

Extend the engine with:

```rust
pub struct SourceFile {
    pub path: RepoPath,
    pub text: String,
}

pub fn source(
    &self,
    revision: &str,
    path: &RepoPath,
) -> Result<SourceFile, EngineError>;
```

The source pane displays the exact committed blob and opens at the selected
item's source span. It works for old revisions and bare repositories and never
consults the worktree, invokes `$EDITOR`, or creates an editable temporary file.

### 6.5 Diff-view options

- Unified-diff toggle using `unified_hunks` without rereading Git.
- Optional no-wrap mode with per-pane horizontal scrolling.
- Toggle linked/unlinked horizontal offsets in no-wrap mode.
- Whitespace visualization, only if it does not alter diff semantics.

### 6.6 Performance work

Keep operations synchronous while they remain responsive. Measure input-to-
redraw latency on the existing representative repositories and a larger
synthetic repository. Record best/median/p95, host, build profile, file count,
revision shape, and packed-object state.

If a projection action exceeds 100 ms median or 200 ms p95 on the documented
reference host, add a cancellable worker and loading state before adding
persistent caches. Persistent caches require separate profiling, byte bounds,
eviction, and invalidation tests.

### 6.7 Documentation and final gate

Before merging the completed work:

- Update `TECHNICAL_DESIGN.md` with the actual workspace, engine boundary, TEA
  model/update/view boundary, structured diff rows, terminal lifecycle, cache
  policy, testing, security, and implementation sequence.
- Update `README.md` with installation defaults, both `tui` command forms,
  keybindings, terminal requirements, and the no-default-features build.
- Update `PRODUCT.md` to promote only implemented terminal behavior into current
  scope and leave unfinished improvements as later features.
- Audit `ratatui`/`crossterm` features and confirm the approved `ownai-git`
  feature tree is unchanged.
- Snapshot default and no-default `--help` output.
- Run every Show MVP and Diff MVP gate under the final dependency graph.

The plan is complete only when authoritative documentation describes the code in
the same change and this plan is marked completed or moved to the project's
completed-plan convention.

## 7. Testing summary

- **Engine:** temporary Git repositories cover ordering, empty projections,
  focused-diff filtering, selection validation, errors, and cache boundaries.
- **State:** pure `update` tests drive `Msg` sequences and assert the resulting
  model and effects; failed operations do not replace the last valid model, and
  engine calls appear only as `Cmd`s.
- **Rendering:** `ratatui::TestBackend` covers file trees, empty states, help,
  diagnostics, responsive layouts, and side-by-side wrapping.
- **Terminal:** an injected driver covers partial setup and cleanup; a PTY smoke
  test covers the real crossterm path where supported.
- **Feature graph:** automated default/no-default checks prove terminal
  dependencies do not leak into the latter build.
- **Safety:** no test launches an editor, writes repository data, or depends on
  the developer's terminal configuration.
