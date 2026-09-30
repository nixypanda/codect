-- setup() validation (D6), sticky-state lifetime (D7), fold-option management (D5).

return function(H)
  H.test("setup rejects an invalid default_mode and falls back", function()
    local codect = require("codect")
    local messages = H.capture_notify(function()
      codect.setup({ default_mode = "full" })
    end)
    H.eq(codect.config.default_mode, "signatures", "falls back to signatures")
    H.truthy(#messages > 0, "warns about the invalid default_mode")
    H.contains(messages[1], "default_mode")
    codect.setup({})
  end)

  H.test("clear_overrides drops every sticky override", function()
    H.open_fixture()
    vim.cmd("CodectShow types")

    local impl = H.item_by_key("impl Widget")
    vim.api.nvim_win_set_cursor(0, { impl.fold_start, 0 })
    require("codect.folds").user_action("open")

    local state = require("codect.state")
    H.truthy(state.get_override(H.state().path, impl.key) ~= nil, "override recorded")

    require("codect").clear_overrides()
    H.eq(state.get_override(H.state().path, impl.key), nil, "override cleared")
  end)

  H.test("BufWipeout prunes sticky overrides", function()
    H.open_fixture()
    vim.cmd("CodectShow types")

    local impl = H.item_by_key("impl Widget")
    vim.api.nvim_win_set_cursor(0, { impl.fold_start, 0 })
    require("codect.folds").user_action("open")

    local state = require("codect.state")
    local path = H.state().path
    H.truthy(state.get_override(path, impl.key) ~= nil, "override recorded")

    vim.cmd("silent! %bwipeout!")
    H.eq(state.get_override(path, impl.key), nil, "override pruned on wipe")
  end)

  H.test("manage_fold_options = false leaves window fold options alone", function()
    local codect = require("codect")
    codect.setup({ manage_fold_options = false })

    H.open_fixture()
    vim.wo.foldmethod = "manual"
    vim.cmd("CodectShow types")
    H.eq(vim.wo.foldmethod, "manual", "foldmethod is not clobbered")

    codect.setup({})
  end)

  H.test("manage_fold_options = true sets foldminlines for one-line folds", function()
    H.open_fixture()
    vim.cmd("CodectShow signatures")
    H.eq(vim.wo.foldminlines, 0, "foldminlines is zeroed")
  end)

  H.test("buffer_source normalizes an empty buffer to no bytes", function()
    local view = require("codect.view")
    vim.cmd("silent! %bwipeout!")
    vim.cmd("enew")
    H.eq(view.buffer_source(0), "", "empty buffer has no bytes")
    vim.api.nvim_buf_set_lines(0, 0, -1, false, { "x" })
    H.eq(view.buffer_source(0), "x\n", "a single line keeps its newline")
  end)

  H.test("manage_fold_options = false keeps retained containers open across applies (F7)", function()
    local codect = require("codect")
    codect.setup({ manage_fold_options = false })

    H.open_fixture()
    -- The user owns the fold setup; this mirrors configuring the plugin's
    -- foldexpr by hand and starting from a fully-closed window.
    vim.wo.foldmethod = "expr"
    vim.wo.foldexpr = "v:lua.require'codect.folds'.foldexpr(v:lnum)"
    vim.wo.foldenable = true
    vim.wo.foldminlines = 0
    vim.wo.foldlevel = 0

    vim.cmd("CodectShow types")
    local widget = H.item_by_key("editors/nvim/tests/fixtures/sample.rs::type::Widget")
    local make = H.item_by_key("editors/nvim/tests/fixtures/sample.rs::fn::make")
    H.falsy(vim.fn.foldclosed(widget.fold_start) == widget.fold_start, "retained struct is open")
    H.eq(vim.fn.foldclosed(make.fold_start), make.fold_start, "dropped function is closed")

    -- A repeated apply must not escalate and collapse the retained container.
    vim.cmd("CodectFold types")
    H.falsy(
      vim.fn.foldclosed(widget.fold_start) == widget.fold_start,
      "retained struct stays open after re-apply"
    )
    H.eq(vim.fn.foldclosed(make.fold_start), make.fold_start, "dropped function stays closed")

    codect.setup({})
  end)

  H.test("window-local fold options are restored when the buffer leaves the window (F8)", function()
    H.open_fixture()
    vim.wo.foldmethod = "manual"
    vim.wo.foldminlines = 7

    vim.cmd("CodectShow signatures")
    H.eq(vim.wo.foldmethod, "expr", "the plugin owns the options while the buffer is shown")
    H.eq(vim.wo.foldminlines, 0, "foldminlines is zeroed while the buffer is shown")

    -- Switch this window to a buffer the plugin does not own.
    vim.cmd("enew")
    H.eq(vim.wo.foldmethod, "manual", "foldmethod is restored on leave")
    H.eq(vim.wo.foldminlines, 7, "foldminlines is restored on leave")
  end)
end
