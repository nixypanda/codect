-- ownai.folds: fold ranges, foldexpr/foldtext, and open/close.
--
-- Folds are `foldmethod=expr`: `foldexpr` reads a cached line -> level map and
-- `foldtext` renders the OwnAI closed-fold fragment. Open/closed state is
-- applied explicitly with `:foldclose` so a declaration's state is independent
-- of its nesting depth, which lets the plugin keep containers open while
-- folding signatures, and honour sticky per-declaration overrides.

local state = require("ownai.state")
local outline = require("ownai.outline")

local M = {}

local FOLDEXPR = "v:lua.require'ownai.folds'.foldexpr(v:lnum)"
local FOLDTEXT = "v:lua.require'ownai.folds'.foldtext()"

--- The innermost declaration whose fold region contains `line`.
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

--- Build the line -> fold-level map for the buffer's current state.
---
--- Lines inside a declaration get the deepest enclosing declaration's level.
--- A declaration's first line is marked `>N` so adjacent siblings still form
--- separate folds. Lines outside every declaration return 0.
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

--- Recompute the fold map and apply open/closed state.
function M.apply(buf)
  local st = state.get(buf)
  if not st then
    return
  end

  st.linemap = M.compute_linemap(st)

  local win = vim.fn.bufwinid(buf)
  if win == -1 then
    -- Not displayed: the fold map is ready and folds resolve when shown.
    return
  end

  M.apply_window(st, win)
end

--- Apply window-local fold options and open/closed state in a window.
function M.apply_window(st, win)
  vim.api.nvim_win_call(win, function()
    vim.wo.foldmethod = "expr"
    vim.wo.foldexpr = FOLDEXPR
    vim.wo.foldenable = true
    vim.wo.foldtext = FOLDTEXT
    -- Reset to "everything open", then close what the mode/overrides require.
    vim.wo.foldlevel = 0
    vim.wo.foldlevel = 99
    for _, item in ipairs(st.items or {}) do
      if state.is_closed(st, item) then
        pcall(vim.cmd, string.format("%d,%dfoldclose", item.fold_start, item.end_line))
      end
    end
    vim.cmd("redraw")
  end)
end

--- Ensure the window-local fold options are set for a displayed buffer.
function M.attach_window(buf)
  local st = state.get(buf)
  if not st then
    return
  end
  M.apply_window(st, vim.api.nvim_get_current_win())
end

--- `foldexpr` implementation. Reads the cached map for the current buffer.
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

--- `foldtext` implementation. Renders the closed-fold fragment for the
--- declaration whose fold starts at `v:foldstart`.
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

--- User fold actions. `action` is one of:
---   "toggle" | "open" | "close" | "open_recursive" | "close_recursive"
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

--- Open (or close) a declaration by stable key, e.g. from the outline picker.
function M.set_open(buf, item, open)
  local st = state.get(buf)
  if not st then
    return
  end
  state.set_override(st.path, item.key, open)
  M.apply(buf)
end

--- Cycle fold depth: types -> signatures -> full -> types. Returns the new mode.
function M.next_mode(mode)
  if mode == "types" then
    return "signatures"
  elseif mode == "signatures" then
    return "full"
  end
  return "types"
end

return M
