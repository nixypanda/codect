# Codect DSH Plugin

DeepSeek Harness sidebar plugin for codect. Appears in the right sidebar and
provides two views:

- **Show** — browse the focused projection (types or signatures) of any
  revision. Mirrors the workspace-files sidebar: pick a revision, see the
  declaration outline, click to navigate.

- **Diff** — pick two commit points (BASE → TARGET) and see the focused diff.
  Mirrors the git diff sidebar: only declaration-level changes are visible;
  implementation-only changes (bodies, comments, whitespace) are stripped.

## Status

The host service, the Remote wiring, and the CLI JSON contract tests are in
place and working. The views now present the same focused experience as the TUI
and Neovim:

- **Show** renders a foldable file tree beside the selected file's canonical
  projection text (`ReadBlock`, line-numbered and highlighted). Tree rows use
  the shared folder/file-type icons and order directories first by natural,
  case-insensitive name (matching the workspace-files sidebar) with the same row
  metrics; the pane is collapsible and mouse-resizable, sitting left of the
  content at wide widths and stacked above it in a narrow sidebar. The
  declaration outline is a collapsible section above the projection. Clicking a
  declaration opens the real source file at that line through the sidebar's file
  resource.
- **Diff** renders a changed-file tree with the same icons and status badges
  beside a unified or side-by-side projection diff, with a declaration-level
  outline comparison as a secondary toggle. Both layouts render through the
  optional community `dsh-diff-view` client service (Shiki highlighting,
  intra-line word marks, real line numbers, context collapse); without it the
  body degrades to the platform `DiffBlock` (unified, plain) and then to a plain
  text dump. The two diff sides accept commits **and** the mutable snapshots
  `:index`, `:worktree`, and `:empty`; a snapshot row offers one-click
  comparisons for uncommitted (`HEAD → :worktree`), staged (`HEAD → :index`),
  unstaged (`:index → :worktree`), and whole-tree (`:empty → :worktree`)
  changes. Its tree is also collapsible and mouse-resizable, with a width
  persisted independently of Show.

Both use the shared `@deepseek-ai/dsh-client-ui-primitives` and `--dsw-*` theme
tokens, and degrade to plain markup when the primitives are unavailable. The
resizable split stores its width per view in `localStorage`. The DSH platform
findings and design notes are in
[`docs/editors/dsh.md`](../../docs/editors/dsh.md).

Side-by-side and syntax highlighting come from the optional community
`dsh-diff-view` plugin (its `diffView` client service); the views degrade
cleanly when it is absent. Deferred: the native `codect` resource protocol /
`patterns` tab types, and a "full"/source mode with semantic folds.

## Architecture

