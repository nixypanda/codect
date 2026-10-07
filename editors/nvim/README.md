# codect.nvim

Semantic fold viewer for [Codect](../../README.md). The buffer holds real
source; Codect supplies the declaration map and the mode-correct closed-fold
text. Expanding a fold reveals real code.

This plugin lives in the Codect repository while `codect.show.v1` is unstable and
is intended to split into its own `codect.nvim` repository later.

## Requirements

- Neovim 0.10 or newer (the plugin uses `vim.system`, `vim.json`, `vim.ui.select`).
- The `codect` binary. It is packaged separately; the plugin cannot bundle it.

The plugin resolves the binary in this order:

1. `vim.g.codect_binary`
2. `CODECT_BIN`
3. `codect` on `PATH`

With Nix, pair the plugin with the binary:

```nix
programs.neovim.plugins = [ pkgs.codect-nvim ];
programs.neovim.extraPackages = [ pkgs.codect ];
```

## Setup

```lua
require("codect").setup({
  default_mode = "signatures", -- used when a command omits its argument
  debounce_ms = 200,           -- TextChanged refresh debounce
  manage_fold_options = true,  -- set window-local fold options (see below)
  keymaps = {
    enabled = true,            -- false disables every binding
    next_declaration = "]f",
    prev_declaration = "[f",
    cycle_mode = "<leader>of", -- types -> signatures -> full -> types
    fold_toggle = "za",
    fold_open = "zo",
    fold_close = "zc",
    fold_open_recursive = "zO",
    fold_close_recursive = "zC",
  },
})
```

`setup` is optional: commands work with the defaults. Set any individual
binding to `false` or `""` to leave it unregistered.

With `manage_fold_options = true` (the default) the plugin owns the
window-local fold options for Codect buffers: `foldmethod=expr`, `foldexpr`,
`foldtext`, `foldenable`, `foldminlines=0`, and a `foldlevel` reset on every
apply. Because it resets `foldlevel`, native `zr`/`zm` depth keys do not
persist; use `:CodectFold`, `za`/`zo`/`zc`, or the cycle key instead. The
previous window-local fold options are saved and restored when the buffer
leaves the window, so they do not leak into the next buffer.

Set `manage_fold_options = false` to keep your own fold configuration
(`foldmethod`, `foldexpr`, `foldtext`, `foldenable`, `foldminlines`). The plugin
still applies its open/closed state, and it still resets `foldlevel` and forces
a fold recompute so the apply starts from "everything open" and cannot collapse
a retained container. This option is only useful if the window is using the
plugin's `foldexpr` (or you drive `foldexpr` yourself); without semantic folds
there is nothing for the plugin to open or close.

`default_mode` must be `types` or `signatures`. `setup` warns and falls back to
`signatures` otherwise.

The plugin's mode vocabulary is its own, independent of the binary's `--mode`. The
binary also has a `tests` mode (Signatures restricted to test declarations,
recognized for Rust and Python only) which this plugin does not expose: adding it
means deciding where it sits in the `types -> signatures -> full` cycle and how a
buffer folds when most declarations are not retained.

## Commands

| Command | Behavior |
|---|---|
| `:CodectShow [types\|signatures]` | Project the current buffer via stdin and fold it to the mode (default `signatures`). Attaches the buffer keymaps. |
| `:CodectFold [types\|signatures\|full]` | Fold to the mode. Reuses the cached projection for the current bytes and path; fetches one only when the mode is not cached or the buffer's file changed. Attaches the buffer keymaps. `full` unfolds everything. |
| `:CodectOutline` | Declaration picker via `vim.ui.select` (no plugin dependencies). |
| `:CodectEnable [types\|signatures]` | Turn global auto-fold on and fold the current buffer (default: the last enabled mode, else `default_mode`). |
| `:CodectDisable` | Turn global auto-fold off and unfold the buffers auto-fold folded. An explicitly folded buffer keeps its folds only until auto-fold also folds it; from then on `disable()` unfolds it too. |
| `:CodectToggle [types\|signatures]` | Toggle global auto-fold (default: the last enabled mode, else `default_mode`). |

