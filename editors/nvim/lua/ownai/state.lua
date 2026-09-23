-- ownai.state: per-buffer projection state and sticky fold overrides.
--
-- Two kinds of state live here:
--
--   * Per-buffer state (mode, cached documents, the processed outline, the
--     line -> fold-level map). Rebuilt on every projection/refresh.
--
--   * Sticky user fold overrides, keyed by `(path, stable_key)` because a
--     stable_key is only unique within a file. Overrides survive re-folding,
--     refreshes, mode switches, and buffer reloads.

local outline = require("ownai.outline")

local M = {}

--- bufnr -> buffer state.
M.buffers = {}

--- "path\0stable_key" -> boolean. true = the user opened it, false = closed.
M.overrides = {}

local function override_key(path, stable_key)
  return path .. "\0" .. stable_key
end

--- Get (or create) the state table for a buffer.
function M.get(buf)
  return M.buffers[buf]
end

function M.is_active(buf)
  local st = M.buffers[buf]
  return st ~= nil and st.active == true
end

function M.set(buf, st)
  M.buffers[buf] = st
end

function M.clear(buf)
  M.buffers[buf] = nil
end

--- Record a sticky override for a declaration.
function M.set_override(path, stable_key, open)
  M.overrides[override_key(path, stable_key)] = open == true
end

--- Read a sticky override; nil when the user has expressed no preference.
function M.get_override(path, stable_key)
  return M.overrides[override_key(path, stable_key)]
end

function M.clear_overrides(path)
  local prefix = path .. "\0"
  for key in pairs(M.overrides) do
    if key:sub(1, #prefix) == prefix then
      M.overrides[key] = nil
    end
  end
end

--- Drop every sticky override. Exposed as `require("ownai").clear_overrides()`.
function M.clear_all_overrides()
  M.overrides = {}
end

--- Is this declaration closed by default in `mode`?
---
--- Semantics:
---   * `full` folds nothing.
---   * A declaration the mode drops is closed and shows a "hidden" marker.
---   * A retained signature-kind declaration is closed and shows its
---     mode-correct canonical fragment.
---   * Retained containers/members stay open.
function M.default_closed(item, mode)
  if mode == "full" then
    return false
  end
  if not item.retained then
    return true
  end
  return outline.is_signature_kind(item.kind)
end

--- Final open/closed state for an item, honouring sticky overrides.
function M.is_closed(st, item)
  local override = M.get_override(st.path, item.key)
  if override ~= nil then
    return not override
  end
  return M.default_closed(item, st.mode)
end

--- Resolve whether `item` is retained in `mode`.
---
--- Authoritative: the mode's own document carries `retained_in_mode`. A
--- document for the target mode is always installed before this runs (see
--- `view.ensure`), so no approximation from the mode-independent outline is
--- needed or allowed. `full` retains everything.
local function resolve_retained(item, mode, doc)
  if mode == "full" then
    return true
  end
  if doc and doc.mode == mode then
    local outline_item = doc._outline_by_key and doc._outline_by_key[item.key]
    if outline_item then
      return outline_item.retained_in_mode == true
    end
  end
  return true
end

--- Closed-fold text for an item.
---
--- Contract: a retained item's text is the matching `projection.items[]`
--- `canonical_text`; a dropped item falls back to the outline `signature`.
--- Dropped items additionally carry a `hidden in <mode>` marker so the two are
--- visually distinct.
local function resolve_closed_text(item, mode, doc)
  if item.retained then
    if doc and doc._items_by_key then
      local projected = doc._items_by_key[item.key]
      if projected and projected.canonical_text then
        return outline.first_line(projected.canonical_text)
      end
    end
    return outline.first_line(item.signature)
  end
  return string.format(
    "%s %s — hidden in %s  ·  %s",
    item.kind,
    item.name,
    mode,
    outline.first_line(item.signature)
  )
end

--- Index a decoded document's file entry for fast lookup.
local function index_doc(doc, file_entry)
  doc._items_by_key = {}
  for _, item in ipairs((file_entry.projection or {}).items or {}) do
    doc._items_by_key[item.stable_key] = item
  end
  doc._outline_by_key = {}
  for _, item in ipairs(file_entry.outline or {}) do
    doc._outline_by_key[item.stable_key] = item
  end
  doc._file = file_entry
end

--- Install a freshly decoded document for `buf`.
---
--- `doc` is the whole `ownai.show.v1` document; `file_entry` is the matching
--- entry from `doc.files`. `mode` is the mode to fold with (which may differ
--- from `doc.mode` for a local `:OwnaiFold`). `hash` is the source hash the
--- document was projected from, so a later re-fold can reuse it.
function M.install(buf, doc, file_entry, mode, root, path, hash)
  index_doc(doc, file_entry)

  local st = M.buffers[buf] or {}
  st.buf = buf
  st.root = root
  st.path = path
  st.mode = mode
  st.active = true
  st.file_entry = file_entry
  st.source_hash = hash
  st.docs = st.docs or {}
  st.docs[doc.mode] = { doc = doc, hash = hash, path = path }

  M.set(buf, st)
  M.decorate(st)
  return st
end

--- The cached document for `mode`, when its source hash and target path match.
---
--- The path is part of the key because the same bytes at a different path can
--- project to a different outline (language detection, path-namespaced stable
--- keys), so a path change must force a refetch rather than reuse a stale
--- `file_entry`.
function M.cached_doc(st, mode, hash, path)
  local entry = st and st.docs and st.docs[mode]
  if entry and (hash == nil or entry.hash == hash) and (path == nil or entry.path == path) then
    return entry.doc
  end
  return nil
end

--- Switch an active buffer to another mode, reusing cached documents.
function M.set_mode(buf, mode)
  local st = M.buffers[buf]
  if not st then
    return nil
  end
  st.mode = mode
  M.decorate(st)
  return st
end

--- (Re)compute the mode-dependent fields on the processed outline.
function M.decorate(st)
  local entry = st.docs and st.docs[st.mode] or nil
  local doc = entry and entry.doc or nil
  -- The outline is mode-independent, so any installed document can supply it
  -- (used by `full`, which has no document of its own).
  local source = (doc and doc._file) or st.file_entry
  if not source then
    return
  end

  local built = outline.build(source, st.buf)
  st.items = built.list
  st.by_key = built.by_key

  for _, item in ipairs(st.items) do
    item.retained = resolve_retained(item, st.mode, doc)
    item.closed_text = resolve_closed_text(item, st.mode, doc)
  end

  -- Index fold starts to their item, preferring the innermost declaration.
  st.start_to_item = {}
  for _, item in ipairs(st.items) do
    local existing = st.start_to_item[item.fold_start]
    if not existing or item.level > existing.level then
      st.start_to_item[item.fold_start] = item
    end
  end
end

return M