```
editors/dsh/
├── package.json            # @nixypanda/dsh-codect — bundle + client config
├── cordis.patch.yml        # Bundle patch — inserts into cordis entry list
├── tsconfig.json
├── tsconfig.build.json     # Declaration-only tsc build → lib/types
├── scripts/
│   ├── build.mjs           # esbuild build → lib/
│   └── stubs/              # Local stub declarations for the host-side tsc build
├── src/
│   ├── shared/
│   │   └── schema.ts       # codect.show.v1 / codect.diff.v1 JSON types
│   ├── api-codect/         # Host-side (Electron main process)
│   │   ├── index.ts        # cordis Service + @Remote methods
│   │   ├── typert.host.ts  # Typert host protocol (generated/hand-written)
│   │   └── typert.remote-client.ts  # Typert remote client stub
│   └── client-codect/      # Client-side (web GUI renderer)
│       ├── client.ts       # ModuleLoader entry — window.__ModuleLoader__.load()
│       ├── definition.ts   # Sidebar tab definitions (show + diff kinds)
│       ├── show-view.tsx   # Projection browser (tree + code + outline)
│       ├── diff-view.tsx   # Focused diff viewer (tree + DiffBlock)
│       ├── title.tsx       # Tab title chip
│       ├── tree.ts         # Pure file-tree model (dirs-first, natural order)
│       ├── outline.ts      # Pure declaration-outline nesting
│       ├── address.ts      # Pure `dsh-resource://file/...` address builder
│       ├── labels.ts       # Localized label bundles for the output cards
│       ├── primitives.ts   # Guarded seam to `dsh-client-ui-primitives`
│       ├── styles.ts       # Runtime stylesheet (`--dsw-*` tokens)
│       ├── use-debounced.ts # Debounce helper for the revision input
│       └── components/     # TreeView, CodePane, OutlineView, DiffBody, ModeToggle, SplitPane
├── test/
│   ├── contract.test.js    # Contract tests: codect JSON matches schema
│   └── unit.test.js        # Pure tree/address helpers (Node type stripping)
└── README.md
```

## How it fits into DSH

The plugin follows the exact patterns from the bundled `dsh-api-workspace-files`
and `dsh-client-ui-sidebar-files` packages:

| Layer | DSH package | Our package |
|---|---|---|
| Host RPC | `dsh-api-workspace-files` (cordis Service + Typert) | `api-codect/index.ts` + `typert.host.ts` |
| Host runner | `ctx.fs` / `ctx.remote` | `execFile("codect", ...)` in service |
| Client UI | `dsh-client-ui-sidebar-files` (React sidebar tab) | `client-codect/client.ts` + views |
| RPC protocol | `dsh-typert-protocol` (auto-generated) | `typert.host.ts` (hand-written) |

### Backend (host)

`api-codect/index.ts` registers a cordis `Service` named `"codect"` with
`@Remote`-decorated methods `show` and `diff`. The `static inject` declares
dependencies on the Typert service. `static Config` provides a
schemastery-validated config with a `binary` field for the codect executable
path.

### Frontend (client)

`client-codect/client.ts` uses `window.__ModuleLoader__.load()` to register an
`apply(ctx)` function that:

1. Registers two sidebar tab kinds: `"codect-show"` and `"codect-diff"`
2. Registers locale strings (English)
3. Registers the tab body slots via `ctx.slots.inject("sidebar.right.pane.tab")`
4. Registers the tab title slots via `ctx.slots.inject("sidebar.right.pane.tab.title")`
5. Registers keyboard shortcuts (`Cmd+Shift+J` / `Ctrl+Shift+J`)

## Building

```sh
cd editors/dsh
npm install
npm run build    # produces lib/
```

Or via nix:

```sh
nix build .#codect-dsh
```

## Running tests

The suite needs a built `codect` binary: the contract tests fail fast unless
`CODECT_BIN` points at one. From the repository root, `just test-dsh` builds
the binary and runs the suite against it:

```sh
just test-dsh
```

Or build and run it by hand:

```sh
cargo build -p cli
cd editors/dsh
CODECT_BIN="$PWD/../../target/debug/codect" node --test "test/*.test.js"
```

Contract tests verify that `codect show/diff --format json` output matches
the schema.ts types (including the `outline` field on `ShowFile`).

## Integration into DSH

To bundle this plugin into DeepSeek Harness:

1. **Add to DSH monorepo** — place in the packages tree
2. **Typert codegen** — run `@deepseek-ai/dsh-typert-generator` on the FaceModel
   to produce `typert.host.js` and `typert.remote-client.js`
3. **Add to `desktop-runtime.json`** — list in `sharedPackages` + `files`
4. **Activate in cordis config** — include the plugin in the loader YAML
5. **Build** — the DSH build system compiles and bundles into `app.asar`

The `cordis.patch.yml` handles step 4 automatically when the bundle is loaded.

## Codect CLI contract

The plugin speaks the codect JSON contract:

```
codect show --format json --mode <types|signatures> [--path P]... [REVISION]
codect diff --format json --mode <types|signatures> [--path P]... BASE TARGET
```

- Text output is the canonical projection, byte-for-byte stable.
- JSON is versioned: `codect.show.v1`, `codect.diff.v1`.
- Each `diff` side is a commit, `:index`, `:worktree`, `:empty`, or the
  canonical empty-tree object id. `:index` reads stage-zero blobs; `:worktree`
  reads tracked files plus untracked, non-ignored files on disk (never unsaved
  editor buffers). `show` projects a committed revision, `--stdin`, or
  `--worktree`; this plugin exposes only the committed-revision input.
- Exit 0 = success, 1 = fatal (empty stdout, stderr diagnostic), 2 = usage.
- Body-only changes produce an empty focused diff (intentional).
