-- Focused Diffview adapter for the pinned Diffview 4516612 API.
-- Diffview has no public content or file-list provider. Keep all access to its
-- internal classes here and refuse to install against an unknown revision.
local M = {}

local modes = setmetatable({}, { __mode = "k" })
local cache = {}
local views = setmetatable({}, { __mode = "v" })
local installed = false
local warned = false

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
  if cache[key] then return cache[key] end
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
  cache[key] = doc
  return doc
end

local function projections(doc)
  local files = {}
  for _, file in ipairs(doc.files) do
    files[file.path] = file
  end
  return files
end

local function attach(entry, projected)
  local left = split_lines(projected.base and projected.base.projection.text)
  local right = split_lines(projected.target and projected.target.projection.text)
  for _, file in ipairs(entry.layout:files()) do
    -- Commit buffers are Diffview-owned, read-only scratch buffers. Never
    -- install a producer on LOCAL or STAGE: those may be editable buffers.
    if file.rev and file.rev.commit then
      file.get_data = function(_, _, pos)
        return pos == "left" and left or right
      end
    end
  end
end

local function refresh_existing(view, by_path)
  if not view.files then return end
  for _, entry in view.files:iter() do
    local projected = by_path and by_path[entry.path]
    if projected then attach(entry, projected) end
    for _, file in ipairs(entry.layout:files()) do
      if file.rev and file.rev.commit then
        if not projected then file.get_data = nil end
        file:dispose_buffer()
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
  local parse_history = GitAdapter.parse_fh_data
  local get_updated_files = DiffView.get_updated_files

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
      if view.left.type ~= RevType.COMMIT or view.right.type ~= RevType.COMMIT then
        if not view._ownai_mutable_notice then
          notify("index and worktree comparisons await snapshot support; showing source")
          view._ownai_mutable_notice = true
        end
        return callback(err, files)
      end
      local doc, doc_err = diff_doc(view.adapter.ctx.toplevel, view.left.commit, view.right.commit, mode)
      if not doc then
        notify(doc_err)
        return callback(err, files)
      end
      local by_path = projections(doc)
      refresh_existing(view, by_path)
      for _, section in ipairs({ "working", "staged", "conflicting" }) do
        local kept = {}
        for _, file in ipairs(files[section]) do
          local projected = by_path[file.path]
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