## Auto-fold mode

Auto-fold is one persistent toggle. While it is on, every file buffer you open
is folded to the chosen mode; while it is off, new files open normally and the
buffers the plugin folded are returned to normal.

```lua
require("codect").enable("types") -- omit the mode to use default_mode
require("codect").is_enabled()    -- boolean
require("codect").disable()
require("codect").toggle("signatures")
```

- `enable(mode?)` turns it on, validates that `mode` is `types` or `signatures`
  (`full` is rejected), folds the current buffer immediately, then folds every
  other buffer as it is next entered (`BufWinEnter`) or read (`BufReadPost`).
  `mode` defaults to the mode the last enable used, so `disable()` followed by
  `enable()` returns to the same depth; `default_mode` applies only before the
  first enable of the session. Ineligible buffers (not a file, unnamed, a
  directory, outside a Git repository) and files whose language the binary does
  not project are skipped silently; a genuine setup failure such as a missing
  binary or a schema mismatch warns at most once per enable.
- `disable()` turns it off and unfolds every buffer auto-fold folded by
  switching it to `full` locally — no binary call. A buffer you folded yourself
  with `:CodectShow`/`:CodectFold` keeps its folds only while auto-fold has not
  also folded it: auto-fold re-folds a buffer to the active mode when it is
  entered, and once that happens the buffer counts as auto-folded and
  `disable()` unfolds it too. A buffer auto-fold never touched is left as you
  left it.
- `toggle(mode?)` disables when on, otherwise enables. It returns whether
  auto-fold is on afterwards, so an invalid `mode` leaves it off.
- The state lives in the plugin module, not in your config, so re-running
  `setup()` does not reset it. It is session-scoped: nothing is persisted, so a
  Neovim restart starts disabled with no remembered mode.

A non-fatal auto-fold failure (for example an unsupported language inside a Git
repository) is remembered per `(path, content hash, mode)`, so re-entering an
unchanged buffer does not re-run `git rev-parse` and the CLI. The memory is
dropped when the content changes, the buffer is wiped, or a new `enable()`
starts.

Auto-fold uses the same read-only projection pipeline as `:CodectShow`. The
per-buffer `:CodectShow` and `:CodectFold` keep working while the toggle is on;
they set that buffer's depth until it is next entered.

## Folding

- Folds nest by `parent_key`: an `impl` nests its methods, a struct its fields,
  an enum its variants.
- In `types` and `signatures`, signature-kind declarations (function, method,
  value, constant, static, port, operator, foreign block) are folded and
  type/container declarations stay open. `full` folds nothing. Single-line
  declarations fold too.
- A retained declaration's closed fold shows its mode-correct
  `projection.items[].canonical_text`. A declaration the mode drops is folded
  with a `kind name — hidden in <mode>` marker and the outline `signature`.
- A fold start extends upward over complete leading annotations: line comments
  (`///`, `//!`, `//`, `--`, `#`), block comments (`/** */`, `/* */`, `/*! */`,
  `{-| -}`, `{- -}`), pragmas (`{-# #-}`), balanced attributes (`#[ ... ]`), and
  decorators (`@...`, including parenthesized/continued forms). Nested blocks
  and blank lines *inside* a block or continued decorator are folded as one
  run; a blank line *between* separate runs still ends it. Bracket and
  parenthesis counting ignores string literals, so `#[doc = "a ] b"]` balances.
  A trailing comment (`//`, `#`, `--`, `/*`, `{-`) that directly follows another
  declaration stays with that declaration; a forward-attaching form (`///`,
  `//!`, `/**`, `/*!`, `{-|`, `{-#`, `#[`, `#![`, `@…`) may cross into the
  preceding declaration's span. A `#!` shebang is never claimed.
