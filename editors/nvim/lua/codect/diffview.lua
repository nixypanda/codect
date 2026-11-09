-- Focused Diffview adapter for the pinned Diffview 4516612 API.
-- Diffview has no public content or file-list provider. Keep all access to its
-- internal classes here and refuse to install against an unknown revision.
local M = {}

local pathformat = require("codect.path")
local cache_module = require("codect.cache")

local modes = setmetatable({}, { __mode = "k" })
local cache = cache_module.new(32, 16 * 1024 * 1024)
local views = setmetatable({}, { __mode = "v" })
local installed = false
local warned = false
local projection_buffer = 0

local EXPECTED = {
  ["diffview.vcs.adapters.git"] = "5d77fd042d1af44ee97570032fb4823a00809cc60d7e2e1a68a32b88ad20d26c",
  ["diffview.scene.views.diff.diff_view"] = "f2c757359a4dfb0565a77f3e9b4653e8a6486d929bdf2713c459eb25eff415a0",
  ["diffview.vcs.file"] = "a5171a6e5b64913957f2adb9bd4f251911821c4b151b2423f95e4b4e52f37954",
  -- Constructed move entries call the private FileEntry constructor directly.
  ["diffview.scene.file_entry"] = "6d4cbc82fa3e182771eae70382f2a06ae64657d0cb3207ef105951bc9e86963d",
  -- Move construction builds a null-tree rev and reads section revs directly.
  ["diffview.vcs.adapters.git.rev"] = "184b02e53d00dc05807119b427c11b33fe9055377c0a41c48c4cffa3088068d7",
  ["diffview.vcs.rev"] = "a827c12cabaccc05cfb4f999d94fe0beda2604a85d6ec530c0e1e207d5435af1",
  ["diffview.scene.view"] = "65efc14c99a3d8389490fbba8ee85f794fe20c5d9cb351720aacec28b81def1e",
  ["diffview.scene.layouts.diff_1"] = "a1be9d1b042a2482528203f1f7119ab5df73a2296799175c2b10fc718e96d94d",
  ["diffview.scene.layouts.diff_2"] = "3965ab81d7006f59cf4ab09ed89ebed20f011a2ba4ecbc9280fc18c84dad75b5",
  ["diffview.scene.layouts.diff_2_hor"] = "c16a7cd82ac49d64ecc83cea2b4e588b9116777ff398a0eb18ed9567e3050803",
  ["diffview.scene.layouts.diff_2_ver"] = "548a831cb2de47d53bff322c527ac6a7da88d3e0b02e8ed229a144fed69a6ab1",
  ["diffview.ui.models.file_tree.file_tree"] = "33cf7d88b879fced18a9e20453854cc00dbda00712a331fc7800ea1736d5243c",
  ["diffview.ui.models.file_tree.node"] = "917c612a3299f7fdf95b7e6d0346a40cc766a95d33d72843dbac638b68726f7e",
}

local function compatible()
  for module, hash in pairs(EXPECTED) do
    local path = vim.api.nvim_get_runtime_file("lua/" .. module:gsub("%.", "/") .. ".lua", false)[1]
      or vim.api.nvim_get_runtime_file("lua/" .. module:gsub("%.", "/") .. "/init.lua", false)[1]
    if not path then return false, "missing " .. module end
    local fd = io.open(path, "rb")
    if not fd then return false, "cannot read " .. path end
    local source = fd:read("*a")
    fd:close()
    if vim.fn.sha256(source) ~= hash then
      return false, "unsupported Diffview source: " .. module
    end
  end
  return true
end

local function notify(message)
  vim.notify("Codect Diffview: " .. message, vim.log.levels.WARN)
end

local function current_view()
  return require("diffview.lib").get_current_view()
end

local function split_lines(value)
  value = (value or ""):gsub("\n$", "")
  if value == "" then return {} end
  return vim.split(value, "\n", { plain = true })
end

