-- Focused Diffview adapter for the pinned Diffview 4516612 API.
-- Diffview has no public content or file-list provider. Keep all access to its
-- internal classes here and refuse to install against an unknown revision.
local M = {}

local modes = setmetatable({}, { __mode = "k" })
local cache = {}
local views = setmetatable({}, { __mode = "v" })
local installed = false
local warned = false
local projection_buffer = 0

local EXPECTED = {
  ["diffview.vcs.adapters.git"] = "5d77fd042d1af44ee97570032fb4823a00809cc60d7e2e1a68a32b88ad20d26c",
  ["diffview.scene.views.diff.diff_view"] = "f2c757359a4dfb0565a77f3e9b4653e8a6486d929bdf2713c459eb25eff415a0",
  ["diffview.vcs.file"] = "a5171a6e5b64913957f2adb9bd4f251911821c4b151b2423f95e4b4e52f37954",
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
  vim.notify("OwnAI Diffview: " .. message, vim.log.levels.WARN)
end

local function current_view()
  return require("diffview.lib").get_current_view()
end

local function split_lines(value)
  value = (value or ""):gsub("\n$", "")
  if value == "" then return {} end
  return vim.split(value, "\n", { plain = true })
end

local function diff_doc(root, base, target, mode)
  local key = table.concat({ root, base, target, mode }, "\0")
  local immutable = base ~= ":index" and base ~= ":worktree"
    and target ~= ":index" and target ~= ":worktree"
  if immutable and cache[key] then return cache[key] end
  local binary = require("ownai.cli").resolve_binary()
  if not binary then return nil, "ownai binary not found" end
  local result = vim.system({ binary, "diff", "--format", "json", "--mode", mode, base, target }, {
    cwd = root, text = true, timeout = 30000,
  }):wait()
  if result.code ~= 0 then return nil, vim.trim(result.stderr or "ownai diff failed") end
  local ok, doc = pcall(vim.json.decode, result.stdout)
  if not ok or type(doc) ~= "table" or doc.schema ~= "ownai.diff.v1" or type(doc.files) ~= "table" then
    return nil, "invalid or unsupported ownai.diff.v1 document"
  end
  if immutable then cache[key] = doc end
  return doc
end

local function projections(doc)
  local files = {}
  for _, file in ipairs(doc.files) do
    files[file.path] = file
  end
  return files
end

local function release(file)
  if file._ownai_foldlevel ~= nil then
    file.winopts.foldlevel = file._ownai_foldlevel
    file._ownai_foldlevel = nil
  end
  if file._ownai_projection then
    file:dispose_buffer()
    file._ownai_projection = nil
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
      if file._ownai_projection then file:dispose_buffer() end
      if file.rev then
        if not file._ownai_projection then release(file) end
        -- Diffview normally starts diff panes at foldlevel=0. A focused
        -- projection is already short; hiding its unchanged lines can make the
        -- two sides look like a single collapsed declaration instead of a diff.
        -- Keep foldmethod=diff for highlights and open every projected line.
        if file._ownai_foldlevel == nil then
          file._ownai_foldlevel = file.winopts.foldlevel
        end
        file.winopts.foldlevel = 99
        file._ownai_projection = symbol == "a" and left or right
        file.get_data = function(_, _, pos) return pos == "left" and left or right end
      end
    end
  end
end

local function refresh_existing(view, by_section)
  if not view.files then return end
  for _, entry in view.files:iter() do
    local projected = by_section and by_section[entry.kind] and by_section[entry.kind][entry.path]
    if projected then attach(entry, projected) end
    if not projected then
      for _, file in ipairs(entry.layout:files()) do
        if file._ownai_projection then release(file) end
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
  local parse_history = GitAdapter.parse_fh_data
  local get_updated_files = DiffView.get_updated_files
  local create_buffer = File.create_buffer
  local destroy = File.destroy

  File.destroy = function(file, force)
    local projected = file._ownai_projection ~= nil
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
    file._ownai_projection = nil
    if projected then file.get_data = nil end
  end

  -- LOCAL and STAGE normally resolve to editable user buffers. Focused panes
  -- always get independent scratch buffers, including for those revisions.
  File.create_buffer = require("diffview.async").wrap(function(file, callback)
    if not file._ownai_projection or file.nulled or file.binary then
      return create_buffer(file, callback)
    end
    if file:is_valid() then return callback(file.bufnr) end
    projection_buffer = projection_buffer + 1
    local bufnr = vim.api.nvim_create_buf(false, false)
    file.bufnr = bufnr
    vim.api.nvim_buf_set_name(bufnr, "diffview://ownai/" .. projection_buffer .. "/" .. file.path)
    for option, value in pairs(File.bufopts) do vim.bo[bufnr][option] = value end
    vim.bo[bufnr].modifiable = true
    vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, file._ownai_projection)
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
    local base = data.left_hash or require("diffview.vcs.adapters.git.rev").GitRev.NULL_TREE_SHA
    local target = state.prepared_log_opts.base and state.prepared_log_opts.base.commit or data.right_hash
    if not target then return success, entry end
    local doc, err = diff_doc(adapter.ctx.toplevel, base, target, mode)
    if not doc then
      notify(err)
      return success, entry
    end
    local by_path, kept = projections(doc), {}
    for _, file in ipairs(entry.files) do
      local projected = by_path[file.path]
      if projected then
        attach(file, projected)
        kept[#kept + 1] = file
      else
        file:destroy()
      end
    end
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
        comparisons[key] = projections(doc)
        return comparisons[key]
      end
      -- Plain DiffviewOpen has separate HEAD/index and index/worktree rows.
      -- An explicit range is one comparison, even when its target is local.
      local section_maps = {}
      for _, section in ipairs({ "working", "staged" }) do
        local base, target = left, right
        if left == ":index" and right == ":worktree" and section == "staged" then
          local head = view.adapter:head_rev()
          base = head and head.commit or require("diffview.vcs.adapters.git.rev").GitRev.NULL_TREE_SHA
          target = ":index"
        end
        local map, doc_err = compare(base, target)
        if not map then notify(doc_err); return callback(err, files) end
        section_maps[section] = map
      end
      -- Conflicts use multiple index stages and have no v1 snapshot mapping.
      section_maps.conflicting = {}
      refresh_existing(view, section_maps)
      for _, section in ipairs({ "working", "staged", "conflicting" }) do
        local kept = {}
        for _, file in ipairs(files[section]) do
          local projected = section_maps[section][file.path]
          if projected then
            attach(file, projected)
            kept[#kept + 1] = file
          else
            file:destroy()
          end
        end
        files[section] = kept
      end
      callback(nil, files)
    end)
  end)

  installed = true
  return true
end

function M.on_view_opened(view)
  if not M.install() then return end
  modes[view] = require("ownai").config.default_mode
  views[view.adapter] = view
end

function M.set_mode(mode)
  local view = current_view()
  if not view then return notify("open a Diffview tab first") end
  if mode ~= "types" and mode ~= "signatures" and mode ~= "source" then
    return notify("mode must be types, signatures, or source")
  end
  if not M.install() then return end
  if modes[view] == mode then return end
  modes[view] = mode
  if view.panel and view.panel.update_entries then
    local selected = view.panel.cur_item and view.panel.cur_item[1]
    local hash = selected and selected.commit and selected.commit.hash
    local path = view.panel.cur_item and view.panel.cur_item[2] and view.panel.cur_item[2].path
    view.panel:update_entries(function()
      if not hash then return end
      for _, entry in ipairs(view.panel.entries or {}) do
        if entry.commit.hash == hash then
          local file = entry.files[1]
          for _, candidate in ipairs(entry.files) do
            if candidate.path == path then file = candidate break end
          end
          if file then view:set_file(file) end
          return
        end
      end
    end)
  elseif view.update_files then
    -- An unchanged path survives Diffview's file-list update. Drop the old
    -- scratch buffer and producer so its next load uses the selected mode.
    refresh_existing(view, nil)
    view:update_files()
  end
end

return M
