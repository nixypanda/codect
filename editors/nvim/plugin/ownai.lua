-- ownai: lazy command declarations.
--
-- Sourced automatically when `editors/nvim` is on 'runtimepath'. Commands are
-- always available; `require("ownai").setup(opts)` configures keymaps and
-- refresh behaviour.

if vim.g.loaded_ownai == 1 then
  return
end
vim.g.loaded_ownai = 1

local function mode_complete(modes)
  return function()
    return modes
  end
end

vim.api.nvim_create_user_command("OwnaiShow", function(args)
  require("ownai.view").show(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "OwnAI: project the current buffer and fold it to a mode (default signatures)",
})

vim.api.nvim_create_user_command("OwnaiFold", function(args)
  require("ownai.view").fold(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures", "full" }),
  desc = "OwnAI: re-fold the current buffer locally (types|signatures|full)",
})

vim.api.nvim_create_user_command("OwnaiOutline", function()
  require("ownai.view").outline()
end, {
  nargs = 0,
  desc = "OwnAI: pick a declaration from the outline",
})

vim.api.nvim_create_user_command("OwnaiEnable", function(args)
  require("ownai").enable(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "OwnAI: enable global auto-fold and fold the current buffer (default: last enabled mode, else the configured default)",
})

vim.api.nvim_create_user_command("OwnaiDisable", function()
  require("ownai").disable()
end, {
  nargs = 0,
  desc = "OwnAI: disable global auto-fold and unfold the buffers it folded",
})

vim.api.nvim_create_user_command("OwnaiToggle", function(args)
  require("ownai").toggle(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "OwnAI: toggle global auto-fold (default: last enabled mode, else the configured default)",
})

-- Install default autocmds immediately so commands work without setup().
require("ownai").setup()