-- v2 JSON paths use the shared injective display encoding. Diffview paths and
-- Git arguments are raw byte strings, so every comparison and constructed
-- FileEntry belongs to the decoded side of this mapping.
local function display_path_parts(display)
  local parent, base = display:match("^(.*)/([^/]*)$")
  if not parent then return "", display end
  return parent, base
end

local function path_components(value)
  local parts = {}
  for part in value:gmatch("[^/]+") do
    parts[#parts + 1] = part
  end
  return parts
end

local function restore_display_path(entry)
  local saved = entry._codect_native_display
  if saved then
    entry.basename = saved.basename
    entry.parent_path = saved.parent_path
    for _, symbol in ipairs({ "a", "b" }) do
      local side = entry.layout[symbol]
      if side and side.file then side.file.winbar = saved.winbars[symbol] end
    end
  end
  entry._codect_native_display = nil
  entry._codect_display_path = nil
end

local function set_display_path(entry, file)
  local display = file.display_path
  if display == file.path then
    if entry._codect_display_path then restore_display_path(entry) end
    return
  end
  if not entry._codect_native_display then
    local saved = {
      basename = entry.basename,
      parent_path = entry.parent_path,
      winbars = {},
    }
    for _, symbol in ipairs({ "a", "b" }) do
      local side = entry.layout[symbol]
      saved.winbars[symbol] = side and side.file and side.file.winbar or nil
    end
    entry._codect_native_display = saved
  end
  local parent, base = display_path_parts(display)
  entry.parent_path = parent
  entry.basename = base
  entry._codect_display_path = display
  for _, symbol in ipairs({ "a", "b" }) do
    local side = entry.layout[symbol]
    local peer = side and side.file
    if peer and type(peer.winbar) == "string" and type(peer.path) == "string" then
      local display_side = symbol == "a" and (file.display_old_path or display) or display
      local start, finish = peer.winbar:find(peer.path, 1, true)
      if start then
        peer.winbar = peer.winbar:sub(1, start - 1)
          .. display_side .. peer.winbar:sub(finish + 1)
      end
    end
  end
end

local function validate_doc(doc)
  if type(doc) ~= "table" or doc.schema ~= "codect.diff.v2" or type(doc.files) ~= "table" then
    return nil, "invalid or unsupported codect.diff.v2 document"
  end
  for _, file in ipairs(doc.files) do
    if type(file) ~= "table" or type(file.path) ~= "string" then
      return nil, "invalid file entry in codect.diff.v2 document"
    end
    if file.status ~= "added" and file.status ~= "deleted"
        and file.status ~= "modified" and file.status ~= "moved" then
      return nil, "invalid status in codect.diff.v2 document: " .. file.path
    end
    local raw_path = pathformat.decode_display(file.path)
    if not raw_path or raw_path == "" then
      return nil, "invalid escaped path in codect.diff.v2 document: " .. file.path
    end
    if file.status == "moved" then
      if type(file.old_path) ~= "string" or file.old_path == "" or file.old_path == file.path then
        return nil, "invalid moved file in codect.diff.v2 document: " .. file.path
      end
      local old_path = pathformat.decode_display(file.old_path)
      if not old_path or old_path == "" or old_path == raw_path then
        return nil, "invalid moved file in codect.diff.v2 document: " .. file.path
      end
      if type(file.base) ~= "table" or type(file.target) ~= "table" then
        return nil, "moved file lacks both sides in codect.diff.v2 document: " .. file.path
      end
    end
  end
  return doc
end

local function diff_doc(root, base, target, mode)
  local key = table.concat({ root, base, target, mode }, "\0")
  local immutable = base ~= ":index" and base ~= ":worktree"
    and target ~= ":index" and target ~= ":worktree"
  if immutable then
    local cached = cache.get(key)
    if cached then return cached end
  end
  local binary = require("codect.cli").resolve_binary()
  if not binary then return nil, "codect binary not found" end
  local result = vim.system({ binary, "diff", "--format", "json", "--mode", mode, base, target }, {
    cwd = root, text = true, timeout = 30000,
  }):wait()
  if result.code ~= 0 then return nil, vim.trim(result.stderr or "codect diff failed") end
  local ok, doc = pcall(vim.json.decode, result.stdout)
  if not ok then return nil, "invalid or unsupported codect.diff.v2 document" end
  local valid, err = validate_doc(doc)
  if not valid then return nil, err end
  if immutable then cache.put(key, doc, #result.stdout) end
  return doc
end

local function pair_key(old_path, path)
  return old_path .. "\0" .. path
end

-- Diffview's Git adapter does not pass `-z`, so Git C-quotes paths that contain
-- a backslash, quote, or control byte even under `core.quotePath=false`.
-- Undo that quoting before comparing against decoded Codect paths.
local GIT_QUOTE_ESCAPE = {
  a = "\a", b = "\b", f = "\f", n = "\n", r = "\r", t = "\t", v = "\v",
  ["\\"] = "\\", ['"'] = '"',
}

local function raw_git_path(value)
  if type(value) ~= "string" or value:sub(1, 1) ~= '"' or value:sub(-1) ~= '"' then
    return value
  end
  local body, out, i = value:sub(2, -2), {}, 1
  while i <= #body do
    local char = body:sub(i, i)
    if char ~= "\\" then
      out[#out + 1] = char
      i = i + 1
    else
      local next_char = body:sub(i + 1, i + 1)
      if GIT_QUOTE_ESCAPE[next_char] then
        out[#out + 1] = GIT_QUOTE_ESCAPE[next_char]
        i = i + 2
      elseif next_char:match("^[0-7]$") then
        local octal = body:sub(i + 1, i + 3):match("^[0-7][0-7]?[0-7]?")
        out[#out + 1] = string.char(tonumber(octal, 8) % 256)
        i = i + 1 + #octal
      else
        out[#out + 1] = "\\"
        i = i + 1
      end
    end
  end
  return table.concat(out)
end

local function index_doc(doc)
  local indexed = { files = {}, by_pair = {}, by_path = {} }
  for _, file in ipairs(doc.files) do
    local raw = {
      status = file.status,
      path = pathformat.decode_display(file.path),
      display_path = file.path,
      base = file.base,
      target = file.target,
    }
    if file.status == "moved" then
      raw.old_path = pathformat.decode_display(file.old_path)
      raw.display_old_path = file.old_path
    end
    indexed.files[#indexed.files + 1] = raw
    if raw.status == "moved" then
      indexed.by_pair[pair_key(raw.old_path, raw.path)] = raw
    else
      indexed.by_path[raw.path] = raw
    end
  end
  return indexed
end

local function match_projection(index, entry)
  if entry.oldpath ~= nil then
    return index.by_pair[pair_key(entry.oldpath, entry.path)]
  end
  return index.by_path[entry.path]
end

local function release(file)
  if file._codect_foldlevel ~= nil then
    file.winopts.foldlevel = file._codect_foldlevel
    file._codect_foldlevel = nil
  end
  if file._codect_projection then
    file:dispose_buffer()
    file._codect_projection = nil
  elseif file.rev and (file.rev.type == require("diffview.vcs.rev").RevType.LOCAL
      or file.rev.type == require("diffview.vcs.rev").RevType.STAGE) then
    -- The real worktree or index buffer may contain user edits. Detach it
    -- without deleting or changing it when entering focused mode.
    file:detach_buffer()
    file.bufnr = nil
  else
    file:dispose_buffer()
  end
  file.get_data = nil
end

local function attach(entry, projected)
  local left = split_lines(type(projected.base) == "table" and projected.base.projection.text)
  local right = split_lines(type(projected.target) == "table" and projected.target.projection.text)
  -- A File's `symbol` is assigned only when Diffview selects its layout.
  -- History entries are projected as they stream in, before that selection,
  -- so derive the side from the layout slot instead.
  for _, symbol in ipairs({ "a", "b" }) do
    local window = entry.layout[symbol]
    local file = window and window.file
    if file then
      if file._codect_projection then file:dispose_buffer() end
      if file.rev then
        if not file._codect_projection then release(file) end
        -- Diffview normally starts diff panes at foldlevel=0. A focused
        -- projection is already short; hiding its unchanged lines can make the
        -- two sides look like a single collapsed declaration instead of a diff.
        -- Keep foldmethod=diff for highlights and open every projected line.
        if file._codect_foldlevel == nil then
          file._codect_foldlevel = file.winopts.foldlevel
        end
        file.winopts.foldlevel = 99
        file._codect_projection = symbol == "a" and left or right
        file.get_data = function(_, _, pos) return pos == "left" and left or right end
      end
    end
  end
end

local function refresh_existing(view, by_section, restore_display)
  if not view.files then return end
  for _, entry in view.files:iter() do
    local index = by_section and by_section[entry.kind]
    local projected = index and match_projection(index, entry)
    if projected then
      attach(entry, projected)
      set_display_path(entry, projected)
    elseif restore_display then
      restore_display_path(entry)
    end
    if not projected then
      for _, file in ipairs(entry.layout:files()) do
        if file._codect_projection then release(file) end
      end
    end
  end
end

local function empty_history_entry(adapter, entry)
  local null = require("diffview.scene.file_entry").FileEntry.new_null_entry(adapter)
  for _, file in ipairs(entry.files) do file:destroy() end
  entry.files = { null }
  entry.nulled = true
  entry:update_status()
  entry:update_stats()
end

function M.install()
  if installed then return true end
  local ok, reason = compatible()
  if not ok then
    if not warned then notify(reason .. "; focused mode disabled") end
    warned = true
    return false
  end

  local GitAdapter = require("diffview.vcs.adapters.git").GitAdapter
  local DiffView = require("diffview.scene.views.diff.diff_view").DiffView
  local RevType = require("diffview.vcs.rev").RevType
  local File = require("diffview.vcs.file").File
  local FileEntry = require("diffview.scene.file_entry").FileEntry
  local FileTree = require("diffview.ui.models.file_tree.file_tree").FileTree
  local FileTreeNode = require("diffview.ui.models.file_tree.node").Node
  local GitRev = require("diffview.vcs.adapters.git.rev").GitRev
  local parse_history = GitAdapter.parse_fh_data
  local get_updated_files = DiffView.get_updated_files
  local create_buffer = File.create_buffer
  local destroy = File.destroy

  local ENTRY_STATUS = { added = "A", deleted = "D", modified = "M", moved = "R" }

  local function make_entry(adapter, file, context, stats)
    local moved = file.status == "moved"
    local oldpath = moved and file.old_path or nil
    local function side(path, rev, nulled)
      return File({
        adapter = adapter,
        path = path,
        kind = context.kind,
        commit = context.commit,
        rev = rev,
        nulled = nulled,
      })
    end
    local a = side(oldpath or file.path, context.revs.a, file.status == "added")
    local b = side(file.path, context.revs.b, file.status == "deleted")
    return FileEntry({
      adapter = adapter,
      path = file.path,
      oldpath = oldpath,
      status = ENTRY_STATUS[file.status],
      stats = stats,
      kind = context.kind,
      commit = context.commit,
      revs = context.revs,
      layout = context.layout_class({ a = a, b = b }),
    })
  end

  local function entry_intersects(entry, file)
    local path = raw_git_path(entry.path)
    local oldpath = entry.oldpath ~= nil and raw_git_path(entry.oldpath) or nil
    if file.status == "moved" then
      return path == file.path or path == file.old_path
        or oldpath == file.path or oldpath == file.old_path
    end
    return path == file.path or oldpath == file.path
  end

  local function reusable_entry(entries, file)
    for _, entry in ipairs(entries) do
      local path = raw_git_path(entry.path)
      local oldpath = entry.oldpath ~= nil and raw_git_path(entry.oldpath) or nil
      if file.status == "moved" then
        if path == file.path and oldpath == file.old_path
            and type(entry.status) == "string" and entry.status:sub(1, 1) == "R" then
          return entry
        end
      elseif path == file.path and oldpath == nil then
        return entry
      end
    end
  end

  local function summed_stats(entries, file)
    local additions, deletions, found = 0, 0, false
    for _, entry in ipairs(entries) do
      local related = file.status == "moved" and entry_intersects(entry, file)
        or entry.path == file.path
      if related and type(entry.stats) == "table" then
        if type(entry.stats.additions) == "number" then
          additions = additions + entry.stats.additions
          found = true
        end
        if type(entry.stats.deletions) == "number" then
          deletions = deletions + entry.stats.deletions
          found = true
        end
      end
    end
    if found then return { additions = additions, deletions = deletions } end
  end

  -- Rebuild a Diffview file list from Codect events. Reuse one exact Git
  -- rename entry; otherwise construct a two-sided entry from the section or
  -- commit revs. Every unmatched Git entry is surplus and is disposed, so a
  -- move cannot survive twice or keep a null side from a delete/add pair.
  local function reconcile(entries, index, context)
    local used = {}
    local kept = {}
    for _, file in ipairs(index.files) do
      local entry = reusable_entry(entries, file)
      if entry and (entry.path ~= file.path
          or (file.status == "moved" and entry.oldpath ~= file.old_path)) then
        -- The Git path was C-quoted. Constructing from the decoded raw path
        -- keeps Git arguments and Diffview identities byte-correct.
        entry = nil
      end
      if entry then
        used[entry] = true
        if file.status == "moved" then
          if not (entry.layout.a and entry.layout.b) then
            entry:convert_layout(context.layout_class)
          end
          for _, symbol in ipairs({ "a", "b" }) do
            local side = entry.layout[symbol]
            if side and side.file then side.file.nulled = false end
          end
        end
      else
        local relevant = false
        for _, candidate in ipairs(entries) do
          if entry_intersects(candidate, file) then
            relevant = true
            break
          end
        end
        if relevant then
          entry = make_entry(context.adapter, file, context, summed_stats(entries, file))
        end
      end
      if entry then
        attach(entry, file)
        set_display_path(entry, file)
        kept[#kept + 1] = entry
      end
    end
    for _, entry in ipairs(entries) do
      if not used[entry] then entry:destroy() end
    end
    return kept
  end

  -- FileTree's path helpers normalize backslashes as separators and expose raw
  -- entry.path components. Focused Codect entries instead build directory keys
  -- and paths from raw components split only on "/", while labels come from the
  -- encoded display path. Source-mode entries keep the native tree unchanged.
  local add_file_entry = FileTree.add_file_entry
  FileTree.add_file_entry = function(tree, file)
    local display = file._codect_display_path
    if not display or display == file.path then
      return add_file_entry(tree, file)
    end
    local raw_parts, display_parts = path_components(file.path), path_components(display)
    if #raw_parts ~= #display_parts then
      return add_file_entry(tree, file)
    end
    local node, raw_path = tree.root, ""
    tree._codect_display_dirs = tree._codect_display_dirs or {}
    for i = 1, #raw_parts - 1 do
      local key = raw_parts[i]
      raw_path = i == 1 and key or (raw_path .. "/" .. key)
      tree._codect_display_dirs[raw_path] = display_parts[i]
      local child = node.children[key]
      if not child then
        child = FileTreeNode(key, {
          name = display_parts[i],
          path = raw_path,
          kind = file.kind,
          collapsed = false,
          status = " ",
        })
        node:add_child(child)
      elseif child.data then
        child.data.name = display_parts[i]
      end
      node = child
    end
    node:add_child(FileTreeNode(raw_parts[#raw_parts], file))
  end

  local create_comp_schema = FileTree.create_comp_schema
  FileTree.create_comp_schema = function(tree, data)
    local schema = create_comp_schema(tree, data)
    local dirs = tree._codect_display_dirs
    if not dirs then return schema end
    local function display_path(raw)
      local parts, labels, prefix = path_components(raw), {}, ""
      for i, part in ipairs(parts) do
        prefix = i == 1 and part or (prefix .. "/" .. part)
        labels[i] = dirs[prefix] or part
      end
      return table.concat(labels, "/")
    end
    local function walk(components)
      for _, child in ipairs(components) do
        if type(child) == "table" then
          if child.name == "directory" and child.context and child.context.path
              and type(child.context.name) == "string"
              and child.context.name:find("/", 1, true) then
            child.context.name = display_path(child.context.path)
          end
          walk(child)
        end
      end
    end
    walk(schema)
    return schema
  end

  File.destroy = function(file, force)
    local projected = file._codect_projection ~= nil
    if not projected and not force and file.rev
        and (file.rev.type == RevType.STAGE or file.rev.type == RevType.LOCAL)
        and file.bufnr and vim.api.nvim_buf_is_valid(file.bufnr)
        and vim.bo[file.bufnr].modified then
      -- A source index/worktree buffer can disappear from the focused list.
      -- Its unsaved edits still belong to the user, even after the view closes.
      file:detach_buffer()
      file.bufnr = nil
      return
    end
    destroy(file, force or projected)
    file._codect_projection = nil
    if projected then file.get_data = nil end
  end

  -- LOCAL and STAGE normally resolve to editable user buffers. Focused panes
  -- always get independent scratch buffers, including for those revisions.
  File.create_buffer = require("diffview.async").wrap(function(file, callback)
    if not file._codect_projection or file.nulled or file.binary then
      return create_buffer(file, callback)
    end
    if file:is_valid() then return callback(file.bufnr) end
    projection_buffer = projection_buffer + 1
    local bufnr = vim.api.nvim_create_buf(false, false)
    file.bufnr = bufnr
    vim.api.nvim_buf_set_name(bufnr, "diffview://codect/" .. projection_buffer .. "/" .. file.path)
    for option, value in pairs(File.bufopts) do vim.bo[bufnr][option] = value end
    vim.bo[bufnr].modifiable = true
    vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, file._codect_projection)
    vim.bo[bufnr].modifiable = false
    vim.api.nvim_buf_call(bufnr, function() vim.cmd("filetype detect") end)
    file:post_buf_created()
    callback(bufnr)
  end)

  GitAdapter.parse_fh_data = function(adapter, data, commit, state)
    local success, entry = parse_history(adapter, data, commit, state)
    if not success then return success, entry end
    local view = views[adapter]
    local mode = view and modes[view]
    if not mode or mode == "source" or view.adapter ~= adapter then return success, entry end
    -- The Git stream may call this from a libuv fast event. Resume on the
    -- regular scheduler before invoking the CLI or touching editor state.
    local async = require("diffview.async")
    async.await(async.scheduler())
    local base = data.left_hash or GitRev.NULL_TREE_SHA
    local target = state.prepared_log_opts.base and state.prepared_log_opts.base.commit or data.right_hash
    if not target then return success, entry end
    local doc, err = diff_doc(adapter.ctx.toplevel, base, target, mode)
    if not doc then
      notify(err)
      return success, entry
    end
    local revs = {
      a = data.left_hash and GitRev(RevType.COMMIT, data.left_hash) or GitRev.new_null_tree(),
      b = state.prepared_log_opts.base or GitRev(RevType.COMMIT, data.right_hash),
    }
    local kept = reconcile(entry.files, index_doc(doc), {
      adapter = adapter,
      kind = "working",
      revs = revs,
      layout_class = state.layout_opt.default_layout or DiffView.get_default_layout(),
      commit = commit,
    })
    if #kept == 0 then
      -- Keep the commit in the history, with Diffview's own "No diff" row.
      entry.files = {}
      empty_history_entry(adapter, entry)
    else
      entry.files = kept
      entry:update_status()
      entry:update_stats()
    end
    return success, entry
  end

  DiffView.get_updated_files = require("diffview.async").wrap(function(view, callback)
    get_updated_files(view, function(err, files)
      local async = require("diffview.async")
      async.await(async.scheduler())
      local mode = modes[view]
      if err or not files or not mode or mode == "source" then return callback(err, files) end
      local function snapshot(rev)
        if rev.type == RevType.COMMIT then return rev.commit end
        if rev.type == RevType.STAGE and rev.stage == 0 then return ":index" end
        if rev.type == RevType.LOCAL then return ":worktree" end
      end
      local left, right = snapshot(view.left), snapshot(view.right)
      if not left or not right then return callback(err, files) end
      local root = view.adapter.ctx.toplevel
      local comparisons = {}
      local function compare(base, target)
        local key = base .. "\0" .. target
        if comparisons[key] then return comparisons[key] end
        local doc, doc_err = diff_doc(root, base, target, mode)
        if not doc then return nil, doc_err end
        comparisons[key] = index_doc(doc)
        return comparisons[key]
      end
      -- Plain DiffviewOpen has separate HEAD/index and index/worktree rows.
      -- An explicit range is one comparison, even when its target is local.
      local head
      local section_indexes, section_contexts = {}, {}
      for _, section in ipairs({ "working", "staged" }) do
        local base, target = left, right
        local revs = { a = view.left, b = view.right }
        if left == ":index" and right == ":worktree" and section == "staged" then
          head = head or view.adapter:head_rev()
          base = head and head.commit or GitRev.NULL_TREE_SHA
          target = ":index"
          revs = { a = head or GitRev.new_null_tree(), b = GitRev(RevType.STAGE, 0) }
        end
        local index, doc_err = compare(base, target)
        if not index then notify(doc_err); return callback(err, files) end
        section_indexes[section] = index
        section_contexts[section] = {
          adapter = view.adapter,
          kind = section,
          revs = revs,
          layout_class = DiffView.get_default_layout(),
        }
      end
      refresh_existing(view, section_indexes)
      for _, section in ipairs({ "working", "staged" }) do
        files[section] = reconcile(files[section], section_indexes[section], section_contexts[section])
      end
      -- Conflicts use multiple index stages and have no v2 snapshot mapping.
      for _, entry in ipairs(files.conflicting) do entry:destroy() end
      files.conflicting = {}
      callback(nil, files)
    end)
  end)

  installed = true
  return true
end

function M.on_view_opened(view)
  if not M.install() then return end
  modes[view] = require("codect").config.default_mode
  views[view.adapter] = view
end

function M.set_mode(mode)
  local view = current_view()
  if not view then return notify("open a Diffview tab first") end
  if mode ~= "types" and mode ~= "signatures" and mode ~= "tests" and mode ~= "source" then
    return notify("mode must be types, signatures, tests, or source")
  end
  if not M.install() then return end
  if modes[view] == mode then return end
  local previous_mode = modes[view]
  modes[view] = mode
  if view.panel and view.panel.update_entries then
    local selected = view.panel.cur_item and view.panel.cur_item[1]
    local hash = selected and selected.commit and selected.commit.hash
    local current = view.panel.cur_item and view.panel.cur_item[2]
    local path = current and (previous_mode == "source" and raw_git_path(current.path) or current.path)
    local oldpath = current and current.oldpath ~= nil
      and (previous_mode == "source" and raw_git_path(current.oldpath) or current.oldpath)
      or nil
    view.panel:update_entries(function()
      if not hash or not path then return end
      for _, entry in ipairs(view.panel.entries or {}) do
        if entry.commit.hash == hash then
          local file = entry.files[1]
          local same_path
          for _, candidate in ipairs(entry.files) do
            if candidate.path == path then
              same_path = same_path or candidate
              if candidate.oldpath == oldpath then
                file = candidate
                break
              end
            end
          end
          file = same_path or file
          if file then view:set_file(file) end
          return
        end
      end
    end)
  elseif view.update_files then
    -- An unchanged path survives Diffview's file-list update. Drop the old
    -- scratch buffer and producer so its next load uses the selected mode.
    refresh_existing(view, nil, mode == "source")
    view:update_files()
  end
end

return M
