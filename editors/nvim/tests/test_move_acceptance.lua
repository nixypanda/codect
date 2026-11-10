-- Independent acceptance coverage for file moves in the pinned Diffview adapter.
--
-- This intentionally lives outside run.lua: it needs the pinned Diffview
-- checkout and drives real Git repositories, like test_diffview.lua. Run it
-- from the repository root:
--
--   CODECT_DIFFVIEW_RTP=/path/to/diffview.nvim \
--   CODECT_BIN="$PWD/target/debug/codect" \
--   nvim --headless -u NONE -l editors/nvim/tests/test_move_acceptance.lua
--
-- Gaps covered here, beyond test_diffview.lua:
--   * Tests mode: a non-test-only move is absent; a move that gains or loses
--     its last test remains one move with the correct one-sided projection.
--   * A body-only edit within a move is a move with equal focused panes and no
--     content hunks in Signatures and Tests.
--   * A modified real worktree buffer survives source -> focused -> close.
--   * Source mode restores real source bytes for staged moves, and switching
--     back to Tests re-filters the staged move list.
--   * A constructed staged move (Git disabled rename detection, Codect still
--     pairs) binds its target side to stage-0 index, never LOCAL.
--   * Show path selection: the current binary marks its display encoding; a
--     marked doc, a legacy unmarked doc, and an unknown marker are handled
--     with a literal-backslash/tab wrong-file decoy. This is the same shared
--     path decoder the Diffview adapter uses, but exercised through :CodectShow.

local diffview_path = vim.env.CODECT_DIFFVIEW_RTP
if not diffview_path or diffview_path == "" then
  print("move acceptance skipped (set CODECT_DIFFVIEW_RTP)")
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

local function read_lines(path)
  local f = assert(io.open(path, "rb"))
  local value = f:read("*a")
  f:close()
  return value
end

local function buf_lines(bufnr)
  return table.concat(vim.api.nvim_buf_get_lines(bufnr, 0, -1, false), "\n")
end

local function wait_for(ms, fn, why)
  assert(vim.wait(ms, fn, 20), why)
end

local function find_by_path(files, path)
  for _, file in ipairs(files or {}) do
    if file.path == path then return file end
  end
end

local function assert_move(entry, old_path, new_path)
  assert(entry, "missing move entry for " .. new_path)
  assert(entry.path == new_path, "move target is " .. tostring(entry.path) .. ", want " .. new_path)
  assert(entry.oldpath == old_path, "move source is " .. tostring(entry.oldpath) .. ", want " .. old_path)
  assert(entry.status == "R", "move status is " .. tostring(entry.status) .. ", want R")
  assert(entry.layout.a and entry.layout.b, "move " .. new_path .. " does not have two layout sides")
  assert(not entry.layout.a.file.nulled and not entry.layout.b.file.nulled,
    "move " .. new_path .. " has a null pane")
end

local function wait_projected(entry, why)
  wait_for(10000, function()
    return entry.layout.a.file.bufnr and entry.layout.b.file.bufnr
      and vim.api.nvim_buf_is_loaded(entry.layout.a.file.bufnr)
      and vim.api.nvim_buf_is_loaded(entry.layout.b.file.bufnr)
  end, why .. ": projection buffers did not load")
end

-- Source lists replace focused projections asynchronously. A focused entry can
-- still be present when the mode switches, so source-shape checks require the
-- absence of every adapter projection, not just a matching path.
local RevType = require("diffview.vcs.rev").RevType
local function all_nonprojected(files)
  for _, entry in ipairs(files or {}) do
    for _, file in ipairs(entry.layout:files()) do
      if file._codect_projection ~= nil then return false end
    end
  end
  return true
end

-- Diffview auto-selects a file when source mode updates the list. Refetch the
-- displayed layout each tick: the redispatched source list replaces the
-- focused entry and its pending buffers. Let that asynchronous read finish
-- before selecting another entry; otherwise teardown reports the still-loading
-- auto-selected buffer as invalidated.
local function wait_auto_source(view, why)
  wait_for(10000, function()
    local file = view.panel and view.panel.cur_file
    local layout = view.cur_layout
    if type(file) == "table" and file.layout and file.layout:is_files_loaded() then
      layout = file.layout
    end
    if not layout or not layout:is_files_loaded() then return false end
    for _, file in ipairs(layout:files()) do
      if not file.nulled then
        local bufnr = file.bufnr
        if not bufnr or not vim.api.nvim_buf_is_loaded(bufnr) then return false end
        if vim.trim(buf_lines(bufnr)) == "" then return false end
      end
    end
    return true
  end, why .. ": displayed source file did not finish loading")
