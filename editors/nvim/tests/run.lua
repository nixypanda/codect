-- Codect Neovim plugin headless test runner.
--
-- Usage (from the repository root):
--   CODECT_BIN=target/debug/codect nvim --headless --cmd "set rtp+=editors/nvim" \
--     -l editors/nvim/tests/run.lua
--
-- `just test-nvim` builds the binary and sets CODECT_BIN.

local source = debug.getinfo(1, "S").source:sub(2)
local root = vim.fn.fnamemodify(source, ":p:h:h")
local repo = vim.fn.fnamemodify(root, ":h:h")

vim.opt.runtimepath:prepend(root)
vim.cmd("cd " .. vim.fn.fnameescape(repo))

-- Resolve a default binary so the suite runs without CODECT_BIN.
if vim.env.CODECT_BIN == nil or vim.env.CODECT_BIN == "" then
  for _, candidate in ipairs({ "target/debug/codect", "target/release/codect" }) do
    local path = repo .. "/" .. candidate
    if vim.fn.executable(path) == 1 then
      vim.env.CODECT_BIN = path
      break
    end
  end
end

local binary = require("codect.cli").resolve_binary()
if not binary then
  io.stderr:write("codect binary not found; set CODECT_BIN or build target/debug/codect\n")
  vim.cmd("cquit 1")
end
print("codect binary: " .. binary)

-- Load the plugin so commands/keymaps are registered, then configure it.
vim.cmd("runtime! plugin/codect.lua")
require("codect").setup({})

local H = dofile(root .. "/tests/harness.lua")
H.root = root
H.repo = repo

local specs = {
  "test_show.lua",
  "test_dropped.lua",
  "test_folds.lua",
  "test_annotations.lua",
  "test_one_liners.lua",
  "test_sticky.lua",
  "test_outline.lua",
  "test_schema.lua",
  "test_config.lua",
  "test_auto.lua",
}

for _, spec in ipairs(specs) do
  local register = dofile(root .. "/tests/" .. spec)
  register(H)
end

H.run()
