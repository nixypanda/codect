-- Integration test; run with CODECT_DIFFVIEW_RTP pointing at Diffview 4516612.
local diffview_path = vim.env.CODECT_DIFFVIEW_RTP
if not diffview_path or diffview_path == "" then
  print("Diffview integration skipped (set CODECT_DIFFVIEW_RTP)")
  return
end

local source = debug.getinfo(1, "S").source:sub(2)
local plugin = vim.fn.fnamemodify(source, ":p:h:h")
local codect_root = vim.fn.fnamemodify(plugin, ":h:h")
vim.opt.runtimepath:prepend(plugin)
vim.opt.runtimepath:prepend(diffview_path)
vim.env.CODECT_BIN = vim.env.CODECT_BIN or codect_root .. "/target/debug/codect"
vim.cmd("runtime! plugin/codect.lua")
vim.cmd("runtime! plugin/diffview.lua")
require("diffview").setup({ use_icons = false })

local function run(argv, cwd, input)
  local opts = { cwd = cwd, text = true }
  if input ~= nil then opts.stdin = input end
  local result = vim.system(argv, opts):wait()
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
local other = tmp .. "/other.rs"
write(path, "pub fn example() -> i32 { 1 }\n")
write(other, "pub fn other() -> i32 { 1 }\n")
run({ "git", "add", "." }, tmp)
run({ "git", "commit", "-qm", "initial" }, tmp)
write(path, "pub fn example() -> i32 { 2 }\n")
run({ "git", "commit", "-qam", "body only" }, tmp)
write(path, "pub fn example(x: i32) -> i32 { x }\n")
write(other, "pub fn other(x: i32) -> i32 { x }\n")
run({ "git", "commit", "-qam", "signature" }, tmp)

