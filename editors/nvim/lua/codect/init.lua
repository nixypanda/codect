-- codect: setup(), configuration, and buffer-local keymaps.
--
-- `full` is a local unfold target, so `:CodectShow` and `default_mode` accept
-- only `types`/`signatures`.

local M = {}

local SHOW_MODES = { types = true, signatures = true }

M.defaults = {
  default_mode = "signatures",
  debounce_ms = 200,
  manage_fold_options = true,
  keymaps = {
    enabled = true,
    next_declaration = "]f",
    prev_declaration = "[f",
    cycle_mode = "<leader>of",
    fold_toggle = "za",
    fold_open = "zo",
    fold_close = "zc",
    fold_open_recursive = "zO",
    fold_close_recursive = "zC",
  },
}

M.config = vim.deepcopy(M.defaults)

local function map(buf, lhs, rhs, desc)
  if not lhs or lhs == "" or lhs == false then
    return
  end
  vim.keymap.set("n", lhs, rhs, {
    buffer = buf,
    silent = true,
    nowait = true,
    desc = "Codect: " .. desc,
  })
end

function M.attach_keymaps(buf)
  local keymaps = M.config.keymaps
  if not keymaps or keymaps.enabled == false then
    return
  end

  local folds = require("codect.folds")
  local view = require("codect.view")

  map(buf, keymaps.next_declaration, function()
    view.goto_declaration(1)
  end, "next declaration")
  map(buf, keymaps.prev_declaration, function()
    view.goto_declaration(-1)
  end, "previous declaration")
  map(buf, keymaps.cycle_mode, function()
    view.cycle()
  end, "cycle fold depth")

  map(buf, keymaps.fold_toggle, function()
    folds.user_action("toggle")
  end, "toggle fold")
  map(buf, keymaps.fold_open, function()
    folds.user_action("open")
  end, "open fold")
  map(buf, keymaps.fold_close, function()
    folds.user_action("close")
  end, "close fold")
  map(buf, keymaps.fold_open_recursive, function()
    folds.user_action("open_recursive")
  end, "open fold recursively")
  map(buf, keymaps.fold_close_recursive, function()
    folds.user_action("close_recursive")
  end, "close fold recursively")
end

function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts or {})

  local default_mode = M.config.default_mode
  if not SHOW_MODES[default_mode] then
    vim.notify(
      string.format(
        "Codect: default_mode must be `types` or `signatures`, got %s; using %q",
        vim.inspect(default_mode),
        M.defaults.default_mode
      ),
      vim.log.levels.WARN
    )
    M.config.default_mode = M.defaults.default_mode
  end

  require("codect.view").setup_autocmds()

  for buf in pairs(require("codect.state").buffers) do
    if vim.api.nvim_buf_is_valid(buf) then
      M.attach_keymaps(buf)
    end
  end

  return M.config
end

function M.clear_overrides()
  require("codect.state").clear_all_overrides()
end

function M.enable(mode)
  return require("codect.view").enable(mode)
end

function M.disable()
  return require("codect.view").disable()
end

function M.toggle(mode)
  return require("codect.view").toggle(mode)
end

function M.is_enabled()
  return require("codect.view").is_enabled()
end

return M
