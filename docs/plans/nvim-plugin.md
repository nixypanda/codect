# Plan: OwnAI Neovim plugin — semantic fold viewer

> Status: **Phase 0, Phase 1, Phase 2, and the Nix packaging are implemented.**
> `ownai show --format json` emits the versioned `ownai.show.v1` document for
> committed, stdin, and worktree input, with a mode-independent declaration
> outline. The schema is committed at `docs/schema/ownai.show.v1.json`, golden
> documents live under `fixtures/schema/`, and the CLI contract is documented in
> `TECHNICAL_DESIGN.md` section 14.2.
>
> The Neovim plugin (Phase 2) lives under `editors/nvim/` and the flake packages
> it as `packages.<system>.ownai-nvim`, with `just test-nvim` and a
> `checks.<system>.ownai-nvim` flake check. The diff slice (Phase 3) remains
> future work.
>
> Owner intent: a Neovim plugin that shows the *entire* codebase but folds
> details according to OwnAI's projection mode (Types / Signatures), letting the
> user expand any fold to real code. Show first; diff second.

## 1. Decisions already made

| Decision | Choice |
|---|---|
| Fold model | **Source buffer + semantic folds.** The buffer holds real source; OwnAI supplies the declaration/fold map. Expanding reveals real code. |
| Diff buffer (later phase) | **Git source diff + OwnAI fold boundaries.** Git does the text diff; OwnAI supplies semantic fold boundaries and the changed-item map. |
| First slice | **Show first** (`:OwnaiShow`), not diff. |
| Show buffer content | **Working-tree file** (editable), not a committed-blob scratch buffer. |
| Fold-map delivery | **Extend `ownai show --format json`** with a mode-independent outline. |
| Plugin home | **Separate repo `ownai.nvim`** eventually; develop in-repo under `editors/nvim/` until `ownai.show.v1` freezes, then split. Schema is owned and versioned by this repo. |

### 1.1 The unifying trick

`foldtext()` returns the declaration's closed-fold text: the matching
`projection.items[].canonical_text` when the requested mode retains the
declaration, or `outline.signature` (the full Signatures form) when it does not.
A real-source buffer with all folds closed at Types depth then renders like
`ownai show --mode types`, while every fold opens to real code. Show and the
fold view become one buffer at two fold depths.

### 1.2 The constraint this design works around

OwnAI reads **committed blobs only** on every historical path. A working-tree
buffer cannot be folded from `show <rev> --format json` because spans would come
from HEAD and drift as the user types. Fix: project on-disk or stdin bytes. This
is cheap because the language crates are pure
(`LanguageProjector::project(ProjectionInput { path, source, mode })`) and
`ownai-core` is Git-free.