vim.cmd("cd " .. vim.fn.fnameescape(tmp))
local test_case = vim.env.CODECT_TEST_DIFFVIEW_CASE or "history"
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
assert(#view.panel.entries[1].files == 2, "history did not retain both changed files")
local file
for _, candidate in ipairs(view.panel.entries[1].files) do
  if candidate.path == "example.rs" then file = candidate break end
end
assert(file, "example.rs missing from selected commit")
view:set_file(file)
assert(vim.wait(10000, function()
  return file.layout.a.file.bufnr and vim.api.nvim_buf_is_loaded(file.layout.a.file.bufnr)
    and file.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(file.layout.b.file.bufnr)
end, 20), "projection buffer did not load")
local before = table.concat(vim.api.nvim_buf_get_lines(file.layout.a.file.bufnr, 0, -1, false), "\n")
local after = table.concat(vim.api.nvim_buf_get_lines(file.layout.b.file.bufnr, 0, -1, false), "\n")
assert(before:find("example%(%)") and not before:find("x: i32", 1, true), "left pane is not the parent projection: " .. before)
assert(after:find("x: i32", 1, true), "right pane is not the selected commit projection: " .. after)
assert(before ~= after, "selected commit has identical projection panes")
assert(file.layout.a.file.winopts.foldlevel == 99 and file.layout.b.file.winopts.foldlevel == 99,
  "focused diff panes start folded")
assert(vim.wo[view.cur_layout.a.id].diff and vim.wo[view.cur_layout.b.id].diff,
  "focused panes are not in diff mode")
assert(vim.wo[view.cur_layout.a.id].foldlevel == 99 and vim.wo[view.cur_layout.b.id].foldlevel == 99,
  "projected windows are folded")
local lines = vim.api.nvim_buf_get_lines(file.layout.b.file.bufnr, 0, -1, false)
assert(table.concat(lines, "\n"):find("example%(x: i32%)"), "projected signature missing")
assert(not table.concat(lines, "\n"):find("{ x }", 1, true), "function body leaked")
assert(not vim.bo[file.layout.b.file.bufnr].modifiable, "history buffer is writable")

local other_file
for _, candidate in ipairs(view.panel.entries[1].files) do
  if candidate.path == "other.rs" then other_file = candidate break end
end
assert(other_file, "other.rs missing from selected commit")
view:set_file(other_file)
assert(vim.wait(10000, function()
  return other_file.layout.a.file.bufnr and other_file.layout.b.file.bufnr
    and vim.api.nvim_buf_is_loaded(other_file.layout.a.file.bufnr)
    and vim.api.nvim_buf_is_loaded(other_file.layout.b.file.bufnr)
end, 20), "second file's projection panes did not load")
local other_before = table.concat(vim.api.nvim_buf_get_lines(other_file.layout.a.file.bufnr, 0, -1, false), "\n")
local other_after = table.concat(vim.api.nvim_buf_get_lines(other_file.layout.b.file.bufnr, 0, -1, false), "\n")
assert(other_before:find("other%(%)") and other_after:find("other%(x: i32%)"),
  "second file does not show its per-commit difference")

vim.cmd("CodectDiffview types")
assert(vim.wait(15000, function()
  return not view.panel.updating and #view.panel.entries >= 3 and view.panel.entries[1].nulled
end, 20), "Types mode did not hide signature-only commit")
vim.cmd("CodectDiffview signatures")
assert(vim.wait(15000, function()
  return not view.panel.updating and #view.panel.entries[1].files == 2
    and not view.panel.entries[1].nulled
end, 20), "Signatures mode did not restore both changed files")

view:set_file(view.panel.entries[2].files[1])
assert(vim.wait(10000, function()
  return view.panel.cur_item[1] and view.panel.cur_item[1].commit.hash == view.panel.entries[2].commit.hash
end, 20), "body-only commit was not selected")
local selected_hash = view.panel.cur_item[1].commit.hash
vim.cmd("CodectDiffview source")
assert(vim.wait(15000, function() return not view.panel.updating and #view.panel.entries >= 3 end, 20))
assert(not view.panel.entries[2].nulled, "source mode failed to restore body-only commit")
assert(vim.wait(10000, function()
  return view.panel.cur_item[1] and view.panel.cur_item[1].commit.hash == selected_hash
end, 20), "mode switch lost selected commit")
file = view.panel.entries[1].files[1]
view:set_file(file)
local source_name = file.path == "example.rs" and "example" or "other"
assert(file.path == "example.rs" or file.path == "other.rs", "unexpected source file path")
local source_body = source_name == "example" and "2" or "1"
local expected_before = "pub fn " .. source_name .. "() -> i32 { " .. source_body .. " }"
local expected_after = "pub fn " .. source_name .. "(x: i32) -> i32 { x }"
local function source_lines(side)
  local value = side and side.file and side.file.bufnr
  if not value or not vim.api.nvim_buf_is_loaded(value) then
    return nil
  end
  return table.concat(vim.api.nvim_buf_get_lines(value, 0, -1, false), "\n")
end
assert(vim.wait(10000, function()
  local a, b = file.layout.a.file, file.layout.b.file
  if not (a.bufnr and b.bufnr
      and vim.api.nvim_buf_is_loaded(a.bufnr) and vim.api.nvim_buf_is_loaded(b.bufnr)
      and file.layout:is_files_loaded()
      and view.cur_layout.a and view.cur_layout.b
      and vim.api.nvim_win_get_buf(view.cur_layout.a.id) == a.bufnr
      and vim.api.nvim_win_get_buf(view.cur_layout.b.id) == b.bufnr) then
    return false
  end
  local left = source_lines(file.layout.a) or ""
  local right = source_lines(file.layout.b) or ""
  return left:find(expected_before, 1, true) ~= nil
    and right:find(expected_after, 1, true) ~= nil
end, 20), "source history buffers did not finish loading")
local source_before = source_lines(file.layout.a)
local source_after = source_lines(file.layout.b)
assert(source_before:find(expected_before, 1, true),
  "source history left side is not the parent commit source")
assert(source_after:find(expected_after, 1, true),
  "source history right side is not the commit source")
vim.cmd("DiffviewClose")
end

if test_case == "range" then
vim.cmd("DiffviewOpen HEAD~1..HEAD")
assert(vim.wait(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.files:len() == 2
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
vim.cmd("CodectDiffview source")
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
if test_case == "local" then
  local body = tmp .. "/body.rs"
  write(body, "pub fn body() -> i32 { 1 }\n")
  run({ "git", "add", "." }, tmp)
  run({ "git", "commit", "-qm", "add body file" }, tmp)
  write(path, "pub fn example(x: u32) -> i32 { x as i32 }\n")
  write(body, "pub fn body() -> i32 { 2 }\n")
  run({ "git", "add", "." }, tmp)
  local index_source = run({ "git", "show", ":example.rs" }, tmp)
  write(path, "pub fn example(x: u64) -> i32 { x as i32 }\n")
  write(body, "pub fn body() -> i32 { 3 }\n")
  local disk_source = vim.trim(assert(io.open(path, "rb")):read("*a"))

  vim.cmd("DiffviewOpen")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.files and #current.files.staged == 1 and #current.files.working == 1
  end, 20), "focused local sections did not load")
  local current = lib.get_current_view()
  assert(current.files.staged[1].path == "example.rs", "staged body-only file leaked")
  assert(current.files.working[1].path == "example.rs", "unstaged body-only file leaked")
  local staged = current.files.staged[1]
  current:set_file(staged)
  assert(vim.wait(10000, function()
    return staged.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(staged.layout.b.file.bufnr)
  end, 20), "staged projection did not load")
  local stagebuf = staged.layout.b.file.bufnr
  assert(not vim.bo[stagebuf].modifiable and vim.bo[stagebuf].buftype == "nowrite", "stage pane is editable")
  assert(table.concat(vim.api.nvim_buf_get_lines(stagebuf, 0, -1, false), "\n"):find("x: u32", 1, true), vim.inspect(vim.api.nvim_buf_get_lines(stagebuf, 0, -1, false)))
  assert(run({ "git", "show", ":example.rs" }, tmp) == index_source, "index changed by projection")

  local working = current.files.working[1]
  current:set_file(working)
  assert(vim.wait(10000, function()
    return working.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(working.layout.b.file.bufnr)
  end, 20), "worktree projection did not load")
  local workbuf = working.layout.b.file.bufnr
  assert(not vim.bo[workbuf].modifiable and vim.bo[workbuf].buftype == "nowrite", "worktree pane is editable")
  assert(table.concat(vim.api.nvim_buf_get_lines(workbuf, 0, -1, false), "\n"):find("x: u64", 1, true))
  assert(vim.trim(assert(io.open(path, "rb")):read("*a")) == disk_source, "worktree changed by projection")

  vim.cmd("CodectDiffview types")
  assert(vim.wait(10000, function()
    return #current.files.staged == 0 and #current.files.working == 0
  end, 20), "Types mode did not filter signature-only files")
  vim.cmd("CodectDiffview source")
  assert(vim.wait(10000, function()
    return #current.files.staged == 2 and #current.files.working == 2
  end, 20), "source mode did not restore ordinary local file lists")
  vim.cmd("CodectDiffview signatures")
  assert(vim.wait(10000, function() return #current.files.working == 1 end, 20))
  write(path, "pub fn example(x: u128) -> i32 { x as i32 }\n")
  current:update_files()
  assert(vim.wait(10000, function()
    local entry = current.files.working[1]
    if not entry then return false end
    local buf = entry.layout.b.file.bufnr
    return buf and vim.api.nvim_buf_is_loaded(buf)
      and table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), "\n"):find("x: u128", 1, true)
  end, 20), "mutable snapshot refresh reused stale projection")
  local focused = current.files.working[1].layout.b.file.bufnr
  vim.cmd("DiffviewClose")
  assert(not vim.api.nvim_buf_is_valid(focused), "focused worktree scratch buffer leaked on close")
end
if test_case == "root" then
  local root = run({ "git", "rev-list", "--max-parents=0", "HEAD" }, tmp)
  vim.cmd("DiffviewFileHistory --range=" .. root)
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating and #current.panel.entries == 1
  end, 20), "root commit history did not load")
  local current = lib.get_current_view()
  local first = current.panel.entries[1]
  assert(not first.nulled and #first.files == 2, "root projection missing")
  current:set_file(first.files[1])
  assert(vim.wait(10000, function()
    return first.files[1].layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(first.files[1].layout.b.file.bufnr)
  end, 20), "root projection buffer did not load")
  local lines = vim.api.nvim_buf_get_lines(first.files[1].layout.b.file.bufnr, 0, -1, false)
  assert(table.concat(lines, "\n"):find("example", 1, true), "root signature missing")
  vim.cmd("DiffviewClose")
end
if test_case == "branch" then
  require("diffview").open({ "HEAD~2...HEAD" })
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.files and current.files:len() == 2
  end, 20), "three-dot branch comparison did not load")
  vim.cmd("DiffviewClose")
  vim.cmd("DiffviewFileHistory --range=HEAD~2..HEAD")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating and #current.panel.entries == 2
  end, 20), "branch history did not load")
  local entries = lib.get_current_view().panel.entries
  assert(not entries[1].nulled and entries[2].nulled, "branch commit projections are incorrect")
  vim.cmd("DiffviewClose")
