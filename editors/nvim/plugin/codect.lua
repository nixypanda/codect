-- codect: lazy command declarations.
--
-- Sourced automatically when `editors/nvim` is on 'runtimepath'. Commands are
-- always available; `require("codect").setup(opts)` configures keymaps and
-- refresh behaviour.

if vim.g.loaded_codect == 1 then
  return
end
vim.g.loaded_codect = 1

local function mode_complete(modes)
  return function()
    return modes
  end
end

vim.api.nvim_create_user_command("CodectShow", function(args)
  require("codect.view").show(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "Codect: project the current buffer and fold it to a mode (default signatures)",
})

vim.api.nvim_create_user_command("CodectFold", function(args)
  require("codect.view").fold(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures", "full" }),
  desc = "Codect: re-fold the current buffer locally (types|signatures|full)",
})

vim.api.nvim_create_user_command("CodectOutline", function()
  require("codect.view").outline()
end, {
  nargs = 0,
  desc = "Codect: pick a declaration from the outline",
})

vim.api.nvim_create_user_command("CodectEnable", function(args)
  require("codect").enable(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "Codect: enable global auto-fold and fold the current buffer (default: last enabled mode, else the configured default)",
})

vim.api.nvim_create_user_command("CodectDisable", function()
  require("codect").disable()
end, {
  nargs = 0,
  desc = "Codect: disable global auto-fold and unfold the buffers it folded",
})

vim.api.nvim_create_user_command("CodectToggle", function(args)
  require("codect").toggle(args.fargs[1])
end, {
  nargs = "?",
  complete = mode_complete({ "types", "signatures" }),
  desc = "Codect: toggle global auto-fold (default: last enabled mode, else the configured default)",
})

vim.api.nvim_create_user_command("CodectDiffview", function(args)
  require("codect.diffview").set_mode(args.fargs[1])
end, {
  nargs = 1,
  complete = mode_complete({ "types", "signatures", "source" }),
  desc = "Codect: switch the current Diffview tab between focused and source diffs",
})

-- Diffview emits this before its scheduled file-list update. Loading the
-- adapter here also works when Diffview is loaded lazily after Codect.
vim.api.nvim_create_autocmd("User", {
  pattern = "DiffviewViewOpened",
  callback = function()
    local ok, lib = pcall(require, "diffview.lib")
    if ok then require("codect.diffview").on_view_opened(lib.get_current_view()) end
  end,
})

-- Install default autocmds immediately so commands work without setup().
require("codect").setup()
