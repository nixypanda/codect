-- ownai.outline: turn the mode-independent declaration outline into a fold
-- tree and fold regions.
--
-- The document's `outline` is complete regardless of mode, so this module only
-- needs structure: parent links, nesting depth, a fold start, and an end line.

local M = {}

--- Declaration kinds that render as signatures: closed by default in every
--- non-full mode.
local SIGNATURE_KINDS = {
  ["function"] = true,
  method = true,
  value = true,
  constant = true,
  static = true,
  port = true,
  operator = true,
  foreign_block = true,
}

function M.is_signature_kind(kind)
  return SIGNATURE_KINDS[kind] == true
end

--- First line of a (possibly multi-line) canonical fragment, trimmed.
function M.first_line(text)
  if type(text) ~= "string" then
    return ""
  end
  local line = text:match("^[^\n]*") or ""
  return vim.trim(line)
end

local function ltrim(s)
  return (s:gsub("^%s+", ""))
end

local function is_blank(s)
  return s == nil or s:match("^%s*$") ~= nil
end

local function contains(s, needle)
  return s:find(needle, 1, true) ~= nil
end

local function starts_with(s, prefix)
  return s:sub(1, #prefix) == prefix
end

local function count_plain(s, needle)
  local n, i = 0, 1
  while true do
    local a, b = s:find(needle, i, true)
    if not a then
      break
    end
    n = n + 1
    i = b + 1
  end
  return n
end

--- A single-line comment. `#[` / `#![` are Rust attributes, not comments.
local function is_line_comment(s)
  if starts_with(s, "///") or starts_with(s, "//!") or starts_with(s, "//") then
    return true
  end
  if starts_with(s, "--") then
    return true
  end
  if starts_with(s, "#") then
    if starts_with(s, "#[") or starts_with(s, "#![") then
      return false
    end
    -- `#-}` closes a Haskell `{-# ... #-}` pragma.
    if starts_with(s, "#-}") then
      return false
    end
    return true
  end
  return false
end

--- Block annotations: opener, closer. `{-#` must precede `{-`, and `#![` must
--- precede `#[`, because the closer of the former also matches the latter.
local BLOCK_SPECS = {
  { open = "/*", close = "*/" },
  { open = "{-#", close = "#-}" },
  { open = "{-", close = "-}" },
  { open = "#![", close = "]" },
  { open = "#[", close = "]" },
}

--- The line that opens a block annotation ending at `last`, or nil.
local function find_block_start(lines, last, spec)
  for i = last, 1, -1 do
    local raw = lines[i] or ""
    if is_blank(raw) then
      return nil
    end
    if starts_with(ltrim(raw), spec.open) then
      -- Rust attributes: the brackets must balance across the whole block.
      if spec.close == "]" then
        local depth = 0
        for j = i, last do
          depth = depth + count_plain(lines[j] or "", "[") - count_plain(lines[j] or "", "]")
        end
        if depth ~= 0 then
          return nil
        end
      end
      return i
    end
  end
  return nil
end

--- The line that opens a `@decorator` ending at `last`, including
--- parenthesized/continued forms, or nil.
local function find_decorator_start(lines, last)
  local balance = 0
  for i = last, 1, -1 do
    local raw = lines[i] or ""
    if is_blank(raw) then
      return nil
    end
    balance = balance + count_plain(raw, "(") - count_plain(raw, ")")
    if balance == 0 then
      if starts_with(ltrim(raw), "@") then
        return i
      end
      return nil
    end
    if balance > 0 then
      return nil
    end
  end
  return nil
end

--- The start line of a complete leading annotation block ending at `last`, or
--- nil when `last` is not the final line of one.
local function annotation_start(lines, last)
  local raw = lines[last]
  if is_blank(raw) then
    return nil
  end
  local s = ltrim(raw)

  -- A shebang on line 1 is a file directive, not a declaration annotation.
  if last == 1 and starts_with(s, "#!") and not starts_with(s, "#![") then
    return nil
  end

  if is_line_comment(s) then
    return last
  end

  local decorator = find_decorator_start(lines, last)
  if decorator then
    return decorator
  end

  for _, spec in ipairs(BLOCK_SPECS) do
    if contains(raw, spec.close) then
      local start = find_block_start(lines, last, spec)
      if start then
        return start
      end
    end
  end

  return nil
end

--- Extend a fold start upward over complete leading annotations.
---
--- `start_line` is 1-based and comes from the outline span, which excludes
--- attributes/decorators/doc comments. `lines` is the buffer's full line list
--- and `is_boundary` answers whether a line belongs to a different (non-
--- ancestor) declaration, which ends a leading run. Returns the 1-based start.
function M.fold_start(lines, start_line, is_boundary)
  local line = start_line
  while line > 1 do
    local block_start = annotation_start(lines, line - 1)
    if not block_start then
      break
    end
    if is_boundary and block_start > 1 and is_boundary(block_start - 1) then
      break
    end
    line = block_start
  end
  return line
end

--- Build the fold tree from an `ownai.show.v1` file entry.
---
--- Returns `{ list = items, by_key = map }`. Each item gains:
---   * `key`, `parent`, `kind`, `name`, `signature`
---   * `fold_start` (1-based, extended over annotations)
---   * `end_line` (1-based)
---   * `level` (1-based nesting depth, capped at 20)
---   * `children` (direct children in outline order)
---   * `depth`
function M.build(file_entry, buf)
  local list = {}
  local by_key = {}

  for _, raw in ipairs(file_entry.outline or {}) do
    local item = {
      key = raw.stable_key,
      parent = raw.parent_key,
      kind = raw.kind,
      name = raw.name,
      signature = raw.signature or "",
      span = raw.span,
      end_line = raw.span and raw.span.end_line or 0,
      children = {},
    }
    list[#list + 1] = item
    by_key[item.key] = item
  end

  -- Nesting depth via parent links. A missing parent is treated as top level.
  local function depth_of(item, seen)
    if item.depth then
      return item.depth
    end
    seen = seen or {}
    if seen[item.key] then
      item.depth = 0
      return 0
    end
    seen[item.key] = true
    local parent = item.parent and by_key[item.parent]
    local depth = parent and (depth_of(parent, seen) + 1) or 0
    item.depth = depth
    return depth
  end

  for _, item in ipairs(list) do
    depth_of(item)
    local parent = item.parent and by_key[item.parent]
    if parent then
      parent.children[#parent.children + 1] = item
    end
  end

  local lines = (buf and vim.api.nvim_buf_is_valid(buf)) and vim.api.nvim_buf_get_lines(buf, 0, -1, false) or {}

  local function is_ancestor(item, key)
    local p = item.parent
    while p do
      if p == key then
        return true
      end
      local parent = by_key[p]
      p = parent and parent.parent
    end
    return false
  end

  local function span_contains(span, line)
    return span ~= nil and span.start_line <= line and line <= span.end_line
  end

  -- A line is a declaration boundary when it belongs to a different, non-
  -- ancestor declaration. A leading run must not cross one: a comment that
  -- directly follows another declaration belongs to that declaration.
  local function is_boundary(item, line)
    for _, other in ipairs(list) do
      if other ~= item and span_contains(other.span, line) and not is_ancestor(item, other.key) then
        return true
      end
    end
    return false
  end

  for _, item in ipairs(list) do
    local start = item.span and item.span.start_line or 1
    item.fold_start = M.fold_start(lines, start, function(line)
      return is_boundary(item, line)
    end)
    item.level = math.min(item.depth + 1, 20)
  end

  return { list = list, by_key = by_key }
end

return M
