-- Integration test; run with OWNAI_DIFFVIEW_RTP pointing at Diffview 4516612.
local diffview_path = vim.env.OWNAI_DIFFVIEW_RTP
if not diffview_path or diffview_path == "" then
  print("Diffview integration skipped (set OWNAI_DIFFVIEW_RTP)")
  return
end

local source = debug.getinfo(1, "S").source:sub(2)
local plugin = vim.fn.fnamemodify(source, ":p:h:h")
local ownai_root = vim.fn.fnamemodify(plugin, ":h:h")
vim.opt.runtimepath:prepend(plugin)
vim.opt.runtimepath:prepend(diffview_path)
vim.env.OWNAI_BIN = vim.env.OWNAI_BIN or ownai_root .. "/target/debug/ownai"
vim.cmd("runtime! plugin/ownai.lua")
vim.cmd("runtime! plugin/diffview.lua")
require("diffview").setup({ use_icons = false })

local function run(argv, cwd)
  local result = vim.system(argv, { cwd = cwd, text = true }):wait()
  assert(result.code == 0, table.concat(argv, " ") .. "\n" .. (result.stderr or ""))
  return vim.trim(result.stdout or "")
end

local function write(path, value)
  local f = assert(io.open(path, "wb"))
  f:write(value)
  f:close()
end

local tmp = vim.fn.tempname()
vim.fn.mkdir(tmp, "p")
run({ "git", "init", "-q" }, tmp)
run({ "git", "config", "user.email", "test@example.com" }, tmp)
run({ "git", "config", "user.name", "Test" }, tmp)
local path = tmp .. "/example.rs"
write(path, "pub fn example() -> i32 { 1 }\n")
run({ "git", "add", "." }, tmp)
run({ "git", "commit", "-qm", "initial" }, tmp)
write(path, "pub fn example() -> i32 { 2 }\n")
run({ "git", "commit", "-qam", "body only" }, tmp)
write(path, "pub fn example(x: i32) -> i32 { x }\n")
run({ "git", "commit", "-qam", "signature" }, tmp)

vim.cmd("cd " .. vim.fn.fnameescape(tmp))
local test_case = vim.env.OWNAI_TEST_DIFFVIEW_CASE or "history"
local lib = require("diffview.lib")
if test_case == "history" then
vim.cmd("DiffviewFileHistory")
assert(vim.wait(15000, function()
  local view = lib.get_current_view()
  return view and view.panel and not view.panel.updating and #view.panel.entries >= 3
end, 20), "history did not load")
local view = lib.get_current_view()
assert(#view.panel.entries >= 3)
assert(not view.panel.entries[1].nulled, "signature commit was hidden")
assert(view.panel.entries[2].nulled, "body-only commit was not hidden")
local file = view.panel.entries[1].files[1]
view:set_file(file)
assert(vim.wait(10000, function()
  return file.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(file.layout.b.file.bufnr)
end, 20), "projection buffer did not load")
local lines = vim.api.nvim_buf_get_lines(file.layout.b.file.bufnr, 0, -1, false)
assert(table.concat(lines, "\n"):find("example%(x: i32%)"), "projected signature missing")
assert(not table.concat(lines, "\n"):find("{ x }", 1, true), "function body leaked")
assert(not vim.bo[file.layout.b.file.bufnr].modifiable, "history buffer is writable")

view:set_file(view.panel.entries[2].files[1])
assert(vim.wait(10000, function()
  return view.panel.cur_item[1] and view.panel.cur_item[1].commit.hash == view.panel.entries[2].commit.hash
end, 20), "body-only commit was not selected")
local selected_hash = view.panel.cur_item[1].commit.hash
vim.cmd("OwnaiDiffview source")
assert(vim.wait(15000, function() return not view.panel.updating and #view.panel.entries >= 3 end, 20))
assert(not view.panel.entries[2].nulled, "source mode failed to restore body-only commit")
assert(vim.wait(10000, function()
  return view.panel.cur_item[1] and view.panel.cur_item[1].commit.hash == selected_hash
end, 20), "mode switch lost selected commit")
file = view.panel.entries[1].files[1]
view:set_file(file)
assert(vim.wait(10000, function()
  return file.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(file.layout.b.file.bufnr)
end, 20), "source history buffer did not finish loading")
vim.cmd("DiffviewClose")
end

if test_case == "range" then
vim.cmd("DiffviewOpen HEAD~1..HEAD")
assert(vim.wait(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.files:len() == 1
end, 20), "commit range did not load")
view = lib.get_current_view()
file = view.panel:ordered_file_list()[1]
assert(file, "projected range file absent")
assert(vim.wait(10000, function()
  return file.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(file.layout.b.file.bufnr)
end, 20), "range projection buffer did not load")
lines = vim.api.nvim_buf_get_lines(file.layout.b.file.bufnr, 0, -1, false)
assert(table.concat(lines, "\n"):find("example%(x: i32%)"), "range projection missing")
assert(not table.concat(lines, "\n"):find("{ x }", 1, true), "range body leaked")
vim.cmd("OwnaiDiffview source")
assert(vim.wait(10000, function()
  local current = view.panel:ordered_file_list()[1]
  if not current or not current.layout.b.file.bufnr then return false end
  local bufnr = current.layout.b.file.bufnr
  return vim.api.nvim_buf_is_loaded(bufnr)
    and table.concat(vim.api.nvim_buf_get_lines(bufnr, 0, -1, false), "\n"):find("{ x }", 1, true)
end, 20), "source buffer did not load")
file = view.panel:ordered_file_list()[1]
lines = vim.api.nvim_buf_get_lines(file.layout.b.file.bufnr, 0, -1, false)
assert(table.concat(lines, "\n"):find("{ x }", 1, true), "source mode did not restore source bytes")
vim.cmd("DiffviewClose")
end
print("Diffview integration passed")
