-- ownai: setup(), configuration, and buffer-local keymaps.

local M = {}

M.defaults = {
  -- Mode used when :OwnaiShow/:OwnaiFold is called without an argument.
  default_mode = "signatures",
  -- Debounce for TextChanged refreshes, in milliseconds.
  debounce_ms = 200,
  -- Set the window-local fold options (foldmethod, foldexpr, foldtext,
  -- foldenable, foldminlines, foldlevel) for OwnAI buffers. Set false to keep
  -- your own fold configuration; OwnAI will still open/close its folds.
  manage_fold_options = true,
  keymaps = {
    -- Set to false to register no keymaps at all.
    enabled = true,
    -- Set any individual binding to false or "" to leave it unregistered.
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
    desc = "OwnAI: " .. desc,
  })
end

--- Register the buffer-local keymaps for an active OwnAI buffer.
function M.attach_keymaps(buf)
  local keymaps = M.config.keymaps
  if not keymaps or keymaps.enabled == false then
    return
  end

  local folds = require("ownai.folds")
  local view = require("ownai.view")

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

--- Merge user options and install autocmds.
function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts or {})

  -- `:OwnaiShow` and `:OwnaiFold` only accept types|signatures, so a default
  -- of `full` would break both commands.
  local default_mode = M.config.default_mode
  if default_mode ~= "types" and default_mode ~= "signatures" then
    vim.notify(
      string.format(
        "OwnAI: default_mode must be `types` or `signatures`, got %s; using %q",
        vim.inspect(default_mode),
        M.defaults.default_mode
      ),
      vim.log.levels.WARN
    )
    M.config.default_mode = M.defaults.default_mode
  end

  require("ownai.view").setup_autocmds()

  for buf in pairs(require("ownai.state").buffers) do
    if vim.api.nvim_buf_is_valid(buf) then
      M.attach_keymaps(buf)
    end
  end

  return M.config
end

--- Drop every sticky fold override for every file in this session.
function M.clear_overrides()
  require("ownai.state").clear_all_overrides()
end

return M
