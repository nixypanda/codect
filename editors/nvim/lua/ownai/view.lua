-- ownai.view: :OwnaiShow / :OwnaiFold / :OwnaiOutline and refresh wiring.

local cli = require("ownai.cli")
local folds = require("ownai.folds")
local state = require("ownai.state")

local M = {}

local VALID_MODES = { types = true, signatures = true, full = true }

--- Leaf members that are declarations but are not useful ]f/[f targets.
local NAV_SKIP = { field = true, variant = true }

local function config()
  return require("ownai").config
end

local function fail(message)
  vim.notify("OwnAI: " .. message, vim.log.levels.ERROR)
  return nil
end

--- Resolve the buffer's file to a repository root and repo-relative path.
---
--- Returns `{ root, path, abs }`, or nil plus a clear error for a non-file
--- buffer, an unnamed buffer, or a file outside a Git repository.
function M.resolve_target(buf)
  buf = buf or vim.api.nvim_get_current_buf()

  local buftype = vim.bo[buf].buftype
  if buftype ~= "" then
    return nil, string.format("this buffer is not a file (buftype=%q)", buftype)
  end

  local name = vim.api.nvim_buf_get_name(buf)
  if name == "" then
    return nil, "this buffer has no file name; write it first"
  end

  local abs = vim.fn.resolve(vim.fn.fnamemodify(name, ":p"))
  local dir = vim.fn.fnamemodify(abs, ":h")
  local root_raw = vim.fn.system({ "git", "-C", dir, "rev-parse", "--show-toplevel" })
  if vim.v.shell_error ~= 0 or vim.trim(root_raw) == "" then
    return nil, "this file is not inside a Git repository (" .. abs .. ")"
  end

  local root = vim.fn.resolve(vim.trim(root_raw))
  if abs == root then
    return nil, "cannot project a directory"
  end
  local prefix = root .. "/"
  if abs:sub(1, #prefix) ~= prefix then
    return nil, "this file is outside the repository " .. root
  end

  return { root = root, path = abs:sub(#prefix + 1), abs = abs }
end

--- Exact buffer bytes, normalized with a trailing newline.
function M.buffer_source(buf)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  if #lines == 0 then
    return ""
  end
  return table.concat(lines, "\n") .. "\n"
end

local function find_file(doc, path)
  for _, file in ipairs(doc.files or {}) do
    if file.path == path then
      return file
    end
  end
  local files = doc.files or {}
  if #files == 1 then
    return files[1]
  end
  return nil
end

--- Project the current buffer and fold it to `mode`.
function M.show(mode)
  local buf = vim.api.nvim_get_current_buf()
  mode = mode or config().default_mode or "signatures"
  if not VALID_MODES[mode] or mode == "full" then
    return fail(string.format(":OwnaiShow expects `types` or `signatures`, got %q", mode))
  end

  local target, target_err = M.resolve_target(buf)
  if not target then
    return fail(target_err)
  end

  local source = M.buffer_source(buf)
  local doc, cli_err = cli.show({
    root = target.root,
    path = target.path,
    mode = mode,
    source = source,
  })
  if not doc then
    return fail(cli_err)
  end

  local file_entry = find_file(doc, target.path)
  if not file_entry then
    return fail("ownai returned no projection for " .. target.path)
  end

  local st = state.install(buf, doc, file_entry, mode, target.root, target.path)
  st.source_hash = vim.fn.sha256(source)
  require("ownai").attach_keymaps(buf)
  folds.apply(buf)
  return st
end

--- Re-fold locally with no OwnAI call.
function M.fold(mode)
  local buf = vim.api.nvim_get_current_buf()
  mode = mode or config().default_mode or "signatures"
  if not VALID_MODES[mode] then
    return fail(string.format(":OwnaiFold expects `types`, `signatures`, or `full`, got %q", mode))
  end

  local st = state.get(buf)
  if not st then
    -- Nothing projected yet: a real projection is the only source of outline.
    if mode == "full" then
      return fail("nothing to unfold; run :OwnaiShow first")
    end
    return M.show(mode)
  end

  state.set_mode(buf, mode)
  folds.apply(buf)
  return st
end

--- Cycle fold depth: types -> signatures -> full -> types.
function M.cycle()
  local buf = vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  local current = st and st.mode or "types"
  local next_mode = folds.next_mode(current)
  if next_mode == "full" then
    return M.fold("full")
  end
  return M.show(next_mode)
end

--- Refresh an active buffer: re-project only when its bytes changed.
function M.refresh(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st or not vim.api.nvim_buf_is_valid(buf) then
    return
  end

  local target, target_err = M.resolve_target(buf)
  if not target then
    vim.notify("OwnAI refresh skipped: " .. target_err, vim.log.levels.WARN)
    return
  end

  local source = M.buffer_source(buf)
  local hash = vim.fn.sha256(source)
  if hash == st.source_hash then
    folds.apply(buf)
    return
  end

  local doc, cli_err = cli.show({
    root = target.root,
    path = target.path,
    mode = st.mode,
    source = source,
  })
  if not doc then
    vim.notify("OwnAI refresh failed: " .. cli_err, vim.log.levels.WARN)
    return
  end

  local file_entry = find_file(doc, target.path)
  if not file_entry then
    vim.notify("OwnAI refresh returned no projection for " .. target.path, vim.log.levels.WARN)
    return
  end

  state.install(buf, doc, file_entry, st.mode, target.root, target.path)
  st.source_hash = hash
  folds.apply(buf)
end

local generations = {}

--- Debounce a refresh so the CLI is never invoked per keystroke.
function M.schedule_refresh(buf)
  generations[buf] = (generations[buf] or 0) + 1
  local generation = generations[buf]
  local delay = config().debounce_ms or 200
  vim.defer_fn(function()
    if generations[buf] ~= generation then
      return
    end
    if vim.api.nvim_buf_is_valid(buf) then
      M.refresh(buf)
    end
  end, delay)
end

--- Move to the next/previous declaration. `direction` is 1 or -1.
function M.goto_declaration(direction)
  local buf = vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st or not st.items or #st.items == 0 then
    return
  end

  local sorted = {}
  for _, item in ipairs(st.items) do
    if not NAV_SKIP[item.kind] then
      sorted[#sorted + 1] = item
    end
  end
  table.sort(sorted, function(a, b)
    if a.fold_start == b.fold_start then
      return a.level > b.level
    end
    return a.fold_start < b.fold_start
  end)

  local cursor = vim.api.nvim_win_get_cursor(0)[1]
  local target
  if direction > 0 then
    for _, item in ipairs(sorted) do
      if item.fold_start > cursor then
        target = item
        break
      end
    end
  else
    for index = #sorted, 1, -1 do
      local item = sorted[index]
      if item.fold_start < cursor then
        target = item
        break
      end
    end
  end

  if not target then
    return
  end
  vim.api.nvim_win_set_cursor(0, { target.fold_start, 0 })
  vim.cmd("normal! zv")
end

--- Declaration picker using `vim.ui.select` (no plugin dependencies).
function M.outline()
  local buf = vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st or not st.items or #st.items == 0 then
    return fail("no declarations; run :OwnaiShow first")
  end

  local sorted = {}
  for _, item in ipairs(st.items) do
    sorted[#sorted + 1] = item
  end
  table.sort(sorted, function(a, b)
    if a.fold_start == b.fold_start then
      return a.level > b.level
    end
    return a.fold_start < b.fold_start
  end)

  local labels = {}
  local by_label = {}
  for _, item in ipairs(sorted) do
    local label = string.format(
      "%s%-16s %s  (line %d)",
      string.rep("  ", item.depth),
      item.kind,
      item.name,
      item.fold_start
    )
    labels[#labels + 1] = label
    by_label[label] = item
  end

  vim.ui.select(labels, { prompt = "OwnAI declarations" }, function(choice)
    if not choice then
      return
    end
    local item = by_label[choice]
    if not item then
      return
    end
    vim.api.nvim_win_set_cursor(0, { item.fold_start, 0 })
    vim.cmd("normal! zv")
  end)
end

--- Register refresh and window autocmds.
function M.setup_autocmds()
  local group = vim.api.nvim_create_augroup("Ownai", { clear = true })

  vim.api.nvim_create_autocmd({ "BufWritePost", "InsertLeave" }, {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        M.refresh(event.buf)
      end
    end,
    desc = "OwnAI: refresh folds after a write or leaving insert",
  })

  vim.api.nvim_create_autocmd("TextChanged", {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        M.schedule_refresh(event.buf)
      end
    end,
    desc = "OwnAI: debounced refresh after edits",
  })

  vim.api.nvim_create_autocmd({ "BufWinEnter", "WinEnter" }, {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        folds.attach_window(event.buf)
      end
    end,
    desc = "OwnAI: restore the window-local foldtext",
  })

  vim.api.nvim_create_autocmd("BufWipeout", {
    group = group,
    callback = function(event)
      state.clear(event.buf)
    end,
    desc = "OwnAI: drop per-buffer state",
  })
end

return M
