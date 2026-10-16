-- codect.outline: turn the mode-independent declaration outline into a fold
-- tree and fold regions.
--
-- The document's `outline` is complete regardless of mode, so this module only
-- needs structure: parent links, nesting depth, a fold start, and an end line.
--
-- Block openers are matched longest-first (`{-#` before `{-`, `#![` before
-- `#[`) because the closer of the former also matches the latter.

local M = {}

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

local function quoted_string_end(s, i)
  local quote = s:sub(i, i)
  local j = i + 1
  while j <= #s do
    local c = s:sub(j, j)
    if c == "\\" then
      j = j + 2
    elseif c == quote then
      return j + 1
    else
      j = j + 1
    end
  end
  return #s + 1
end

local function raw_string_end(s, i)
  local rest = s:sub(i)
  local prefix, hashes, quote = rest:match("^(br)(#*)([\"'])")
  if not prefix then
    prefix, hashes, quote = rest:match("^(rb)(#*)([\"'])")
  end
  if not prefix then
    prefix, hashes, quote = rest:match("^(r)(#*)([\"'])")
  end
  if not prefix then
    return nil
  end
  local closer = quote .. hashes
  local from = i + #prefix + #hashes + 1
  local found = s:find(closer, from, true)
  if found then
    return found + #closer
  end
  return #s + 1
end

local function lifetime_end(s, i)
  local ident = s:sub(i + 1):match("^[%a_][%w_]*")
  if ident and s:sub(i + 1 + #ident, i + 1 + #ident) ~= "'" then
    return i + 1 + #ident
  end
  return nil
end

local function strip_strings(s)
  local out = {}
  local i = 1
  while i <= #s do
    local stop = raw_string_end(s, i)
    local handled = false
    if not stop then
      local c = s:sub(i, i)
      if c == "'" then
        local life = lifetime_end(s, i)
        if life then
          out[#out + 1] = s:sub(i, life - 1)
          i = life
          handled = true
        else
          stop = quoted_string_end(s, i)
        end
      elseif c == '"' then
        stop = quoted_string_end(s, i)
      end
    end
    if not handled then
      if stop then
        out[#out + 1] = string.rep(" ", stop - i)
        i = stop
      else
        out[#out + 1] = s:sub(i, i)
        i = i + 1
      end
    end
  end
  return table.concat(out)
end

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

local BLOCK_SPECS = {
  { open = "/*", close = "*/" },
  { open = "{-#", close = "#-}" },
  { open = "{-", close = "-}" },
  { open = "#![", close = "]" },
  { open = "#[", close = "]" },
}

local function find_block_start(lines, last, spec)
  local depth = 0
  for i = last, 1, -1 do
    local raw = lines[i] or ""
    local scanned = strip_strings(raw)
    local opens = count_plain(scanned, spec.open)
    local closes = count_plain(scanned, spec.close)
    if i == last then
      depth = closes - opens
    else
      depth = depth + closes - opens
    end

    if depth <= 0 then
      if depth == 0 and starts_with(ltrim(raw), spec.open) then
        return i
      end
      return nil
    end
  end
  return nil
end

local function find_decorator_start(lines, last)
  local depth = 0
  for i = last, 1, -1 do
    local raw = lines[i] or ""
    local scanned = strip_strings(raw)
    depth = depth + count_plain(scanned, ")") - count_plain(scanned, "(")
    if depth <= 0 then
      if depth == 0 and starts_with(ltrim(raw), "@") then
        return i
      end
      return nil
    end
  end
  return nil
end

local function annotation_form(s)
  if starts_with(s, "///") or starts_with(s, "//!") then
    return "forward"
  end
  if starts_with(s, "//") then
    return "trailing"
  end
  if starts_with(s, "/**") or starts_with(s, "/*!") then
    return "forward"
  end
  if starts_with(s, "/*") then
    return "trailing"
  end
  if starts_with(s, "{-|") or starts_with(s, "{-#") then
    return "forward"
  end
  if starts_with(s, "{-") then
    return "trailing"
  end
  if starts_with(s, "#[") or starts_with(s, "#![") then
    return "forward"
  end
  if starts_with(s, "@") then
    return "forward"
  end
  if starts_with(s, "--") or starts_with(s, "#") then
    return "trailing"
  end
  return nil
end

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
    return last, annotation_form(s)
  end

  local decorator = find_decorator_start(lines, last)
  if decorator then
    return decorator, annotation_form(ltrim(lines[decorator] or ""))
  end

  for _, spec in ipairs(BLOCK_SPECS) do
    if contains(strip_strings(raw), spec.close) then
      local start = find_block_start(lines, last, spec)
      if start then
        return start, annotation_form(ltrim(lines[start] or ""))
      end
    end
  end

  return nil
end

function M.fold_start(lines, start_line, is_boundary)
  local line = start_line
  while line > 1 do
    local block_start, form = annotation_start(lines, line - 1)
    if not block_start then
      break
    end
    if is_boundary and form == "trailing" and block_start > 1 and is_boundary(block_start - 1) then
      break
    end
    line = block_start
  end
  return line
end

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