end

local function rust_source(marker, name, with_test, offset)
  local lines = {}
  -- Enough unique bytes keep the intended old/new pair well above 50% while
  -- old files with the opposite test shape stay unrelated to each other.
  for i = 1, 8 do
    lines[#lines + 1] = "// " .. marker .. " unique marker " .. i .. " "
      .. string.rep(marker:sub(1, 1), 12 + i)
  end
  lines[#lines + 1] = "pub fn " .. name .. "(x: i32) -> i32 {"
  lines[#lines + 1] = "    let value = x + " .. offset .. ";"
  lines[#lines + 1] = "    value"
  lines[#lines + 1] = "}"
  if with_test then
    lines[#lines + 1] = ""
    lines[#lines + 1] = "#[test]"
    lines[#lines + 1] = "fn " .. name .. "_test() {"
    lines[#lines + 1] = "    assert_eq!(" .. name .. "(1), " .. (offset + 1) .. ");"
    lines[#lines + 1] = "}"
  end
  lines[#lines + 1] = ""
  return table.concat(lines, "\n")
end

local tmp = vim.fn.tempname()
vim.fn.mkdir(tmp, "p")
run({ "git", "init", "-q" }, tmp)
run({ "git", "config", "user.email", "test@example.com" }, tmp)
run({ "git", "config", "user.name", "Test" }, tmp)
-- Keep the fixture independent of the developer's global Git configuration.
run({ "git", "config", "diff.renames", "true" }, tmp)
run({ "git", "config", "status.showUntrackedFiles", "normal" }, tmp)

-- Distinct bodies keep the fuzzy pairing unambiguous: only the intended
-- old/new pair shares most of its bytes.
local plain = { old = "plain.rs", new = "plain_new.rs",
  source = rust_source("plain", "plain", false, 11) }
local testy = { old = "testy.rs", new = "testy_new.rs",
  source = rust_source("testy", "testy", true, 19) }
local gain = { old = "gain.rs", new = "gain_new.rs",
  before = rust_source("gain", "gain", false, 23),
  after = rust_source("gain", "gain", true, 23) }
local lose = { old = "lose.rs", new = "lose_new.rs",
  before = rust_source("lose", "lose", true, 29),
  after = rust_source("lose", "lose", false, 29) }

local body_old = rust_source("body", "body", true, 31)
local body_new = body_old:gsub("x %+ 31", "x + 37"):gsub("body%(1%), 32", "body(1), 38")
assert(body_new ~= body_old, "body edit fixture did not change")

write(tmp .. "/" .. plain.old, plain.source)
write(tmp .. "/" .. testy.old, testy.source)
write(tmp .. "/" .. gain.old, gain.before)
write(tmp .. "/" .. lose.old, lose.before)
write(tmp .. "/body_old.rs", body_old)
run({ "git", "add", "." }, tmp)
run({ "git", "commit", "-qm", "base" }, tmp)

-- Commit 2: an edited move whose signatures and tests are unchanged. Only the
-- body differs, so both requested-mode projections are equal.
run({ "git", "mv", "body_old.rs", "body_new.rs" }, tmp)
write(tmp .. "/body_new.rs", body_new)
run({ "git", "commit", "-qam", "body-only move" }, tmp)

-- Leave four unstaged moves (the destination is untracked) so the working
-- section exercises sole-live-side construction and Tests visibility.
for _, move in ipairs({ plain, testy, gain, lose }) do
  assert(vim.fn.rename(tmp .. "/" .. move.old, tmp .. "/" .. move.new) == 0,
    "could not rename " .. move.old)
end
write(tmp .. "/" .. gain.new, gain.after)
write(tmp .. "/" .. lose.new, lose.after)
assert(read_lines(tmp .. "/" .. plain.new) == plain.source, "plain move changed content")
assert(read_lines(tmp .. "/" .. testy.new) == testy.source, "testy move changed content")

vim.cmd("cd " .. vim.fn.fnameescape(tmp))
local lib = require("diffview.lib")

-- Working section: every move is one two-sided row, then a modified real
-- worktree buffer survives source -> focused and closing the view.
vim.cmd("DiffviewOpen")
wait_for(15000, function()
  local view = lib.get_current_view()
  return view and view.files and #view.files.working == 4
end, "working moves did not load")
local view = lib.get_current_view()
for _, move in ipairs({ plain, testy, gain, lose }) do
  assert_move(find_by_path(view.files.working, move.new), move.old, move.new)
end
local focused_plain = find_by_path(view.files.working, plain.new)
view:set_file(focused_plain)
wait_projected(focused_plain, "working focused move")

vim.cmd("CodectDiffview source")
-- Four removed tracked paths plus four untracked destinations. Focused moves
-- are R entries with projections; the real source view is eight D/A rows.
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.panel and not current.panel.updating
    and #current.files.working == 8
    and all_nonprojected(current.files.working)
end, "source mode did not replace focused working moves with Git delete/add rows")
view = lib.get_current_view()
for _, move in ipairs({ plain, testy, gain, lose }) do
  local deleted = find_by_path(view.files.working, move.old)
  local added = find_by_path(view.files.working, move.new)
  assert(deleted and deleted.status == "D" and deleted.oldpath == nil,
    "working source lost the delete row for " .. move.old)
  assert(added and added.status == "?" and added.oldpath == nil,
    "working source lost the untracked add row for " .. move.new)
  assert(deleted.layout.a.file.rev.type == RevType.STAGE
      and deleted.layout.b.file.nulled,
    "working source delete row does not read the index on its old side")
  assert(added.layout.a.file.nulled
      and added.layout.b.file.rev.type == RevType.LOCAL,
    "working source untracked row does not read the worktree on its new side")
end
wait_auto_source(view, "working source")
local source_entry = find_by_path(view.files.working, plain.new)
assert(source_entry and source_entry.status == "?" and source_entry.oldpath == nil,
  "working source untracked add entry changed shape before selection")
assert(source_entry.layout.b.file.rev.type == RevType.LOCAL
    and source_entry.layout.a.file.nulled,
  "working source add entry does not bind the real worktree side to LOCAL")
view:set_file(source_entry)
wait_for(10000, function()
  local bufnr = source_entry.layout.b.file.bufnr
  return bufnr and vim.api.nvim_buf_is_loaded(bufnr)
    and buf_lines(bufnr):find("pub fn plain", 1, true)
end, "real worktree buffer did not load")
local user_buffer = source_entry.layout.b.file.bufnr
assert(vim.bo[user_buffer].modifiable, "real worktree buffer is not editable")
vim.api.nvim_buf_set_lines(user_buffer, 0, -1, false, { "pub fn user_worktree_edit() {}" })
assert(vim.bo[user_buffer].modified, "worktree edit was not recorded")

vim.cmd("CodectDiffview signatures")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and not current.panel.updating
    and #current.files.working == 4
    and find_by_path(current.files.working, plain.new)
end, "focused mode did not restore working moves")
view = lib.get_current_view()
assert_move(find_by_path(view.files.working, plain.new), plain.old, plain.new)
assert(vim.api.nvim_buf_is_valid(user_buffer) and vim.bo[user_buffer].modified,
  "focused mode deleted a modified real worktree buffer")
vim.cmd("DiffviewClose")
assert(vim.api.nvim_buf_is_valid(user_buffer) and vim.bo[user_buffer].modified,
  "closing the view deleted a modified real worktree buffer")
vim.api.nvim_buf_delete(user_buffer, { force = true })

-- Staged section: exact moves in Signatures, Tests visibility for last-test
-- gain/loss, then source restore and back to Tests.
run({ "git", "add", "-A" }, tmp)
local name_status = run({ "git", "diff", "--cached", "--name-status", "-M", "HEAD" }, tmp)
local source_count = 0
for line in name_status:gmatch("[^\n]+") do source_count = source_count + 1 end
assert(source_count >= 4, "Git source view lost staged entries")

vim.cmd("DiffviewOpen")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and not current.panel.updating
    and #current.files.staged == 4 and #current.files.working == 0
end, "staged moves did not load")
view = lib.get_current_view()

local plain_entry = find_by_path(view.files.staged, plain.new)
assert_move(plain_entry, plain.old, plain.new)
view:set_file(plain_entry)
wait_projected(plain_entry, "signatures exact move")
local plain_left = buf_lines(plain_entry.layout.a.file.bufnr)
local plain_right = buf_lines(plain_entry.layout.b.file.bufnr)
assert(plain_left == plain_right and plain_left:find("pub fn plain", 1, true),
  "exact move panes are not equal focused signatures")
assert(not plain_left:find("let value", 1, true), "exact move leaked a function body")
assert(not vim.bo[plain_entry.layout.a.file.bufnr].modifiable
    and not vim.bo[plain_entry.layout.b.file.bufnr].modifiable
    and vim.bo[plain_entry.layout.a.file.bufnr].buftype == "nowrite"
    and vim.bo[plain_entry.layout.b.file.bufnr].buftype == "nowrite",
  "focused staged move reused a writable buffer")

vim.cmd("CodectDiffview tests")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and not current.panel.updating
    and #current.files.staged == 3
end, "Tests mode did not omit the non-test move")
view = lib.get_current_view()
assert(find_by_path(view.files.staged, plain.new) == nil,
  "Tests mode kept a non-test-only move")
assert_move(find_by_path(view.files.staged, testy.new), testy.old, testy.new)
assert_move(find_by_path(view.files.staged, gain.new), gain.old, gain.new)
assert_move(find_by_path(view.files.staged, lose.new), lose.old, lose.new)

local gain_entry = find_by_path(view.files.staged, gain.new)
view:set_file(gain_entry)
wait_projected(gain_entry, "Tests gain move")
local gain_left = buf_lines(gain_entry.layout.a.file.bufnr)
local gain_right = buf_lines(gain_entry.layout.b.file.bufnr)
assert(gain_left == "" and gain_right:find("fn gain_test", 1, true),
  "gaining a last test did not produce one move with an empty old Tests side")
assert(not gain_right:find("pub fn gain", 1, true) and not gain_right:find("let value", 1, true),
  "Tests gain move leaked non-test declarations or a body")

local lose_entry = find_by_path(view.files.staged, lose.new)
view:set_file(lose_entry)
wait_projected(lose_entry, "Tests loss move")
local lose_left = buf_lines(lose_entry.layout.a.file.bufnr)
local lose_right = buf_lines(lose_entry.layout.b.file.bufnr)
assert(lose_left:find("fn lose_test", 1, true) and lose_right == "",
  "losing the last test did not produce one move with an empty new Tests side")

local testy_entry = find_by_path(view.files.staged, testy.new)
view:set_file(testy_entry)
wait_projected(testy_entry, "Tests equal move")
local testy_left = buf_lines(testy_entry.layout.a.file.bufnr)
local testy_right = buf_lines(testy_entry.layout.b.file.bufnr)
assert(testy_left == testy_right and testy_left:find("fn testy_test", 1, true),
  "equal test-only move did not show both Tests sides")

vim.cmd("CodectDiffview signatures")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and not current.panel.updating
    and #current.files.staged == 4
    and find_by_path(current.files.staged, plain.new)
end, "Signatures mode did not restore the staged move")
view = lib.get_current_view()
plain_entry = find_by_path(view.files.staged, plain.new)
view:set_file(plain_entry)
wait_projected(plain_entry, "staged exact move before source")

vim.cmd("CodectDiffview source")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.panel and not current.panel.updating
    and #current.files.staged == source_count
    and all_nonprojected(current.files.staged)
end, "source mode did not replace focused staged moves with Git rename rows")
view = lib.get_current_view()
for _, entry in ipairs(view.files.staged) do
  assert(entry.status == "R" and entry.oldpath ~= nil,
    "staged source entry is not a Git rename row: " .. tostring(entry.path))
  assert(entry.layout.a.file.rev.type == RevType.COMMIT and not entry.layout.a.file.nulled,
    "staged source rename does not read the HEAD commit on its old side")
  assert(entry.layout.b.file.rev.type == RevType.STAGE
      and entry.layout.b.file.rev.stage == 0 and not entry.layout.b.file.nulled,
    "staged source rename does not read stage-0 index on its new side")
end
wait_auto_source(view, "staged source")
local restored = find_by_path(view.files.staged, plain.new)
assert(restored and restored.status == "R" and restored.oldpath == plain.old,
  "staged source rename lost the expected pair before selection")
assert(restored.layout.a.file.rev.type == RevType.COMMIT,
  "staged source old side is not a commit")
assert(restored.layout.b.file.rev.type == RevType.STAGE
    and restored.layout.b.file.rev.stage == 0,
  "staged source new side is not stage-0 index")
view:set_file(restored)
wait_for(10000, function()
  local bufnr = restored.layout.b.file.bufnr
  local old = restored.layout.a.file.bufnr
  return bufnr and old and vim.api.nvim_buf_is_loaded(bufnr)
    and vim.api.nvim_buf_is_loaded(old)
    and restored.layout:is_files_loaded()
    and buf_lines(bufnr):find("let value = x + 11;", 1, true)
    and buf_lines(old):find("let value = x + 11;", 1, true)
end, "restored staged source buffers did not load")
local restored_source = buf_lines(restored.layout.b.file.bufnr)
assert(restored_source:find("pub fn plain", 1, true)
    and restored_source:find("let value = x + 11;", 1, true),
  "source mode did not restore the staged source body")

vim.cmd("CodectDiffview tests")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and not current.panel.updating
    and #current.files.staged == 3
    and find_by_path(current.files.staged, gain.new)
end, "Tests mode did not re-filter after source restore")
view = lib.get_current_view()
gain_entry = find_by_path(view.files.staged, gain.new)
view:set_file(gain_entry)
wait_projected(gain_entry, "Tests re-filter move")
assert(buf_lines(gain_entry.layout.a.file.bufnr) == ""
    and buf_lines(gain_entry.layout.b.file.bufnr):find("fn gain_test", 1, true),
  "Tests re-filter after source restore is wrong")
vim.cmd("DiffviewClose")

-- History: a body-only edit inside a move has equal Signatures/Tests panes
-- (no focused hunks) but real differing source bytes.
vim.cmd("DiffviewFileHistory --range=HEAD~1..HEAD")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.panel and not current.panel.updating
    and #current.panel.entries == 1 and #current.panel.entries[1].files == 1
end, "body-only move history did not load")
view = lib.get_current_view()
local history_entry = view.panel.entries[1]
assert(not history_entry.nulled, "body-only move was hidden in Signatures")
local body_move = history_entry.files[1]
assert_move(body_move, "body_old.rs", "body_new.rs")
view:set_file(body_move)
wait_projected(body_move, "body-only history move")
local body_left = buf_lines(body_move.layout.a.file.bufnr)
local body_right = buf_lines(body_move.layout.b.file.bufnr)
assert(body_left == body_right and body_left:find("pub fn body", 1, true),
  "body-only move has content hunks or unequal signature panes")
assert(not body_left:find("let value", 1, true), "body-only move leaked a body")

vim.cmd("CodectDiffview tests")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.panel and not current.panel.updating
    and #current.panel.entries == 1 and #current.panel.entries[1].files == 1
    and not current.panel.entries[1].nulled
end, "Tests mode omitted the equal body-only move")
view = lib.get_current_view()
body_move = view.panel.entries[1].files[1]
assert_move(body_move, "body_old.rs", "body_new.rs")
view:set_file(body_move)
wait_projected(body_move, "body-only Tests move")
local body_test_left = buf_lines(body_move.layout.a.file.bufnr)
local body_test_right = buf_lines(body_move.layout.b.file.bufnr)
assert(body_test_left == body_test_right
    and body_test_left:find("fn body_test", 1, true),
  "body-only move Tests panes are not the equal test projection")

vim.cmd("CodectDiffview source")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.panel and not current.panel.updating
    and #current.panel.entries == 1
    and #current.panel.entries[1].files == 1
    and all_nonprojected(current.panel.entries[1].files)
    and find_by_path(current.panel.entries[1].files, "body_new.rs")
end, "source mode did not restore the body-only move")
view = lib.get_current_view()
wait_auto_source(view, "history source")
body_move = find_by_path(view.panel.entries[1].files, "body_new.rs")
assert(body_move and body_move.status == "R" and body_move.oldpath == "body_old.rs",
  "history source did not restore the Git rename pair")
assert(body_move.layout.a.file.rev.type == RevType.COMMIT
    and body_move.layout.b.file.rev.type == RevType.COMMIT,
  "history source rename does not read both commit revisions")
view:set_file(body_move)
wait_for(10000, function()
  local right = body_move.layout.b.file.bufnr
  local left = body_move.layout.a.file.bufnr
  return right and vim.api.nvim_buf_is_loaded(right)
    and body_move.layout:is_files_loaded()
    and buf_lines(right):find("let value = x + 37;", 1, true)
    and (not left or not vim.api.nvim_buf_is_loaded(left)
      or buf_lines(left):find("let value = x + 31;", 1, true))
end, "body-only source buffer did not load")
assert(buf_lines(body_move.layout.b.file.bufnr):find("let value = x + 37;", 1, true),
  "source mode did not restore the edited body")
if body_move.layout.a and body_move.layout.a.file and body_move.layout.a.file.bufnr
    and vim.api.nvim_buf_is_loaded(body_move.layout.a.file.bufnr) then
  assert(buf_lines(body_move.layout.a.file.bufnr):find("let value = x + 31;", 1, true),
    "source mode did not keep the original body on the old side")
end
vim.cmd("DiffviewClose")

-- Constructed staged move: Git reports a delete/add pair because rename
-- detection is disabled, while Codect still pairs the identical source. The
-- adapter must construct one R entry whose new side reads stage-0 index, not
-- the editable worktree rev.
local constructed_repo = vim.fn.tempname()
vim.fn.mkdir(constructed_repo, "p")
run({ "git", "init", "-q" }, constructed_repo)
run({ "git", "config", "user.email", "test@example.com" }, constructed_repo)
run({ "git", "config", "user.name", "Test" }, constructed_repo)
run({ "git", "config", "diff.renames", "false" }, constructed_repo)
local constructed_old = "constructed_old.rs"
local constructed_new = "constructed_new.rs"
local constructed_source = table.concat({
  "pub fn constructed(x: i32) -> i32 {",
  "    let value = x + 41;",
  "    value",
  "}",
  "",
}, "\n")
write(constructed_repo .. "/" .. constructed_old, constructed_source)
run({ "git", "add", "." }, constructed_repo)
run({ "git", "commit", "-qm", "constructed base" }, constructed_repo)
run({ "git", "mv", constructed_old, constructed_new }, constructed_repo)
local constructed_status = run(
  { "git", "diff", "--cached", "--name-status", "HEAD" },
  constructed_repo
)
assert(constructed_status:find("D\t" .. constructed_old, 1, true)
    and constructed_status:find("A\t" .. constructed_new, 1, true),
  "constructed staged fixture is not a Git delete/add pair: " .. constructed_status)

vim.cmd("cd " .. vim.fn.fnameescape(constructed_repo))
vim.cmd("DiffviewOpen")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.panel and not current.panel.updating
    and #current.files.staged == 1 and #current.files.working == 0
end, "constructed staged move did not load")
view = lib.get_current_view()
local constructed = find_by_path(view.files.staged, constructed_new)
assert_move(constructed, constructed_old, constructed_new)
assert(constructed.layout.a.file.rev.type == RevType.COMMIT,
  "constructed staged move old side is not the HEAD commit")
