-- OwnAI Neovim plugin headless test runner.
--
-- Usage (from the repository root):
--   OWNAI_BIN=target/debug/ownai nvim --headless --cmd "set rtp+=editors/nvim" \
--     -l editors/nvim/tests/run.lua
--
-- `just test-nvim` builds the binary and sets OWNAI_BIN.

local source = debug.getinfo(1, "S").source:sub(2)
local root = vim.fn.fnamemodify(source, ":p:h:h")
local repo = vim.fn.fnamemodify(root, ":h:h")

vim.opt.runtimepath:prepend(root)
vim.cmd("cd " .. vim.fn.fnameescape(repo))

-- Resolve a default binary so the suite runs without OWNAI_BIN.
if vim.env.OWNAI_BIN == nil or vim.env.OWNAI_BIN == "" then
  for _, candidate in ipairs({ "target/debug/ownai", "target/release/ownai" }) do
    local path = repo .. "/" .. candidate
    if vim.fn.executable(path) == 1 then
      vim.env.OWNAI_BIN = path
      break
    end
  end
end

local binary = require("ownai.cli").resolve_binary()
if not binary then
  io.stderr:write("ownai binary not found; set OWNAI_BIN or build target/debug/ownai\n")
  vim.cmd("cquit 1")
end
print("ownai binary: " .. binary)

-- Load the plugin so commands/keymaps are registered, then configure it.
vim.cmd("runtime! plugin/ownai.lua")
require("ownai").setup({})

local H = dofile(root .. "/tests/harness.lua")
H.root = root
H.repo = repo

local specs = {
  "test_show.lua",
  "test_dropped.lua",
  "test_folds.lua",
  "test_one_liners.lua",
  "test_sticky.lua",
  "test_outline.lua",
  "test_schema.lua",
  "test_config.lua",
}

for _, spec in ipairs(specs) do
  local register = dofile(root .. "/tests/" .. spec)
  register(H)
end

H.run()