- Expansion is sticky, keyed by `(path, stable_key)`: a declaration you open
  stays open across re-folds, refreshes, and mode switches, and one you close
  stays closed. Overrides are dropped when the buffer is wiped; call
  `require("codect").clear_overrides()` to drop them all.
- Folds refresh on `BufWritePost`, `InsertLeave`, and a debounced `TextChanged`.
  The CLI is never invoked per keystroke.

The buffer is only read. The plugin never modifies buffer text, the worktree,
or the index.

## Diffview integration

When Diffview is installed, `:DiffviewFileHistory` and `:DiffviewOpen`
views start in Codect's `default_mode`. Their file lists omit
files whose canonical projections are equal, and their diff panes show only
projected Types or Signatures. A body-only history commit stays in the commit
list with Diffview's `No diff` row. The existing Diffview open, close, file
history, and range commands need no replacement mappings.

The ordinary `:DiffviewOpen` keeps Diffview's staged (HEAD versus index) and
unstaged (index versus worktree) sections. Focused panes use separate read-only
scratch buffers, so switching modes and closing the view leave editable index
and worktree buffers intact. Index and worktree projections are refreshed on
Diffview file-list updates. Git's file list tracks changes on disk and in the
index; unsaved changes that exist only in an editor buffer do not appear until
written.

Use `:CodectDiffview types`, `:CodectDiffview signatures`, or
`:CodectDiffview source` in the current Diffview tab. History refreshes preserve
the selected commit and file when they still exist. The focused panes are
read-only scratch buffers; source mode restores Diffview's normal content.

Diffview has no public provider for its file list and pane contents. This
adapter is guarded by source hashes for Diffview commit
`4516612fe98ff56ae0415a259ff6361a89419b0a`. Pin that revision when using
the integration. If Diffview's internal files differ, Codect warns once and
leaves Diffview unmodified. The integration is loaded automatically when a
Diffview tab opens, including when Diffview is lazy loaded after Codect.

The linked dotfiles config obtains Diffview from nixpkgs. Its `flake.lock`
revision `18dd725c29603f582cf1900e0d25f9f1063dbf11` resolves
`vimPlugins.diffview-nvim.src.rev` to this exact Diffview commit. Add
`editors/nvim` to Neovim's plugin runtimepath and source its `plugin/codect.lua`
to activate the integration there; the five existing Diffview mappings and
their `after = function() require("diffview").setup({}) end` callback need no
changes. The dotfiles configuration does not currently install Codect itself.

The `codect.diff.v1` CLI accepts commits, `:index`, `:worktree`, and Git's empty
tree. Root commits therefore show focused added-file projections. Rename rows
currently use Diffview's destination path; rename-aware matching and a separate
old-path row are not provided. Unmerged index entries have no stage-zero
snapshot mapping in this integration, so conflicting rows are omitted from
focused file lists.

## Health

`:checkhealth codect` reports the resolved binary, its version, the expected
schema, and a live projection probe.

## Tests

```sh
just test-nvim
```

The suite runs headlessly without extra plugins:

```sh
CODECT_BIN=target/debug/codect nvim --headless -u NONE -l editors/nvim/tests/run.lua
```

With the pinned Diffview checkout, run the integration cases separately:

```sh
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim CODECT_TEST_DIFFVIEW_CASE=history \
  CODECT_BIN="$(pwd)/target/debug/codect" nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim CODECT_TEST_DIFFVIEW_CASE=range \
  CODECT_BIN="$(pwd)/target/debug/codect" nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim CODECT_TEST_DIFFVIEW_CASE=local \
  CODECT_BIN="$(pwd)/target/debug/codect" nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua
CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim CODECT_TEST_DIFFVIEW_CASE=root \
  CODECT_BIN="$(pwd)/target/debug/codect" nvim --headless -u NONE -l editors/nvim/tests/test_diffview.lua
```

## License

This plugin is part of Codect and is licensed under the GNU Affero General
Public License, version 3 or (at your option) any later version
(AGPL-3.0-or-later). See the repository [LICENSE](../../LICENSE) for the full
text.
