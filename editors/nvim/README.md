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

## Commands

| Command | Behavior |
|---|---|
| `:OwnaiShow [types\|signatures]` | Project the current buffer via stdin and fold it to the mode (default `signatures`). |
| `:OwnaiFold [types\|signatures\|full]` | Re-fold locally with no OwnAI call. `full` unfolds everything. |
| `:OwnaiOutline` | Declaration picker via `vim.ui.select` (no plugin dependencies). |

## Folding

- Folds nest by `parent_key`: an `impl` nests its methods, a struct its fields,
  an enum its variants.
- In `types` and `signatures`, signature-kind declarations (function, method,
  value, constant, static, port, operator, foreign block) are folded and
  type/container declarations stay open. `full` folds nothing.
- A retained declaration's closed fold shows its mode-correct
  `projection.items[].canonical_text`. A declaration the mode drops is folded
  with a `kind name — hidden in <mode>` marker and the outline `signature`.
- A fold start extends upward over consecutive attributes, decorators, pragmas,
  and doc comments, which the outline span excludes.
- Expansion is sticky, keyed by `(path, stable_key)`: a declaration you open
  stays open across re-folds, refreshes, and mode switches, and one you close
  stays closed.
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