end
if test_case == "move" then
  local RevType = require("diffview.vcs.rev").RevType
  local moved_old = tmp .. "/moved_old.rs"
  local moved_new = tmp .. "/moved_new.rs"
  local moved_work = tmp .. "/moved_work.rs"
  local function test_source(signature)
    return table.concat({
      "pub fn moved(x: " .. signature .. ") -> " .. signature .. " { x }",
      "",
      "#[test]",
      "fn moved_test() {",
      "    assert_eq!(moved(1), 1);",
      "}",
      "",
    }, "\n")
  end

  write(moved_old, test_source("i32"))
  run({ "git", "add", "." }, tmp)
  run({ "git", "commit", "-qm", "add moved test file" }, tmp)
  run({ "git", "mv", "moved_old.rs", "moved_new.rs" }, tmp)
  write(moved_new, test_source("u64"))
  run({ "git", "commit", "-qam", "move test file" }, tmp)

  vim.cmd("DiffviewFileHistory --range=HEAD~1..HEAD")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating
      and #current.panel.entries == 1 and #current.panel.entries[1].files == 1
  end, 20), "moved history entry did not load")
  local current = lib.get_current_view()
  local entry = current.panel.entries[1]
  assert(not entry.nulled, "renamed commit was hidden")
  local moved = entry.files[1]
  assert(moved.path == "moved_new.rs" and moved.oldpath == "moved_old.rs" and moved.status == "R",
    "history did not preserve the Git rename pair")
  assert(moved.layout.a and moved.layout.b and not moved.layout.a.file.nulled and not moved.layout.b.file.nulled,
    "rename entry does not have two live sides")
  current:set_file(moved)
  assert(vim.wait(10000, function()
    return moved.layout.a.file.bufnr and moved.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(moved.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(moved.layout.b.file.bufnr)
  end, 20), "rename projection buffers did not load")
  local before = table.concat(vim.api.nvim_buf_get_lines(moved.layout.a.file.bufnr, 0, -1, false), "\n")
  local after = table.concat(vim.api.nvim_buf_get_lines(moved.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(before:find("x: i32", 1, true) and not before:find("x: u64", 1, true),
    "rename left pane is not the old projection")
  assert(after:find("x: u64", 1, true) and after:find("fn moved_test", 1, true),
    "rename right pane is not the new projection")
  assert(not before:find("assert_eq", 1, true) and not after:find("assert_eq", 1, true),
    "function body leaked through the move")

  vim.cmd("CodectDiffview types")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.panel.entries == 1
      and current.panel.entries[1].nulled
  end, 20), "Types mode did not omit the empty move")
  vim.cmd("CodectDiffview tests")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.panel.entries == 1
      and not current.panel.entries[1].nulled and #current.panel.entries[1].files == 1
  end, 20), "Tests mode did not restore the moved test file")
  moved = current.panel.entries[1].files[1]
  assert(moved.path == "moved_new.rs" and moved.oldpath == "moved_old.rs" and moved.status == "R",
    "Tests mode lost the move pair")
  current:set_file(moved)
  assert(vim.wait(10000, function()
    return moved.layout.a.file.bufnr and moved.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(moved.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(moved.layout.b.file.bufnr)
  end, 20), "Tests mode move buffers did not load")
  before = table.concat(vim.api.nvim_buf_get_lines(moved.layout.a.file.bufnr, 0, -1, false), "\n")
  after = table.concat(vim.api.nvim_buf_get_lines(moved.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(before == after and before:find("fn moved_test", 1, true),
    "Tests mode did not show the equal test-only move on both sides")
  assert(not before:find("pub fn moved", 1, true) and not before:find("assert_eq", 1, true),
    "Tests mode leaked non-test declarations or bodies")
  vim.cmd("CodectDiffview signatures")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.panel.entries == 1
      and not current.panel.entries[1].nulled and #current.panel.entries[1].files == 1
  end, 20), "Signatures mode did not restore the move")
  vim.cmd("DiffviewClose")

  -- Make Git report a delete/add pair while Codect still pairs the move. The
  -- adapter must build one rename entry with two live sides.
  run({ "git", "config", "diff.renames", "false" }, tmp)
  vim.fn.rename(moved_new, moved_work)
  vim.cmd("DiffviewOpen")
  assert(vim.wait(15000, function()
    local view = lib.get_current_view()
    return view and view.files and #view.files.working == 1
  end, 20), "constructed working move did not load")
  current = lib.get_current_view()
  assert(#current.files.staged == 0, "unexpected staged entry")
  local working = current.files.working[1]
  assert(working.path == "moved_work.rs" and working.oldpath == "moved_new.rs" and working.status == "R",
    "delete/add pair was not reconciled into a move")
  assert(working.layout.b.file.rev.type == RevType.LOCAL,
    "constructed worktree target is not the local worktree")
  assert(working.layout.b.file.winbar:find("WORKING TREE", 1, true),
    "constructed worktree target winbar is not the working tree")
  assert(working.layout.a and working.layout.b
      and not working.layout.a.file.nulled and not working.layout.b.file.nulled,
    "constructed move has a null side")
  current:set_file(working)
  assert(vim.wait(10000, function()
    return working.layout.a.file.bufnr and working.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(working.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(working.layout.b.file.bufnr)
  end, 20), "constructed working move buffers did not load")
  before = table.concat(vim.api.nvim_buf_get_lines(working.layout.a.file.bufnr, 0, -1, false), "\n")
  after = table.concat(vim.api.nvim_buf_get_lines(working.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(before:find("x: u64", 1, true) and after:find("x: u64", 1, true),
    "constructed worktree move sides are not the old/new index and worktree projections")
  assert(not vim.bo[working.layout.a.file.bufnr].modifiable
      and not vim.bo[working.layout.b.file.bufnr].modifiable
      and vim.bo[working.layout.a.file.bufnr].buftype == "nowrite"
      and vim.bo[working.layout.b.file.bufnr].buftype == "nowrite",
    "constructed worktree move reused a real writable buffer")
  vim.cmd("CodectDiffview tests")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.files.working == 1
      and current.files.working[1].status == "R"
  end, 20), "Tests mode did not keep the moved test file")
  vim.cmd("CodectDiffview signatures")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.files.working == 1
      and current.files.working[1].status == "R"
  end, 20), "Signatures mode did not restore the working move")

  run({ "git", "add", "-A" }, tmp)
  current:update_files()
  assert(vim.wait(15000, function()
    return #current.files.working == 0 and #current.files.staged == 1
  end, 20), "staged constructed move did not load")
  local staged = current.files.staged[1]
  assert(staged.path == "moved_work.rs" and staged.oldpath == "moved_new.rs" and staged.status == "R",
    "staged section did not reconcile into a move")
  assert(staged.layout.b.file.rev.type == RevType.STAGE and staged.layout.b.file.rev.stage == 0,
    "constructed staged target is not stage-zero index")
  assert(staged.layout.b.file.winbar:find("INDEX", 1, true),
    "constructed staged target winbar is not the index")
  current:set_file(staged)
  assert(vim.wait(10000, function()
    return staged.layout.a.file.bufnr and staged.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(staged.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(staged.layout.b.file.bufnr)
  end, 20), "staged constructed move buffers did not load")
  assert(not vim.bo[staged.layout.a.file.bufnr].modifiable
      and not vim.bo[staged.layout.b.file.bufnr].modifiable
      and vim.bo[staged.layout.a.file.bufnr].buftype == "nowrite"
      and vim.bo[staged.layout.b.file.bufnr].buftype == "nowrite",
    "constructed staged move reused a real writable buffer")

  write(moved_work, test_source("u128"))
  current:update_files()
  assert(vim.wait(15000, function()
    return #current.files.working == 1 and #current.files.staged == 1
  end, 20), "working refresh did not keep both sections")
  local modified = current.files.working[1]
  assert(modified.path == "moved_work.rs" and modified.oldpath == nil and modified.status == "M",
    "working refresh did not return a same-path modification")
  current:set_file(modified)
  assert(vim.wait(10000, function()
    return modified.layout.a.file.bufnr and modified.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(modified.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(modified.layout.b.file.bufnr)
  end, 20), "refreshed modification buffers did not load")
  before = table.concat(vim.api.nvim_buf_get_lines(modified.layout.a.file.bufnr, 0, -1, false), "\n")
  after = table.concat(vim.api.nvim_buf_get_lines(modified.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(before:find("x: u64", 1, true) and after:find("x: u128", 1, true),
    "refresh reused a stale projection for the moved file")

  run({ "git", "add", "-A" }, tmp)
  current:update_files()
  assert(vim.wait(15000, function()
    return #current.files.working == 0 and #current.files.staged == 1
  end, 20), "restaged move did not refresh both sections")
  local refreshed = current.files.staged[1]
  assert(refreshed.path == "moved_work.rs" and refreshed.oldpath == "moved_new.rs"
      and refreshed.status == "R", "restaged move lost its pair")
  current:set_file(refreshed)
  assert(vim.wait(10000, function()
    return refreshed.layout.a.file.bufnr and refreshed.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(refreshed.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(refreshed.layout.b.file.bufnr)
  end, 20), "refreshed move buffers did not load")
  before = table.concat(vim.api.nvim_buf_get_lines(refreshed.layout.a.file.bufnr, 0, -1, false), "\n")
  after = table.concat(vim.api.nvim_buf_get_lines(refreshed.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(before:find("x: u64", 1, true) and after:find("x: u128", 1, true),
    "staged move refresh kept a stale projection")

  -- A modified index buffer belongs to the user. Entering focused mode must
  -- detach it, not delete it, and closing the view must leave it alive.
  vim.cmd("CodectDiffview source")
  assert(vim.wait(15000, function()
    local selected = current.panel.cur_file
    return not current.panel.updating and #current.files.staged == 2
      and selected and selected.layout and selected.layout:is_files_loaded()
  end, 20), "source mode did not restore the staged delete/add entries")
  local source_added
  for _, candidate in ipairs(current.files.staged) do
    if candidate.path == "moved_work.rs" then source_added = candidate end
  end
  assert(source_added, "source mode did not recreate the added index entry")
  assert(source_added.layout.a.file.nulled and not source_added.layout.b.file.nulled,
    "source add entry does not have one null side")
  current:set_file(source_added)
  assert(vim.wait(10000, function()
    return source_added.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(source_added.layout.b.file.bufnr)
  end, 20), "real index buffer did not load")
  local user_buf = source_added.layout.b.file.bufnr
  assert(vim.bo[user_buf].modifiable, "source index buffer is not editable")
  vim.api.nvim_buf_set_lines(user_buf, 0, -1, false, { "pub fn user_edit() {}" })
  assert(vim.bo[user_buf].modified, "index buffer did not record the edit")

  vim.cmd("CodectDiffview signatures")
  assert(vim.wait(15000, function()
    return not current.panel.updating and #current.files.staged == 1
      and current.files.staged[1].path == "moved_work.rs"
  end, 20), "focused mode did not replace the staged delete/add entries")
  assert(vim.api.nvim_buf_is_valid(user_buf) and vim.bo[user_buf].modified,
    "focused mode deleted a modified real index buffer")
  vim.cmd("DiffviewClose")
  assert(vim.api.nvim_buf_is_valid(user_buf) and vim.bo[user_buf].modified,
    "closing the view deleted a modified real index buffer")
end
if test_case == "move-split" then
  local old = tmp .. "/lang_old.elm"
  local new = tmp .. "/lang_new.hs"
  write(old, "x = 1\n")
  run({ "git", "add", "." }, tmp)
  run({ "git", "commit", "-qm", "add language source" }, tmp)
  run({ "git", "mv", "lang_old.elm", "lang_new.hs" }, tmp)
  run({ "git", "commit", "-qm", "cross-language rename" }, tmp)

  vim.cmd("DiffviewFileHistory --range=HEAD~1..HEAD")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating
      and #current.panel.entries == 1 and #current.panel.entries[1].files == 2
  end, 20), "split Git rename did not load two Codect entries")
  local current = lib.get_current_view()
  local entry = current.panel.entries[1]
  local deleted, added
  for _, candidate in ipairs(entry.files) do
    if candidate.status == "D" then deleted = candidate end
    if candidate.status == "A" then added = candidate end
  end
  assert(deleted and added, "Git rename was not split into delete/add entries")
  assert(deleted.path == "lang_old.elm" and deleted.oldpath == nil, "delete entry path is wrong")
  assert(added.path == "lang_new.hs" and added.oldpath == nil, "add entry path is wrong")
  assert(not deleted.layout.a.file.nulled and deleted.layout.b.file.nulled,
    "delete entry does not keep only its old side live")
  assert(added.layout.a.file.nulled and not added.layout.b.file.nulled,
    "add entry does not keep only its new side live")

  current:set_file(deleted)
  assert(vim.wait(10000, function()
    return deleted.layout.a.file.bufnr and vim.api.nvim_buf_is_loaded(deleted.layout.a.file.bufnr)
  end, 20), "deleted side did not load")
  local left = table.concat(vim.api.nvim_buf_get_lines(deleted.layout.a.file.bufnr, 0, -1, false), "\n")
  assert(left:find("missing type annotation", 1, true), "deleted side is not the old Elm projection")
  assert(vim.api.nvim_buf_get_name(deleted.layout.b.file.bufnr):find("diffview://null", 1, true),
    "deleted side rendered a live new pane")

  current:set_file(added)
  assert(vim.wait(10000, function()
    return added.layout.b.file.bufnr and vim.api.nvim_buf_is_loaded(added.layout.b.file.bufnr)
  end, 20), "added side did not load")
  local right = table.concat(vim.api.nvim_buf_get_lines(added.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(vim.trim(right) == "x", "added side is not the new Haskell projection")
  assert(vim.api.nvim_buf_get_name(added.layout.a.file.bufnr):find("diffview://null", 1, true),
    "added side rendered a live old pane")
  vim.cmd("DiffviewClose")
end
if test_case == "move-paths" then
  local literal_old = "weird\\xFF.rs"
  local invalid_new = "weird\xFF.rs"
  local display_literal_old = "weird\\\\xFF.rs"
  local display_invalid_new = "weird\\xFF.rs"
  local blob = run({ "git", "hash-object", "-w", "--stdin" }, tmp,
    "pub fn weird() -> i32 { 1 }\n")

  local function tree_with(path)
    return run({ "git", "mktree", "-z" }, tmp, "100644 blob " .. blob .. "\t" .. path .. "\0")
  end
  local function commit_tree(tree, message)
    local commit = run({ "git", "commit-tree", tree, "-p", "HEAD", "-m", message }, tmp)
    run({ "git", "update-ref", "HEAD", commit }, tmp)
    return commit
  end

  commit_tree(tree_with(literal_old), "literal backslash path")
  commit_tree(tree_with(invalid_new), "invalid byte path")

  local doc = vim.json.decode(run({
    vim.env.CODECT_BIN, "diff", "--format", "json", "--mode", "signatures", "HEAD~1", "HEAD",
  }, tmp))
  assert(#doc.files == 1 and doc.files[1].status == "moved", "CLI did not pair the encoded paths")
  assert(doc.files[1].old_path == display_literal_old and doc.files[1].path == display_invalid_new,
    string.format("unexpected v2 display paths: old=%q path=%q", doc.files[1].old_path, doc.files[1].path))

  vim.cmd("DiffviewFileHistory --range=HEAD~1..HEAD")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating
      and #current.panel.entries == 1 and #current.panel.entries[1].files == 1
  end, 20), "encoded-path move did not load")
  local current = lib.get_current_view()
  local moved = current.panel.entries[1].files[1]
  assert(moved.path == invalid_new and moved.oldpath == literal_old and moved.status == "R",
    string.format("raw move tuple was not preserved: old=%q path=%q", moved.oldpath or "", moved.path))
  assert(moved.basename == display_invalid_new and moved.parent_path == "",
    "file panel did not use the display-escaped path")
  assert(moved.layout.a.file.path == literal_old and moved.layout.b.file.path == invalid_new,
    "pane files do not use raw paths")
  assert(moved.layout.a.file.winbar:find(display_literal_old, 1, true)
      and moved.layout.b.file.winbar:find(display_invalid_new, 1, true),
    "pane winbars do not use display-escaped paths")
  current:set_file(moved)
  assert(vim.wait(10000, function()
    return moved.layout.a.file.bufnr and moved.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(moved.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(moved.layout.b.file.bufnr)
  end, 20), "encoded-path move buffers did not load")
  local left = table.concat(vim.api.nvim_buf_get_lines(moved.layout.a.file.bufnr, 0, -1, false), "\n")
  local right = table.concat(vim.api.nvim_buf_get_lines(moved.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(left == right and left:find("pub fn weird", 1, true),
    "encoded-path move panes do not contain the projection")
  vim.cmd("DiffviewClose")

  local control_old = "ctrl\nold.rs"
  local control_new = "ctrl\tnew.rs"
  local display_control_old = "ctrl\\nold.rs"
  local display_control_new = "ctrl\\tnew.rs"
  commit_tree(tree_with(control_old), "control path source")
  commit_tree(tree_with(control_new), "control path destination")

  doc = vim.json.decode(run({
    vim.env.CODECT_BIN, "diff", "--format", "json", "--mode", "signatures", "HEAD~1", "HEAD",
  }, tmp))
  assert(#doc.files == 1 and doc.files[1].status == "moved", "CLI did not pair the control paths")
  assert(doc.files[1].old_path == display_control_old and doc.files[1].path == display_control_new,
    string.format("unexpected control display paths: old=%q path=%q",
      doc.files[1].old_path, doc.files[1].path))

  vim.cmd("DiffviewFileHistory --range=HEAD~1..HEAD")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.panel and not current.panel.updating
      and #current.panel.entries == 1 and #current.panel.entries[1].files == 1
  end, 20), "control-path move did not load")
  current = lib.get_current_view()
  moved = current.panel.entries[1].files[1]
  assert(moved.path == control_new and moved.oldpath == control_old and moved.status == "R",
    string.format("control raw move tuple was not preserved: old=%q path=%q",
      moved.oldpath or "", moved.path))
  assert(moved.basename == display_control_new and moved.parent_path == "",
    "control path was not display-escaped in the file panel")
  assert(moved.layout.a.file.winbar:find(display_control_old, 1, true)
      and moved.layout.b.file.winbar:find(display_control_new, 1, true),
    "control-path winbars are not display-escaped")
  current:set_file(moved)
  assert(vim.wait(10000, function()
    return moved.layout.a.file.bufnr and moved.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(moved.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(moved.layout.b.file.bufnr)
  end, 20), "control-path move buffers did not load")
  left = table.concat(vim.api.nvim_buf_get_lines(moved.layout.a.file.bufnr, 0, -1, false), "\n")
  right = table.concat(vim.api.nvim_buf_get_lines(moved.layout.b.file.bufnr, 0, -1, false), "\n")
  assert(left == right and left:find("pub fn weird", 1, true),
    "control-path move panes do not contain the projection")
  vim.cmd("DiffviewClose")
end
if test_case == "move-tree-labels" then
  local root = vim.fn.tempname()
  vim.fn.mkdir(root, "p")
  root = vim.fn.resolve(root)
  run({ "git", "init", "-q" }, root)
  run({ "git", "config", "user.email", "test@example.com" }, root)
  run({ "git", "config", "user.name", "Test" }, root)

  local function make_dir(rel)
    vim.fn.mkdir(root .. "/" .. rel, "p")
  end
  local function rename_file(old_rel, new_rel)
    make_dir(new_rel:match("^(.*)/") or ".")
    assert(vim.fn.rename(root .. "/" .. old_rel, root .. "/" .. new_rel) == 0,
      "rename failed: " .. old_rel .. " -> " .. new_rel)
  end

  local control_old_dir = "ctrl\nold.dir"
  local control_new_dir = "ctrl\tnew.dir"
  local literal_old_dir = "src.literal"
  local literal_new_dir = "lit\\nold.dir"
  local newline_old_dir = "src.newline"
  local newline_new_dir = "lit\nnew.dir"

  make_dir(control_old_dir .. "/nested")
  write(root .. "/" .. control_old_dir .. "/nested/control.rs",
    "pub fn control() -> i32 { 1 }\n")
  make_dir(literal_old_dir)
  write(root .. "/" .. literal_old_dir .. "/literal.rs",
    "pub fn literal() -> i32 { 1 }\n")
  make_dir(newline_old_dir)
  write(root .. "/" .. newline_old_dir .. "/newline.rs",
    "pub fn newline() -> i32 { 1 }\n")
  run({ "git", "add", "." }, root)
  run({ "git", "commit", "-qm", "tree label sources" }, root)

  rename_file(control_old_dir .. "/nested/control.rs",
    control_new_dir .. "/nested/control.rs")
  rename_file(literal_old_dir .. "/literal.rs", literal_new_dir .. "/literal.rs")
  rename_file(newline_old_dir .. "/newline.rs", newline_new_dir .. "/newline.rs")
  run({ "git", "config", "diff.renames", "false" }, root)

  vim.cmd("cd " .. vim.fn.fnameescape(root))
  vim.cmd("DiffviewOpen")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.files and #current.files.working == 3
  end, 20), "controlled directory moves did not load")
  local current = lib.get_current_view()

  local function entry_at(target)
    for _, entry in ipairs(current.files.working) do
      if entry.path == target then return entry end
    end
  end
  local control_target = control_new_dir .. "/nested/control.rs"
  local literal_target = literal_new_dir .. "/literal.rs"
  local newline_target = newline_new_dir .. "/newline.rs"
  local control_entry = entry_at(control_target)
  local literal_entry = entry_at(literal_target)
  local newline_entry = entry_at(newline_target)
  assert(control_entry and control_entry.oldpath == control_old_dir .. "/nested/control.rs")
  assert(literal_entry and literal_entry.oldpath == literal_old_dir .. "/literal.rs")
  assert(newline_entry and newline_entry.oldpath == newline_old_dir .. "/newline.rs")

  local function dir_node(raw)
    local node = current.files.working_tree.root
    for part in raw:gmatch("[^/]+") do
      node = node.children[part]
      if not node then return nil end
    end
    return node
  end

  local control_node = dir_node(control_new_dir)
  assert(control_node and control_node.data.path == control_new_dir,
    "control directory lost its raw tree path")
  assert(control_node.data.name == "ctrl\\tnew.dir",
    "control directory label is not display-escaped")
  local nested_node = dir_node(control_new_dir .. "/nested")
  assert(nested_node and nested_node.data.path == control_new_dir .. "/nested",
    "nested control directory lost its raw tree path")
  local schema = current.files.working_tree:create_comp_schema({ flatten_dirs = true })
  local flattened_label
  local function find_flattened(components)
    for _, child in ipairs(components) do
      if type(child) == "table" then
        if child.name == "directory" and child.context
            and child.context.path == control_new_dir .. "/nested" then
          flattened_label = child.context.name
        end
        find_flattened(child)
      end
    end
  end
  find_flattened(schema)
  assert(flattened_label == "ctrl\\tnew.dir/nested",
    "flattened control directory label is not display-escaped")

  local literal_node = dir_node(literal_new_dir)
  assert(literal_node and literal_node.data.path == literal_new_dir,
    "literal-backslash directory lost its raw tree path")
  assert(literal_node.data.name == "lit\\\\nold.dir",
    "literal-backslash directory label is not display-escaped")

  local newline_node = dir_node(newline_new_dir)
  assert(newline_node and newline_node.data.path == newline_new_dir,
    "newline directory lost its raw tree path")
  assert(newline_node.data.name == "lit\\nnew.dir",
    "newline directory label is not display-escaped")
  assert(current.files.working_tree.root.children[control_new_dir] == control_node
      and current.files.working_tree.root.children[literal_new_dir] == literal_node
      and current.files.working_tree.root.children[newline_new_dir] == newline_node,
    "tree node keys are not raw path components")

  vim.cmd("DiffviewClose")
  vim.cmd("silent! %bwipeout!")
  vim.fn.delete(root, "rf")
end
if test_case == "move-source-cycle" then
  local root = vim.fn.tempname()
  vim.fn.mkdir(root, "p")
  root = vim.fn.resolve(root)
  run({ "git", "init", "-q" }, root)
  run({ "git", "config", "user.email", "test@example.com" }, root)
  run({ "git", "config", "user.name", "Test" }, root)

  local old_dir = "ctrl\nold.dir"
  local new_dir = "ctrl\tnew.dir"
  local display_dir = "ctrl\\tnew.dir"
  local old_path = old_dir .. "/src.rs"
  local new_path = new_dir .. "/src.rs"
  local display_path = display_dir .. "/src.rs"
  vim.fn.mkdir(root .. "/" .. old_dir, "p")
  write(root .. "/" .. old_path, "pub fn moved() -> i32 { 1 }\n")
  run({ "git", "add", "." }, root)
  run({ "git", "commit", "-qm", "source cycle base" }, root)
  vim.fn.mkdir(root .. "/" .. new_dir, "p")
  assert(vim.fn.rename(root .. "/" .. old_path, root .. "/" .. new_path) == 0)
  run({ "git", "add", "-A" }, root)

  vim.cmd("cd " .. vim.fn.fnameescape(root))
  vim.cmd("DiffviewOpen")
  local lib = require("diffview.lib")
  assert(vim.wait(15000, function()
    local current = lib.get_current_view()
    return current and current.files and #current.files.staged == 1
      and current.files.staged[1].status == "R"
  end, 20), "source-cycle exact rename did not load")
  local current = lib.get_current_view()
  local entry = current.files.staged[1]
  assert(entry.path == new_path and entry.oldpath == old_path and entry.status == "R")
  assert(entry._codect_display_path == display_path and entry._codect_native_display,
    "focused exact rename was not display-overridden")
  assert(entry.parent_path == display_dir and entry.basename == "src.rs",
    "focused exact rename metadata is not display-escaped")
  assert(entry.layout.b.file.winbar:find(display_dir, 1, true),
    "focused exact rename winbar is not display-escaped")
  local tree = current.files.staged_tree
  local node = tree.root.children[new_dir]
  assert(node and node.data.path == new_dir and node.data.name == display_dir,
    "focused exact rename directory label is wrong")

  local function tree_has_label(node, label)
    for _, child in ipairs(node.children) do
      if child.data and child.data.name == label then return true end
      if child:has_children() and tree_has_label(child, label) then return true end
    end
    return false
  end

  vim.cmd("CodectDiffview source")
  assert(vim.wait(15000, function()
    local source_entry = current.files.staged[1]
    return source_entry and source_entry._codect_display_path == nil
      and source_entry._codect_native_display == nil
      and source_entry.path ~= display_path
      and not tree_has_label(current.files.staged_tree.root, display_dir)
  end, 20), "source mode kept Codect display overrides")
  entry = current.files.staged[1]
  assert(entry.path ~= display_path and entry.parent_path ~= display_dir,
    "source mode did not return to Diffview's native path identity")
  assert(entry.layout.b.file.path == entry.path,
    "source mode pane does not use Diffview's native entry path")
  assert(not tree_has_label(current.files.staged_tree.root, display_dir),
    "source mode tree still used a Codect display label")

  vim.cmd("CodectDiffview signatures")
  assert(vim.wait(15000, function()
    local focused = current.files.staged[1]
    local focused_node = current.files.staged_tree.root.children[new_dir]
    return focused and focused._codect_display_path == display_path
      and focused.parent_path == display_dir
      and focused_node and focused_node.data.name == display_dir
  end, 20), "return to focused mode did not reapply display metadata")
  entry = current.files.staged[1]
  assert(entry.parent_path == display_dir and entry.parent_path ~= "ctrl\\\\tnew.dir",
    "return to focused mode double-escaped the directory label")
  assert(entry.layout.b.file.winbar:find(display_dir, 1, true),
    "return to focused mode did not reapply the display winbar")
  vim.cmd("DiffviewClose")
  vim.cmd("silent! %bwipeout!")
  vim.fn.delete(root, "rf")
end
print("Diffview integration passed")
