-- codect.view: :CodectShow / :CodectFold / :CodectOutline and refresh wiring.
--
-- Auto-fold state is module-level (it survives `setup()` re-invocation) but
-- session-scoped, `enable` accepts only `types`/`signatures` while `full` is the
-- local unfold target, and a failed `(path, hash, mode)` is remembered so a
-- re-entered unchanged buffer does not repeat the CLI call.

local cli = require("codect.cli")
local folds = require("codect.folds")
local state = require("codect.state")

local M = {}

local VALID_MODES = { types = true, signatures = true, full = true }

local AUTO_MODES = { types = true, signatures = true }

local NAV_SKIP = { field = true, variant = true }

local auto = {
  enabled = false,
  mode = nil,
  last_mode = nil,
  warned = false,
  folded = {},
  failed = {},
}

local function config()
  return require("codect").config
end

local function fail(message)
  vim.notify("Codect: " .. message, vim.log.levels.ERROR)
  return nil
end

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
  if vim.fn.isdirectory(abs) == 1 then
    return nil, "cannot project a directory"
  end
  local dir = vim.fn.fnamemodify(abs, ":h")
  -- A missing/unusable `git` is an ordinary ineligible result, not a thrown
  -- error: auto-fold must skip the buffer silently, while `:CodectShow` reports
  -- this message. `vim.fn.system` raises E475 for a non-executable cmd, so guard
  -- with `executable()` and `pcall` rather than letting it escape an autocmd.
  if vim.fn.executable("git") ~= 1 then
    return nil, "`git` is not executable; cannot find the repository root"
  end
  local ok, root_raw = pcall(vim.fn.system, { "git", "-C", dir, "rev-parse", "--show-toplevel" })
  if not ok then
    return nil, "`git` failed to run; cannot find the repository root"
  end
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

function M.buffer_source(buf)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  if #lines == 0 then
    return ""
  end
  if #lines == 1 and lines[1] == "" then
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

function M.ensure(buf, mode, target, hash, source)
  if not target then
    local target_err
    target, target_err = M.resolve_target(buf)
    if not target then
      return nil, target_err
    end
  end

  source = source or M.buffer_source(buf)
  hash = hash or vim.fn.sha256(source)

  local st = state.get(buf)
  if state.cached_doc(st, mode, hash, target.path) then
    state.set_mode(buf, mode)
    require("codect").attach_keymaps(buf)
    folds.apply(buf)
    return state.get(buf)
  end

  local doc, cli_err = cli.show({
    root = target.root,
    path = target.path,
    mode = mode,
    source = source,
  })
  if not doc then
    return nil, cli_err
  end

  local file_entry = find_file(doc, target.path)
  if not file_entry then
    return nil, "codect returned no projection for " .. target.path
  end

  st = state.install(buf, doc, file_entry, mode, target.root, target.path, hash)
  require("codect").attach_keymaps(buf)
  folds.apply(buf)
  return st
end

function M.show(mode)
  local buf = vim.api.nvim_get_current_buf()
  mode = mode or config().default_mode or "signatures"
  if not VALID_MODES[mode] or mode == "full" then
    return fail(string.format(":CodectShow expects `types` or `signatures`, got %q", mode))
  end

  local st, err = M.ensure(buf, mode)
  if not st then
    return fail(err)
  end

  return st
end

function M.fold(mode)
  local buf = vim.api.nvim_get_current_buf()
  mode = mode or config().default_mode or "signatures"
  if not VALID_MODES[mode] then
    return fail(string.format(":CodectFold expects `types`, `signatures`, or `full`, got %q", mode))
  end

  if mode == "full" then
    local st = state.get(buf)
    if not st then
      return fail("nothing to unfold; run :CodectShow first")
    end
    state.set_mode(buf, mode)
    folds.apply(buf)
    return st
  end

  local st, err = M.ensure(buf, mode)
  if not st then
    return fail(err)
  end
  return st
end

local function resolve_auto_mode(mode)
  mode = mode or auto.last_mode or config().default_mode or "signatures"
  if not AUTO_MODES[mode] then
    return nil, string.format(":CodectEnable expects `types` or `signatures`, got %q", mode)
  end
  return mode
end

local AUTO_FATAL_ERRORS = {
  "binary not found",
  "failed to run",
  "invalid json",
  "schema",
}

local function is_fatal_auto_error(err)
  if type(err) ~= "string" then
    return false
  end
  local lower = err:lower()
  for _, needle in ipairs(AUTO_FATAL_ERRORS) do
    if lower:find(needle, 1, true) then
      return true
    end
  end
  return false
end

local function warn_auto(err)
  if auto.warned then
    return
  end
  auto.warned = true
  vim.notify("Codect: auto-fold is unavailable: " .. err, vim.log.levels.WARN)
end

