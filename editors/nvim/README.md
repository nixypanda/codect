# ownai.nvim

Semantic fold viewer for [OwnAI](../../README.md). The buffer holds real
source; OwnAI supplies the declaration map and the mode-correct closed-fold
text. Expanding a fold reveals real code.

This plugin lives in the OwnAI repository while `ownai.show.v1` is unstable and
is intended to split into its own `ownai.nvim` repository later.

## Requirements

- Neovim 0.10 or newer (the plugin uses `vim.system`, `vim.json`, `vim.ui.select`).
- The `ownai` binary. It is packaged separately; the plugin cannot bundle it.

The plugin resolves the binary in this order:

1. `vim.g.ownai_binary`
2. `OWNAI_BIN`
3. `ownai` on `PATH`

With Nix, pair the plugin with the binary:

```nix
programs.neovim.plugins = [ pkgs.ownai-nvim ];
programs.neovim.extraPackages = [ pkgs.ownai ];
```

## Setup

```lua
require("ownai").setup({
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
window-local fold options for OwnAI buffers: `foldmethod=expr`, `foldexpr`,
`foldtext`, `foldenable`, `foldminlines=0`, and a `foldlevel` reset on every
apply. Because it resets `foldlevel`, native `zr`/`zm` depth keys do not
persist; use `:OwnaiFold`, `za`/`zo`/`zc`, or the cycle key instead. The
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

## Commands

| Command | Behavior |
|---|---|
| `:OwnaiShow [types\|signatures]` | Project the current buffer via stdin and fold it to the mode (default `signatures`). Attaches the buffer keymaps. |
| `:OwnaiFold [types\|signatures\|full]` | Fold to the mode. Reuses the cached projection for the current bytes and path; fetches one only when the mode is not cached or the buffer's file changed. Attaches the buffer keymaps. `full` unfolds everything. |
| `:OwnaiOutline` | Declaration picker via `vim.ui.select` (no plugin dependencies). |
| `:OwnaiEnable [types\|signatures]` | Turn global auto-fold on and fold the current buffer (default `signatures`). |
| `:OwnaiDisable` | Turn global auto-fold off and unfold every buffer the plugin folded. |
| `:OwnaiToggle [types\|signatures]` | Toggle global auto-fold (default `signatures`). |

## Auto-fold mode

Auto-fold is one persistent toggle. While it is on, every file buffer you open
is folded to the chosen mode; while it is off, new files open normally and the
buffers the plugin folded are returned to normal.

```lua
require("ownai").enable("types") -- omit the mode to use default_mode
require("ownai").is_enabled()    -- boolean
require("ownai").disable()
require("ownai").toggle("signatures")
```

- `enable(mode?)` turns it on, validates that `mode` is `types` or `signatures`
  (`full` is rejected), folds the current buffer immediately, then folds every
  other buffer as it is next entered (`BufWinEnter`) or read (`BufReadPost`).
  Ineligible buffers (not a file, unnamed, a directory, outside a Git
  repository) and files whose language the binary does not project are skipped
  silently; a genuine setup failure such as a missing binary or a schema
  mismatch warns at most once per enable.
- `disable()` turns it off and unfolds every buffer the plugin had folded by
  switching it to `full` locally — no binary call.
- `toggle(mode?)` disables when on, otherwise enables.
- The state lives in the plugin module, not in your config, so re-running
  `setup()` does not reset it.

Auto-fold uses the same read-only projection pipeline as `:OwnaiShow`. The
per-buffer `:OwnaiShow` and `:OwnaiFold` keep working while the toggle is on;
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
  `require("ownai").clear_overrides()` to drop them all.
- Folds refresh on `BufWritePost`, `InsertLeave`, and a debounced `TextChanged`.
  The CLI is never invoked per keystroke.

The buffer is only read. The plugin never modifies buffer text, the worktree,
or the index.

## Health

`:checkhealth ownai` reports the resolved binary, its version, the expected
schema, and a live projection probe.

## Tests

```sh
just test-nvim
```

The suite runs headlessly without extra plugins:

```sh
OWNAI_BIN=target/debug/ownai nvim --headless -u NONE -l editors/nvim/tests/run.lua
```
