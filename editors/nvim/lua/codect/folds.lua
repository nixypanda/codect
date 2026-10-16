-- codect.folds: fold ranges, foldexpr/foldtext, and open/close.
--
-- Folds are `foldmethod=expr`: `foldexpr` reads a cached line -> level map and
-- `foldtext` renders the Codect closed-fold fragment. Open/closed state is
-- applied explicitly with `:foldclose` so a declaration's state is independent
-- of its nesting depth, which lets the plugin keep containers open while
-- folding signatures, and honour sticky per-declaration overrides.
--
-- Apply folds ancestor-first and read each fold's real open/closed state back
-- from `foldclosed()`, so a re-apply neither re-closes a fold (which escalates)
-- nor leaves a should-be-open fold closed.

local state = require("codect.state")
local outline = require("codect.outline")

local M = {}

local FOLDEXPR = "v:lua.require'codect.folds'.foldexpr(v:lnum)"
local FOLDTEXT = "v:lua.require'codect.folds'.foldtext()"

function M.item_at(st, line)
  local best = nil
  for _, item in ipairs(st.items or {}) do
    if item.fold_start <= line and line <= item.end_line then
      if not best or item.level > best.level or (item.level == best.level and item.fold_start > best.fold_start) then
        best = item
      end
    end
  end
  return best
end

function M.compute_linemap(st)
  local line_count = vim.api.nvim_buf_line_count(st.buf)
  local map = {}

  for _, item in ipairs(st.items or {}) do
    local first = math.max(item.fold_start, 1)
    local last = math.min(item.end_line, line_count)
    for line = first, last do
      if (map[line] or 0) < item.level then
        map[line] = item.level
      end
    end
  end

  local starts = {}
  for _, item in ipairs(st.items or {}) do
    local line = item.fold_start
    if line >= 1 and line <= line_count then
      if not starts[line] or starts[line] < item.level then
        starts[line] = item.level
      end
    end
  end
  for line, level in pairs(starts) do
    if map[line] == level then
      map[line] = ">" .. level
    end
  end

  return map
end

function M.apply(buf)
  local st = state.get(buf)
  if not st then
    return
  end

  st.linemap = M.compute_linemap(st)

  local win = vim.fn.bufwinid(buf)
  if win == -1 then
    return
  end

  M.apply_window(st, win)
end

local saved_window_options = {}

local MANAGED_OPTIONS = {
  "foldmethod",
  "foldexpr",
  "foldtext",
  "foldenable",
  "foldminlines",
  "foldlevel",
}

local function save_window_options(win)
  if saved_window_options[win] then
    return
  end
  local saved = {}
  for _, name in ipairs(MANAGED_OPTIONS) do
    saved[name] = vim.wo[win][name]
  end
  saved_window_options[win] = saved
end

function M.restore_window(win)
  local saved = saved_window_options[win]
  if not saved then
    return
  end
  saved_window_options[win] = nil
  if not vim.api.nvim_win_is_valid(win) then
    return
  end
  for _, name in ipairs(MANAGED_OPTIONS) do
    vim.wo[win][name] = saved[name]
  end
end

function M.apply_window(st, win)
  vim.api.nvim_win_call(win, function()
    save_window_options(win)

    if require("codect").config.manage_fold_options ~= false then
      vim.wo.foldmethod = "expr"
      vim.wo.foldexpr = FOLDEXPR
      vim.wo.foldenable = true
      vim.wo.foldtext = FOLDTEXT
      -- Neovim refuses to close a fold over a single line at the default
      -- foldminlines, which would hide every one-line declaration.
      vim.wo.foldminlines = 0
    end

    -- Reset to "everything open" and force the fold cache to recompute. This
    -- is part of applying the fold state, not a configuration choice: without
    -- it, `:[a,b]foldclose` can act on a stale or already-closed fold and
    -- escalate, collapsing a retained container (visible when
    -- `manage_fold_options = false` skips the rest of the option setup).
    vim.wo.foldlevel = 0
    vim.wo.foldlevel = 99
    -- `zx` forces foldexpr re-evaluation. Setting `foldlevel` alone does not
    -- invalidate a fold cache that was populated while the option was low.
    pcall(vim.cmd, "normal! zx")

    local line_count = vim.api.nvim_buf_line_count(st.buf)

    local order = {}
    for _, item in ipairs(st.items or {}) do
      order[#order + 1] = item
    end
    table.sort(order, function(a, b)
      if a.level ~= b.level then
        return a.level < b.level
      end
      return a.fold_start < b.fold_start
    end)

    local effective_closed = {}
    for _, item in ipairs(order) do
      local first = math.max(item.fold_start or 0, 1)
      local last = math.min(item.end_line or 0, line_count)
      if first <= last then
        local ancestor_closed = false
        local parent = item.parent
        while parent do
          if effective_closed[parent] then
            ancestor_closed = true
            break
          end
          local parent_item = st.by_key and st.by_key[parent]
          parent = parent_item and parent_item.parent
        end

        if ancestor_closed then
          effective_closed[item.key] = true
        elseif state.is_closed(st, item) then
          effective_closed[item.key] = true
          if vim.fn.foldclosed(first) ~= first then
            pcall(vim.cmd, string.format("%d,%dfoldclose", first, last))
          end
        elseif vim.fn.foldclosed(first) == first then
          pcall(vim.cmd, string.format("%d,%dfoldopen", first, last))
        end
      end
    end
    vim.cmd("redraw")
  end)
end

function M.attach_window(buf)
  local st = state.get(buf)
  if not st then
    return
  end
  M.apply_window(st, vim.api.nvim_get_current_win())
end

function M.foldexpr(lnum)
  local st = state.get(vim.api.nvim_get_current_buf())
  if not st or not st.linemap then
    return 0
  end
  local value = st.linemap[lnum]
  if value == nil then
    return 0
  end
  return value
end

function M.foldtext()
  local st = state.get(vim.api.nvim_get_current_buf())
  if not st then
    return ""
  end
  local line = vim.v.foldstart
  local item = st.start_to_item and st.start_to_item[line] or nil
  if not item then
    item = M.item_at(st, line)
  end
  if not item then
    return ""
  end
  return item.closed_text or outline.first_line(item.signature)
end

local function set_descendants(st, item, open)
  for _, child in ipairs(item.children or {}) do
    state.set_override(st.path, child.key, open)
    set_descendants(st, child, open)
  end
end

function M.user_action(action)
  local buf = vim.api.nvim_get_current_buf()
  local st = state.get(buf)
  if not st then
    return
  end

  local line = vim.api.nvim_win_get_cursor(0)[1]
  local item = M.item_at(st, line)
  if not item then
    return
  end

  local currently_closed = state.is_closed(st, item)
  local open
  if action == "toggle" then
    open = currently_closed
  elseif action == "open" or action == "open_recursive" then
    open = true
  else
    open = false
  end

  state.set_override(st.path, item.key, open)
  if action == "open_recursive" or action == "close_recursive" then
    set_descendants(st, item, open)
  end

  M.apply(buf)
end

function M.set_open(buf, item, open)
  local st = state.get(buf)
  if not st then
    return
  end
  state.set_override(st.path, item.key, open)
  M.apply(buf)
end

function M.next_mode(mode)
  if mode == "types" then
    return "signatures"
  elseif mode == "signatures" then
    return "full"
  end
  return "types"
end

return M