assert(constructed.layout.b.file.rev.type == RevType.STAGE
    and constructed.layout.b.file.rev.stage == 0,
  "constructed staged move new side is not stage-0 index (LOCAL is wrong)")
view:set_file(constructed)
wait_projected(constructed, "constructed staged move")
local constructed_left = buf_lines(constructed.layout.a.file.bufnr)
local constructed_right = buf_lines(constructed.layout.b.file.bufnr)
assert(constructed_left == constructed_right
    and constructed_left:find("pub fn constructed", 1, true),
  "constructed staged move panes are not equal signatures")
assert(not constructed_left:find("let value", 1, true),
  "constructed staged move leaked a body")
assert(not vim.bo[constructed.layout.a.file.bufnr].modifiable
    and not vim.bo[constructed.layout.b.file.bufnr].modifiable
    and vim.bo[constructed.layout.a.file.bufnr].buftype == "nowrite"
    and vim.bo[constructed.layout.b.file.bufnr].buftype == "nowrite",
  "constructed staged move reused writable source buffers")

vim.cmd("CodectDiffview source")
wait_for(15000, function()
  local current = lib.get_current_view()
  return current and current.files and current.panel and not current.panel.updating
    and #current.files.staged == 2
    and all_nonprojected(current.files.staged)
    and find_by_path(current.files.staged, constructed_old)
    and find_by_path(current.files.staged, constructed_new)
end, "constructed staged source did not restore the Git delete/add pair")
view = lib.get_current_view()
local constructed_deleted = find_by_path(view.files.staged, constructed_old)
local constructed_added = find_by_path(view.files.staged, constructed_new)
assert(constructed_deleted and constructed_deleted.status == "D"
    and constructed_deleted.oldpath == nil,
  "constructed staged source lost its delete row")
