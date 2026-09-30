-- codect.outline: turn the mode-independent declaration outline into a fold
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

--- The index just past a quoted string starting at `i`, or nil.
---
--- Handles `\` escapes and an unterminated string (consumes the rest of the
--- line). `'` is treated as a string delimiter, which is right for Python
--- decorators and Rust char literals; a bare lifetime in an attribute is rare
--- enough to accept the approximation.
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

--- The index just past a raw/byte string starting at `i`, or nil.
---
--- Recognizes `r"…"`, `r#"…"#`, `br"…"`, `br#"…"#`, and `rb…` forms. The
--- closing quote must be followed by the same number of `#` characters.
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

--- The index just past a Rust lifetime starting at `i` (`'a`, `'static`), or
--- nil when the `'` opens a char literal. A lifetime has no closing quote.
local function lifetime_end(s, i)
  local ident = s:sub(i + 1):match("^[%a_][%w_]*")
  if ident and s:sub(i + 1 + #ident, i + 1 + #ident) ~= "'" then
    return i + 1 + #ident
  end
  return nil
end

--- Replace the contents of string literals with spaces.
---
--- Delimiter counting (`[`/`]`, `(`/`)`) must ignore brackets inside strings:
--- `#[doc = "a ] b"]` has one `[` and one `]`, not one and two. Raw/byte
--- string prefixes are blanked too; they are never delimiters. A Rust lifetime
--- (`'a`) is copied verbatim rather than treated as a char literal.
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
---
--- Scans upward and tracks nesting depth (string-aware), so a balanced inner
--- block (`/* inner */`) is skipped and the outermost opener is returned. Blank
--- lines inside the already-open block are tolerated; a blank line between
--- separate annotation runs still ends the search.
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
    -- depth > 0: inside the block. A blank line here is interior, not a
    -- separator between annotation runs.
  end
  return nil
end

--- The line that opens a `@decorator` ending at `last`, including
--- parenthesized/continued forms, or nil.
---
--- Blank lines inside the parentheses are tolerated (a continued decorator may
--- group its arguments); the search still stops at a blank line between
--- separate runs.
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

--- Classify an annotation block by its opening line.
---
--- `trailing` comments belong to the declaration above when they directly
--- follow it (`//`, `#`, `--`, `/*`, `{-`). `forward` forms attach to the
--- declaration below and may cross into the preceding declaration's span
--- (`///`, `//!`, `/**`, `/*!`, `{-|`, `{-#`, `#[`, `#![`, `@…`).
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

--- The start line of a complete leading annotation block ending at `last`,
--- plus its form ("forward" or "trailing"), or nil when `last` is not the
--- final line of one.
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

--- Extend a fold start upward over complete leading annotations.
---
--- `start_line` is 1-based and comes from the outline span, which excludes
--- attributes/decorators/doc comments. `lines` is the buffer's full line list
--- and `is_boundary` answers whether a line belongs to a different (non-
--- ancestor) declaration, which ends a leading run. Returns the 1-based start.
---
--- A trailing comment form that directly follows another declaration belongs to
--- that declaration and stops the run. A forward-attaching form (`///`, `#[`,
--- `@…`, `/**`, `{-|`, …) belongs to the declaration below and may cross into
--- the preceding declaration's span.
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

--- Build the fold tree from an `codect.show.v1` file entry.
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