This changes a stated product invariant ("reads committed blobs and never writes
the repository, worktree, or index"). OwnAI now **reads** the worktree on this
path. It is still read-only; the product docs say so explicitly, and the new
surface is incapable of writing.

## 2. Phase 0 — Documentation

Deliverables:

1. This plan.
2. Update `docs/PRODUCT.md`: an editor-experience section describing
   working-tree projection, semantic folds, and the read-only guarantee.
3. Update `docs/TECHNICAL_DESIGN.md`: the `ownai.show.v1` schema, the
   stdin/worktree projection path, and where it lives in the crate graph.
4. Commit `docs/schema/ownai.show.v1.json` (JSON Schema) plus golden fixtures
   under `fixtures/schema/`.

Acceptance: docs describe the feature in the same change as the code; the schema
file is committed and referenced by both the CLI tests and the plugin.

## 3. Phase 1 — Core interface

Goal: `ownai show --format json` returns a stable, versioned document for both
committed and working-tree input, and it is verified against the schema.

### 3.1 CLI surface

```text
ownai show --format <text|json> --mode <types|signatures> [REVISION]
ownai show --format json --mode <types|signatures> --stdin --path <PATH>
ownai show --format json --mode <types|signatures> --worktree --path <PATH>
```

Rules:

- `--format` defaults to `text`; existing output is byte-for-byte unchanged.
- `--stdin` reads source bytes from stdin; `--path` supplies language detection
  **and** the repo-relative path used to build stable keys. `--stdin` without
  `--path` is a usage error (exit 2).
- `--worktree` reads the file at `--path` from disk. `--stdin` is the primary
  editor path (handles unsaved buffers, no temp files, no races).
- `--stdin`/`--worktree` are mutually exclusive with `REVISION` and with
  `--area`; `--path` must resolve inside the repository, exactly one `--path` is
  required, and it must name a single file — a directory (including `.` or the
  repository root) is a usage error (exit 2). `--worktree` refuses a symlinked
  target and checks the fully-resolved path stays inside the repository
  (exit 1).
- All new input paths are read-only. They never write the repository, worktree,
  or index.
- Non-UTF-8 source, unsupported extensions, and out-of-repo paths fail with
  typed diagnostics (exit 1), consistent with existing error behavior.

### 3.2 JSON document (`ownai.show.v1`)

```jsonc
{
  "schema": "ownai.show.v1",    // a consumer treats any other value as fatal
  "input": "revision",           // "revision" | "stdin" | "worktree"
  "revision": "HEAD",            // string, or null for stdin/worktree
  "mode": "types",               // "types" | "signatures"
  "files": [                     // one or more, raw path byte order for revisions
    {
      "path": "src/auth.rs",
      "language": "rust",
      "projection": {
        "text": "<the exact canonical text this file would emit in text mode>",
        "items": [ { "stable_key", "parent_key", "kind", "name", "span", "canonical_text" } ]
      },
      "outline": [                 // mode-INDEPENDENT, every declaration
        {
          "stable_key": "impl Session::method::refresh",
          "parent_key": "impl Session",   // null for top-level
          "kind": "method",
          "name": "refresh",
          "span": { "start_line": 40, "end_line": 58,
                    "start_byte": 1024, "end_byte": 1580 },
          "signature": "fn refresh(&mut self, token: Token) -> Result<(), Error>;",
          "retained_in_mode": true
        }
      ]
    }
  ]
}
```

Contract rules:

- `outline` is complete regardless of mode: Types can drop an entire `impl`
  block or a function, so projected items alone cannot locate folds. It is
  derived from the Signatures projection (the superset); `retained_in_mode` is
  set by `stable_key` membership.
- The closed-fold text depends on `retained_in_mode`. When the requested mode
  retains the declaration, use the matching `projection.items[].canonical_text`
  (mode-correct). When it does not, use `signature`, which is the declaration's
  canonical fragment from the Signatures superset and is intentionally the
  superset form. They differ for container declarations (trait/impl/module):
  a trait implementation Types mode keeps with only its associated types has a
  `signature` that also shows the methods Signatures adds. A nested declaration's
  fragment carries its container indentation. Single line where the canonical
  form is single line.
- `projection.text` equals the canonical text used by text mode for that file.
- `span` carries both lines and bytes. **Line values are ONE-based on the wire**
  for editor friendliness — the JSON layer adds one to the zero-based adapter
  span — and byte offsets are zero-based into the decoded UTF-8 source. The
  plugin does not convert line numbers; it converts byte offsets only if needed.
- `span` starts at the declaration node, so preceding attributes, decorators,
  `{-# ... #-}` pragmas, and doc comments are excluded even though `signature`
  may include them. Extend a fold start upward over those lines. A
  `decorator_start_line` field may be added additively within `ownai.show.v1`
  later; there is none today.
- `stable_key` values for identical content are byte-identical between the
  committed-blob path and the stdin/worktree path.
- `stable_key` is unique within its file but not necessarily across the
  repository: Rust `impl` keys are not path-namespaced, so key global state
  (expanded folds, cursors) by `(path, stable_key)`.
- Unknown fields must be tolerated by consumers, and fields may be added
  additively within `ownai.show.v1`; a schema version mismatch is fatal and
  explicit.
- `kind` values are exhaustive over `ItemKind` with no wildcard arm, so a new
  kind is a compile error.

### 3.3 Engine / crate changes

- `ownai-engine`: `project_source(RepoPath, bytes, mode)` assembles the outline
  and projection for editor-supplied bytes without a repository; `Engine::show_outlines`
  does the same for a committed revision. Both reuse the existing
  `LanguageProjector`.
- `ownai-cli`: `--format`, `--stdin`, `--worktree`, and JSON serialization.
  JSON types live in the CLI (`src/json.rs`), not in `ownai-core`, keeping core
  file-format-free.
- `ownai-core`: no Git or serialization dependency added. The outline is built
  from the existing `ProjectedItem` model, so no core type changed.

### 3.4 Phase 1 acceptance

- `ownai show --format json --stdin --path src/x.rs --mode types < file.rs`
  returns schema-valid JSON with a complete `outline` and a mode-correct
  `projection`.
- The same content via a committed revision produces identical `stable_key`
  values and identical `outline` structure (modulo `retained_in_mode`, which is
  mode-dependent).
- Golden fixtures committed; a test validates them against the JSON Schema.
- A corpus test proves the Signatures projection is a superset of Types by
  `stable_key`.
- Existing `show`/`diff` text output and exit codes are unchanged.
- `just check` passes.

## 4. Phase 2 — Plugin (`:OwnaiShow`)

Status: **implemented.** The plugin lives under `editors/nvim/`, with a
no-dependency headless test harness.

Goal: open a working-tree file, fold it by mode, and manipulate folds. Verified
in Neovim.

### 4.1 Module layout

```text
editors/nvim/                     # later: ownai.nvim
  lua/ownai/
    init.lua        # setup(), command + keymap registration
    cli.lua         # invoke ownai, parse JSON, schema check
    outline.lua     # build the fold tree from outline items
    folds.lua       # foldmethod=expr, foldexpr, foldtext
    state.lua       # expanded set keyed by stable_key; mode per buffer
    view.lua        # :OwnaiShow / :OwnaiFold / :OwnaiOutline
    health.lua      # :checkhealth ownai (binary + schema version)
  plugin/ownai.lua  # lazy command declarations
  tests/            # no-dependency headless harness (run.lua)
```

### 4.2 Commands and keys

| Command / key | Behavior |
|---|---|
| `:OwnaiShow [mode]` | Project the current buffer via stdin; fold to `mode` (default `signatures`). Attaches the buffer keymaps. |
| `:OwnaiFold types\|signatures\|full` | Re-fold the current buffer to `mode`. Reuses the cached projection for the current bytes and path; fetches one on a cache miss (mode or file changed). Attaches the buffer keymaps. |
| `:OwnaiOutline` | Picker of declarations from the outline. |
| `:OwnaiEnable [types\|signatures]` | Turn global auto-fold on and fold the current buffer (default `signatures`); new files fold on read. |
| `:OwnaiDisable` | Turn global auto-fold off and unfold every buffer the plugin folded, without a binary call. |
| `:OwnaiToggle [types\|signatures]` | Toggle global auto-fold. |
| `]f` / `[f` | Next/previous declaration. |
| `zr` / `zm` | Not persistent: each apply resets `foldlevel` to open every fold before re-applying the mode's open/closed state. Use `:OwnaiFold` or the cycle key to change depth. |

### 4.3 Fold behavior

- Build the fold tree from `parent_key` so `impl` nests `fn`, `struct` nests
  fields.
- `foldmethod=expr` with a cached map; `foldexpr` reads the map, `foldtext`
  returns the declaration's mode-correct closed-fold text: the matching
  `projection.items[].canonical_text` when the mode retains it, else the
  outline's `signature`.
- Mode is a fold-depth target: Types closes signatures, Signatures opens them,
  Full opens everything.
- Expansion is sticky by `stable_key`; refresh on `InsertLeave`, debounced
  `TextChanged`, and `BufWritePost`, then restore the expanded set.
- One OwnAI call per (content, mode, path) change. `:OwnaiFold` reuses the
  cached document when the bytes, mode, and path all match and fetches one
  otherwise. Never per keystroke.
- Each apply resets `foldlevel` to "everything open" and recomputes folds
  before re-applying the mode's open/closed state, so native `zr`/`zm` depth
  does not persist and a repeated apply cannot collapse a retained container.
- The window-local fold options the plugin replaces are saved and restored
  when the buffer leaves the window.
- Global auto-fold is a persistent toggle (`:OwnaiEnable` / `:OwnaiDisable` /
  `:OwnaiToggle`, or `require("ownai").enable/disable/toggle/is_enabled`). While
  on it folds the current buffer immediately and every other file buffer as it
  enters a window (`BufWinEnter`) or is read (`BufReadPost`); disabling unfolds
  every buffer the plugin folded locally, with no binary call. Ineligible
  buffers (non-file, unnamed, directory, outside a Git repository) and
  unsupported languages are skipped silently; a genuine setup failure warns at
  most once. The flag lives in the plugin module, so `setup()` re-invocation
  preserves it.

### 4.4 Phase 2 acceptance

- `:OwnaiShow types` on a Rust file shows only type declarations, each closed
  fold rendering OwnAI's canonical fragment; opening a fold reveals the real
  body.
- `:OwnaiShow signatures` additionally opens signatures.
- `:OwnaiFold` re-folds to a mode, reusing the cached projection when the
  bytes, mode, and path match and fetching one otherwise.
- Expanding a fold, editing the body, and re-running preserves expansion.
- Works for all four languages on the fixture corpus.
- `:checkhealth ownai` reports binary path and schema compatibility.
- Headless test: pipe a fixture file, assert fold lines and foldtext.

## 5. Phase 3 (later) — Diff slice

Out of scope for this execution, recorded so Phase 1/2 choices stay compatible:

- `ownai diff --format json` exposing item diffs with spans on both sides.
- Plugin uses git's source diff as buffer content; OwnAI supplies fold
  boundaries and the changed-item map; changed declarations auto-unfold.
- Mode-driven fold depth for review; semantic `]c`/`[c` navigation.
- Sticky fold state keyed by `stable_key` across mode switches and re-runs.

## 6. Repo strategy

- **Confirmed: develop under `editors/nvim/` in this repo** while
  `ownai.show.v1` is unstable, so one commit can change the engine and the
  plugin together.
- Own and version the schema here (`docs/schema/`), with golden fixtures.
- Once the schema freezes, split `editors/nvim/` into `ownai.nvim` (e.g.
  `git subtree split`) and have it pin a schema version range and validate at
  startup.

## 7. Nix packaging (flake)

Status: **implemented.** `packages.<system>.ownai-nvim` builds via
`vimUtils.buildVimPlugin`, `pkgs.neovim` is in the dev shell, `just test-nvim`
runs the headless suite, and `checks.<system>.ownai-nvim` runs it in a
throwaway repository.

Goal: a Nix user can install the plugin and the binary from this flake with no
manual cloning, and plugin development/testing runs in `nix develop`.

### 7.1 Flake outputs

Add to `flake.nix`:

```nix
ownaiNvim = pkgs: pkgs.vimUtils.buildVimPlugin {
  pname = "ownai.nvim";
  version = "0.1.0";
  src = ./editors/nvim;   # lua/ and plugin/ land at the store root
  meta.description = "OwnAI semantic fold viewer for Neovim";
};
```

- `packages.<system>.ownai-nvim` — the plugin derivation.
- `packages.<system>.ownai` already exists; keep it the source of the binary.
- Optional `overlays.default` exposing `ownai` and `ownai-nvim`, so consumers
  can pull both from one overlay.

Consumer usage (home-manager shown; NixOS is equivalent):

```nix
programs.neovim.plugins = [ pkgs.ownai-nvim ];
# the plugin needs the binary on PATH:
programs.neovim.extraPackages = [ pkgs.ownai ];
```

or with the overlay:

```nix
programs.neovim.plugins = [ pkgs.ownai-nvim ];
environment.systemPackages = [ pkgs.ownai ];
```

### 7.2 Binary discovery

`buildVimPlugin` cannot wrap a binary, so the plugin resolves `ownai` itself, in
order:

1. `vim.g.ownai_binary` (explicit override),
2. `OWNAI_BIN` environment variable,
3. `ownai` on `PATH`.

The flake package must not hardcode a store path into Lua. Document
`extraPackages = [ pkgs.ownai ]` as the supported pairing. `:checkhealth ownai`
reports the resolved path and the binary's reported schema version, so a
mismatch is diagnosable.

### 7.3 Development shell and checks

- Add `pkgs.neovim` to `devShells.default.packages` so plugin tests run in
  `nix develop`.
- Use `pkgs.vimPlugins.plenary-nvim` for headless tests (already in nixpkgs; no
  new flake input). If a no-dependency harness is preferred, tests can run via
  `nvim --headless -u NONE -l`, but plenary gives cleaner assertions.
- Add a `just` recipe, e.g. `test-nvim`, running the headless plugin suite
  against a fixture file and the built `ownai` binary.
- Optionally add a flake `checks.<system>.ownai-nvim` that runs the headless
  suite, so `nix flake check` covers the plugin.
- `format-nix` (`nix fmt`) must stay green; keep `flake.nix` changes
  `nixfmt-tree`-clean.

### 7.4 Packaging acceptance

- `nix build .#ownai-nvim` produces a derivation containing `lua/` and
  `plugin/`.
- In a shell with `pkgs.ownai` on `PATH`, `:OwnaiShow types` works on a fixture
  file.
- `nix develop` provides `neovim`; `just test-nvim` passes.
- `nix fmt --check` / `just check` stay green.

## 8. Open questions

1. `--stdin` vs `--worktree` as the primary input — plan assumes `--stdin`
   primary, `--worktree` as a fallback.
2. Behavior for untracked files and buffers outside the repository.
3. Whether `outline` needs extraction changes in Elm/Haskell/Python for nested
   items (Rust already emits them). Phase 1 answered this: all four adapters
   already emit nested items, so no extraction change was required.
4. Fold presentation for declarations that Types mode drops entirely (e.g. an
   inherent `impl` with no type info) — folded placeholder vs omitted. The
   outline now marks them with `retained_in_mode: false`.
5. `decorator_start_line` (leading attributes and doc comments) was dropped from
   this slice. Resolved in Phase 2 without a schema change: the plugin extends
   each fold start upward over complete leading annotation runs (line comments,
   block comments, pragmas, balanced attributes, decorators) using the buffer
   text. The field can still be added additively within `ownai.show.v1` if a
   consumer wants the boundary from the engine.
