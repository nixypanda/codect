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

--- Declaration kinds Types mode retains. Everything else is dropped and gets a
--- "hidden" marker. Used only to approximate a mode whose document is not
--- cached; a cached document's `retained_in_mode` is authoritative.
local TYPES_RETAINED = {
  type = true,
  type_alias = true,
  trait = true,
  field = true,
  variant = true,
  constructor = true,
  associated_type = true,
  type_family = true,
  pattern_synonym = true,
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

--- A Types-mode inherent `impl` has no `for`; a trait impl does.
function M.is_trait_impl(signature)
  local first = M.first_line(signature)
  return first:match("^impl%s") ~= nil and first:match(" for ") ~= nil
end

--- Approximate `retained_in_mode` from the outline alone.
function M.approx_retained(item, mode)
  if mode == "signatures" then
    return true
  end
  if mode == "types" then
    if item.kind == "trait_implementation" then
      return M.is_trait_impl(item.signature)
    end
    return TYPES_RETAINED[item.kind] == true
  end
  return true
end

--- Does this source line look like a leading attribute, decorator, pragma, or
--- doc comment? Conservative: only a short, explicit set of prefixes.
function M.is_leading_annotation(line)
  local s = line:gsub("^%s+", "")
  if s == "" then
    return false
  end
  -- Rust attributes: #[...] and #![...]
  if s:match("^#%[") or s:match("^#!%[") then
    return true
  end
  -- Decorators: @Component, @override, ...
  if s:match("^@") then
    return true
  end
  -- Haskell pragmas: {-# ... #-}
  if s:match("^{%-%#") then
    return true
  end
  -- Haddock / Elm doc comments: {-| ...
  if s:match("^{%-|") then
    return true
  end
  -- Line doc comments: ///, //!, //, --
  if s:match("^///") or s:match("^//!") or s:match("^//") then
    return true
  end
  -- Block doc comments: /** ... */ and /*! ... */
  if s:match("^/%*%*") or s:match("^/%*!") then
    return true
  end
  -- Haskell / Elm line comments, including -- | and -- ^
  if s:match("^%-%-") then
    return true
  end
  -- Python and other `#` line comments.
  if s:match("^#") then
    return true
  end
  return false
end

--- Extend a fold start upward over consecutive leading annotations.
---
--- `start_line` is 1-based and comes from the outline span, which excludes
--- attributes/decorators/doc comments. Returns the 1-based extended start.
function M.fold_start(buf, start_line)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local line = start_line
  while line > 1 do
    local previous = lines[line - 1]
    if previous and M.is_leading_annotation(previous) then
      line = line - 1
    else
      break
    end
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

  for _, item in ipairs(list) do
    item.fold_start = M.fold_start(buf, item.span and item.span.start_line or 1)
    item.level = math.min(item.depth + 1, 20)
  end

  return { list = list, by_key = by_key }
end

return M
