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

-- Install default autocmds immediately so commands work without setup().
require("ownai").setup()