assert(constructed_deleted.layout.a.file.rev.type == RevType.COMMIT
    and constructed_deleted.layout.b.file.nulled,
  "constructed staged delete row does not read the HEAD commit")
assert(constructed_added and constructed_added.status == "A"
    and constructed_added.oldpath == nil,
  "constructed staged source lost its added row")
assert(constructed_added.layout.a.file.nulled
    and constructed_added.layout.b.file.rev.type == RevType.STAGE
    and constructed_added.layout.b.file.rev.stage == 0,
  "constructed staged added row does not read stage-0 index")
view:set_file(constructed_added)
wait_for(10000, function()
  local bufnr = constructed_added.layout.b.file.bufnr
  return bufnr and vim.api.nvim_buf_is_loaded(bufnr)
    and buf_lines(bufnr):find("let value = x + 41;", 1, true)
end, "constructed staged source buffer did not load")
vim.cmd("DiffviewClose")
vim.cmd("silent! %bwipeout!")
vim.cmd("cd " .. vim.fn.fnameescape(tmp))
vim.fn.delete(constructed_repo, "rf")

-- Show path selection. The current producer marks its injective display
-- encoding. A literal-backslash file and a tab-named file collide with the
-- opposite legacy/display interpretation, so a wrong decode picks the decoy.
local literal_raw = "a\\tb.rs"        -- one backslash, then t
local literal_display = "a\\\\tb.rs"  -- two backslashes, then t
local tab_raw = "a\tb.rs"             -- one tab byte
local tab_display = "a\\tb.rs"        -- one backslash, then t
assert(tab_display == literal_raw, "show decoy fixtures do not collide")
local literal_source = "pub fn literal_target() -> i32 { 1 }\n"
local tab_source = "pub fn tab_decoy() -> i32 { 2 }\n"
write(tmp .. "/" .. literal_raw, literal_source)
write(tmp .. "/" .. tab_raw, tab_source)