function M.auto_fold(buf)
  if not auto.enabled then
    return
  end

  local mode = auto.mode
  local st = state.get(buf)
  if st and st.active and st.mode == mode then
    return
  end

  local target = M.resolve_target(buf)
  if not target then
    return
  end

  local source = M.buffer_source(buf)
  local hash = vim.fn.sha256(source)

  local failed = auto.failed[buf]
  if failed and failed.path == target.path and failed.hash == hash and failed.mode == mode then
    return
  end

  local result, err = M.ensure(buf, mode, target, hash, source)
  if not result then
    if is_fatal_auto_error(err) then
      warn_auto(err)
    else
      auto.failed[buf] = { path = target.path, hash = hash, mode = mode }
    end
    return
  end

  auto.failed[buf] = nil
  auto.folded[buf] = true
  auto.warned = false
end

function M.enable(mode)
  local resolved, err = resolve_auto_mode(mode)
  if not resolved then
    return fail(err)
  end

  auto.enabled = true
  auto.mode = resolved
  auto.last_mode = resolved
  auto.warned = false
  auto.failed = {}

  M.auto_fold(vim.api.nvim_get_current_buf())

  vim.notify(string.format("Codect: auto-fold enabled (%s)", resolved), vim.log.levels.INFO)
  return true
end

function M.disable()
  auto.enabled = false
  auto.warned = false
  auto.failed = {}

  for buf in pairs(auto.folded) do
    if vim.api.nvim_buf_is_valid(buf) and state.is_active(buf) then
      state.set_mode(buf, "full")
      folds.apply(buf)
    end
  end
  auto.folded = {}

  vim.notify("Codect: auto-fold disabled", vim.log.levels.INFO)
  return true
end

function M.toggle(mode)
  if auto.enabled then
    M.disable()
    return false
  end
  return M.enable(mode) == true
end

function M.is_enabled()
  return auto.enabled
end

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

function M.refresh(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st or not vim.api.nvim_buf_is_valid(buf) then
    return
  end

  if st.mode == "full" then
    folds.apply(buf)
    return
  end

  local target, target_err = M.resolve_target(buf)
  if not target then
    vim.notify("Codect refresh skipped: " .. target_err, vim.log.levels.WARN)
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
    vim.notify("Codect refresh failed: " .. cli_err, vim.log.levels.WARN)
    return
  end

  local file_entry = find_file(doc, target.path)
  if not file_entry then
    vim.notify("Codect refresh returned no projection for " .. target.path, vim.log.levels.WARN)
    return
  end

  state.install(buf, doc, file_entry, st.mode, target.root, target.path, hash)
  folds.apply(buf)
end

local generations = {}

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

function M.outline()
  local buf = vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st or not st.items or #st.items == 0 then
    return fail("no declarations; run :CodectShow first")
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

  vim.ui.select(labels, { prompt = "Codect declarations" }, function(choice)
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

function M.setup_autocmds()
  local group = vim.api.nvim_create_augroup("Codect", { clear = true })

  vim.api.nvim_create_autocmd({ "BufWritePost", "InsertLeave" }, {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        M.refresh(event.buf)
      end
    end,
    desc = "Codect: refresh folds after a write or leaving insert",
  })

  vim.api.nvim_create_autocmd("TextChanged", {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        M.schedule_refresh(event.buf)
      end
    end,
    desc = "Codect: debounced refresh after edits",
  })

  vim.api.nvim_create_autocmd({ "BufWinEnter", "WinEnter" }, {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        folds.attach_window(event.buf)
      end
    end,
    desc = "Codect: restore the window-local foldtext",
  })

  vim.api.nvim_create_autocmd("BufLeave", {
    group = group,
    callback = function(event)
      if state.is_active(event.buf) then
        folds.restore_window(vim.api.nvim_get_current_win())
      end
    end,
    desc = "Codect: restore window-local fold options when the buffer leaves the window",
  })

  vim.api.nvim_create_autocmd("BufWipeout", {
    group = group,
    callback = function(event)
      local st = state.get(event.buf)
      if st and st.path then
        state.clear_overrides(st.path)
      end
      state.clear(event.buf)
      auto.folded[event.buf] = nil
      auto.failed[event.buf] = nil
    end,
    desc = "Codect: drop per-buffer state and sticky overrides",
  })

  vim.api.nvim_create_autocmd("BufReadPost", {
    group = group,
    callback = function(event)
      if not auto.enabled then
        return
      end
      local buf = event.buf
      vim.schedule(function()
        if vim.api.nvim_buf_is_valid(buf) then
          M.auto_fold(buf)
        end
      end)
    end,
    desc = "Codect: auto-fold a newly opened file when auto-fold is enabled",
  })

  vim.api.nvim_create_autocmd("BufWinEnter", {
    group = group,
    callback = function(event)
      if not auto.enabled then
        return
      end
      local buf = event.buf
      vim.schedule(function()
        if vim.api.nvim_buf_is_valid(buf) then
          M.auto_fold(buf)
        end
      end)
    end,
    desc = "Codect: auto-fold a buffer that enters a window when auto-fold is enabled",
  })
end

return M
