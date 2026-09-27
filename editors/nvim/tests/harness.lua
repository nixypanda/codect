-- OwnAI Neovim plugin: minimal headless test harness (no plugin deps).
--
-- Each spec file is `return function(H) ... end` and registers tests through
-- `H.test`. run.lua loads the specs and calls H.run().

local H = {}

H.root = nil
H.repo = nil
H.fixture_rel = "editors/nvim/tests/fixtures/sample.rs"
H.tests = {}

function H.test(name, fn)
  H.tests[#H.tests + 1] = { name = name, fn = fn }
end

local function fmt(value)
  return vim.inspect(value)
end

function H.eq(actual, expected, message)
  if actual ~= expected then
    error(string.format("%s: expected %s, got %s", message or "eq", fmt(expected), fmt(actual)))
  end
end

function H.truthy(value, message)
  if not value then
    error((message or "expected truthy value") .. ": got " .. fmt(value))
  end
end

function H.falsy(value, message)
  if value then
    error((message or "expected falsy value") .. ": got " .. fmt(value))
  end
end

function H.contains(haystack, needle, message)
  if type(haystack) ~= "string" or not haystack:find(needle, 1, true) then
    error(string.format("%s: %q does not contain %q", message or "contains", tostring(haystack), needle))
  end
end

function H.state()
  return require("ownai.state").get(vim.api.nvim_get_current_buf())
end

--- Open the fixture in a clean buffer and clear its sticky overrides.
function H.open_fixture()
  vim.cmd("silent! %bwipeout!")
  require("ownai.state").clear_overrides(H.fixture_rel)
  vim.cmd("edit " .. H.fixture_rel)
  return vim.api.nvim_get_current_buf()
end

--- Open an arbitrary repository-relative file in a clean buffer.
function H.open_path(rel)
  vim.cmd("silent! %bwipeout!")
  require("ownai.state").clear_overrides(rel)
  vim.cmd("edit " .. vim.fn.fnameescape(rel))
  return vim.api.nvim_get_current_buf()
end

--- A fold-state fingerprint: every line's closed-fold start and fold text.
function H.snapshot()
  local count = vim.api.nvim_buf_line_count(0)
  local lines = {}
  for line = 1, count do
    lines[#lines + 1] = string.format("%d:%s", vim.fn.foldclosed(line), vim.fn.foldtextresult(line))
  end
  return table.concat(lines, "\n")
end

--- The declaration whose fold starts at `line`.
function H.item_at(line)
  local st = H.state()
  for _, item in ipairs(st.items or {}) do
    if item.fold_start == line then
      return item
    end
  end
  return nil
end

function H.item_named(name)
  local st = H.state()
  for _, item in ipairs(st.items or {}) do
    if item.name == name then
      return item
    end
  end
  return nil
end

function H.item_by_key(key)
  local st = H.state()
  return st.by_key and st.by_key[key] or nil
end

--- Lines that begin a closed fold.
function H.closed_starts()
  local starts = {}
  local last = vim.api.nvim_buf_line_count(0)
  for line = 1, last do
    if vim.fn.foldclosed(line) == line then
      starts[#starts + 1] = line
    end
  end
  return starts
end

function H.fold_text(item)
  return vim.fn.foldtextresult(item.fold_start)
end

--- Run `fn` with vim.notify captured. Returns the list of messages.
function H.capture_notify(fn)
  local messages = {}
  local original = vim.notify
  vim.notify = function(message)
    messages[#messages + 1] = message
  end
  local ok, err = pcall(fn)
  vim.notify = original
  if not ok then
    error(err)
  end
  return messages
end

--- Write an executable stub and point the plugin at it. Returns a restore fn.
function H.with_stub_binary(body)
  local path = vim.fn.tempname() .. ".sh"
  local file = assert(io.open(path, "w"))
  file:write("#!/bin/sh\ncat >/dev/null\n")
  file:write(body)
  file:write("\n")
  file:close()
  vim.fn.setfperm(path, "rwxr-xr-x")
  local previous = vim.g.ownai_binary
  vim.g.ownai_binary = path
  return function()
    vim.g.ownai_binary = previous
    vim.fn.delete(path)
  end
end

--- A stub that records that it ran by creating `marker`, then exits 0.
function H.with_marker_binary(marker)
  return H.with_stub_binary(string.format("printf called > %s", vim.fn.shellescape(marker)))
end

function H.run()
  local passed, failed = 0, 0
  local failures = {}
  for _, case in ipairs(H.tests) do
    local ok, err = pcall(case.fn)
    if ok then
      passed = passed + 1
      print(string.format("ok   - %s", case.name))
    else
      failed = failed + 1
      failures[#failures + 1] = { name = case.name, err = err }
      print(string.format("FAIL - %s\n       %s", case.name, tostring(err)))
    end
  end
  print(string.format("\n%d passed, %d failed, %d total", passed, failed, passed + failed))
  if failed > 0 then
    vim.cmd("cquit 1")
  else
    vim.cmd("qa!")
  end
end

return H