local function capture_notify(fn)
  local messages = {}
  local original = vim.notify
  vim.notify = function(message)
    messages[#messages + 1] = message
  end
  local ok, err = pcall(fn)
  vim.notify = original
  if not ok then error(err) end
  return messages
end

local function with_stub_binary(body)
  local stub = vim.fn.tempname() .. ".sh"
  local fd = assert(io.open(stub, "w"))
  fd:write("#!/bin/sh\ncat >/dev/null\n")
  fd:write(body)
  fd:write("\n")
  fd:close()
  vim.fn.setfperm(stub, "rwxr-xr-x")
  local previous = vim.g.codect_binary
  vim.g.codect_binary = stub
  return function()
    vim.g.codect_binary = previous
    vim.fn.delete(stub)
  end
end

local function open_abs(absolute)
  vim.cmd("silent! %bwipeout!")
  local buf = vim.fn.bufadd(absolute)
  vim.fn.bufload(buf)
  vim.api.nvim_set_current_buf(buf)
  return buf
end

local function stub_file(file_path, text)
  return {
    path = file_path,
    language = "rust",
    projection = { text = text, items = {} },
    outline = {},
  }
end

local function stub_show_doc(marker, files)
  local doc = {
    schema = "codect.show.v1",
    input = "stdin",
    revision = vim.NIL,
    mode = "signatures",
    files = files,
  }
  if marker ~= nil then doc.path_encoding = marker end
  return doc
end

local function stub_show(doc)
  return "printf '%s' " .. vim.fn.shellescape(vim.json.encode(doc))
end

local function show_on(absolute, why)
  local buf = open_abs(absolute)
  local messages = capture_notify(function()
    vim.cmd("CodectShow signatures")
  end)
  assert(#messages == 0, why .. ": " .. table.concat(messages, "\n"))
  local st = require("codect.state").get(buf)
  assert(st, why .. ": :CodectShow installed no state")
  return st
end

local binary = require("codect.cli").resolve_binary()
assert(binary, "codect binary not found for show path acceptance")
local function show_doc_from_binary(path_value, source_value, why)
  local result = vim.system(
    { binary, "show", "--format", "json", "--mode", "signatures", "--stdin", "--path", path_value },
    { cwd = tmp, text = true, stdin = source_value }
  ):wait()
  assert(result.code == 0, why .. ": " .. (result.stderr or ""))
  local ok, doc = pcall(vim.json.decode, result.stdout)
  assert(ok, why .. ": invalid JSON")
  return doc
end

local literal_doc = show_doc_from_binary(literal_raw, literal_source, "literal show")
assert(literal_doc.schema == "codect.show.v1")
assert(literal_doc.path_encoding == "codect.path.v1",
  "current binary did not emit path_encoding on show")
assert(#literal_doc.files == 1 and literal_doc.files[1].path == literal_display,
  "current binary did not display-encode a literal backslash path: "
    .. vim.inspect(literal_doc.files[1] and literal_doc.files[1].path))

local tab_doc = show_doc_from_binary(tab_raw, tab_source, "tab show")
assert(tab_doc.path_encoding == "codect.path.v1")
assert(#tab_doc.files == 1 and tab_doc.files[1].path == tab_display,
  "current binary did not display-encode a tab path: "
    .. vim.inspect(tab_doc.files[1] and tab_doc.files[1].path))

local invalid_raw = "a\xFFb.rs"
local invalid_display = "a\\xFFb.rs"
local invalid_doc = show_doc_from_binary(invalid_raw, literal_source, "invalid-byte show")
assert(invalid_doc.path_encoding == "codect.path.v1")
assert(#invalid_doc.files == 1 and invalid_doc.files[1].path == invalid_display,
  "current binary did not display-encode invalid UTF-8 bytes: "
    .. vim.inspect(invalid_doc.files[1] and invalid_doc.files[1].path))

local st = show_on(tmp .. "/" .. literal_raw, "marked literal lookup")
assert(st.path == literal_raw and st.file_entry.path == literal_display,
  "marked show did not decode the literal path while preserving its label")
assert(st.file_entry.projection.text:find("literal_target", 1, true),
  "marked show selected the wrong file for the literal path")

st = show_on(tmp .. "/" .. tab_raw, "marked tab lookup")
assert(st.path == tab_raw and st.file_entry.path == tab_display,
  "marked show did not decode the tab path while preserving its label")
assert(st.file_entry.projection.text:find("tab_decoy", 1, true),
  "marked show selected the wrong file for the tab path")

local decoy_files = {
  stub_file(tab_raw, "decoy"),
  stub_file(literal_raw, "target"),
}
local buf = open_abs(tmp .. "/" .. literal_raw)
local restore = with_stub_binary(stub_show(stub_show_doc(nil, decoy_files)))
local messages = capture_notify(function()
  vim.cmd("CodectShow signatures")
end)
restore()
assert(#messages == 0, "legacy show: " .. table.concat(messages, "\n"))
st = require("codect.state").get(buf)
assert(st and st.file_entry.projection.text == "target",
  "legacy unmarked show decoded a raw path and selected the decoy file")
assert(st.path == literal_raw and st.file_entry.path == literal_raw,
  "legacy unmarked show changed the raw path or its label")

local marked_decoy_files = {
  stub_file(tab_display, "decoy"),
  stub_file(literal_display, "target"),
}
buf = open_abs(tmp .. "/" .. literal_raw)
restore = with_stub_binary(stub_show(stub_show_doc("codect.path.v1", marked_decoy_files)))
messages = capture_notify(function()
  vim.cmd("CodectShow signatures")
end)
restore()
assert(#messages == 0, "marked decoy show: " .. table.concat(messages, "\n"))
st = require("codect.state").get(buf)
assert(st and st.file_entry.projection.text == "target",
  "marked show selected the decoy for the literal path")
assert(st.file_entry.path == literal_display, "marked show did not preserve the encoded label")

buf = open_abs(tmp .. "/" .. tab_raw)
restore = with_stub_binary(stub_show(stub_show_doc("codect.path.v1", marked_decoy_files)))
messages = capture_notify(function()
  vim.cmd("CodectShow signatures")
end)
restore()
assert(#messages == 0, "marked tab decoy show: " .. table.concat(messages, "\n"))
st = require("codect.state").get(buf)
assert(st and st.file_entry.projection.text == "decoy",
  "marked show did not select the tab-named file")
assert(st.file_entry.path == tab_display, "marked tab show did not preserve the encoded label")

local unknown_doc = stub_show_doc("codect.path.v9", { stub_file(literal_display, "target") })
buf = open_abs(tmp .. "/" .. literal_raw)
restore = with_stub_binary(stub_show(unknown_doc))
messages = capture_notify(function()
  vim.cmd("CodectShow signatures")
end)
restore()
assert(#messages > 0, "unknown show path encoding did not fail")
local unknown_message = false
for _, message in ipairs(messages) do
  if message:find("unsupported Codect path encoding", 1, true) then unknown_message = true end
end
assert(unknown_message, "unknown show path encoding was not rejected explicitly: "
  .. table.concat(messages, "\n"))
assert(require("codect.state").get(buf) == nil,
  "unknown show path encoding installed projection state")

buf = open_abs(tmp .. "/" .. literal_raw)
restore = with_stub_binary(stub_show(unknown_doc))
messages = capture_notify(function()
  require("codect.view").enable("signatures")
end)
restore()
capture_notify(function()
  require("codect.view").disable()
end)
local auto_rejected = false
for _, message in ipairs(messages) do
  if message:find("path encoding", 1, true) then auto_rejected = true end
end
assert(auto_rejected, "auto-fold accepted an unknown show path encoding")
assert(require("codect.state").get(buf) == nil,
  "auto-fold installed state for an unknown show path encoding")

vim.cmd("silent! %bwipeout!")
vim.fn.delete(tmp, "rf")
print("move acceptance passed")
