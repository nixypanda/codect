-- setup() validation (D6), sticky-state lifetime (D7), fold-option management (D5).

return function(H)
  H.test("setup rejects an invalid default_mode and falls back", function()
    local ownai = require("ownai")
    local messages = H.capture_notify(function()
      ownai.setup({ default_mode = "full" })
    end)
    H.eq(ownai.config.default_mode, "signatures", "falls back to signatures")
    H.truthy(#messages > 0, "warns about the invalid default_mode")
    H.contains(messages[1], "default_mode")
    ownai.setup({})
  end)

  H.test("clear_overrides drops every sticky override", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local impl = H.item_by_key("impl Widget")
    vim.api.nvim_win_set_cursor(0, { impl.fold_start, 0 })
    require("ownai.folds").user_action("open")

    local state = require("ownai.state")
    H.truthy(state.get_override(H.state().path, impl.key) ~= nil, "override recorded")

    require("ownai").clear_overrides()
    H.eq(state.get_override(H.state().path, impl.key), nil, "override cleared")
  end)

  H.test("BufWipeout prunes sticky overrides", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local impl = H.item_by_key("impl Widget")
    vim.api.nvim_win_set_cursor(0, { impl.fold_start, 0 })
    require("ownai.folds").user_action("open")

    local state = require("ownai.state")
    local path = H.state().path
    H.truthy(state.get_override(path, impl.key) ~= nil, "override recorded")

    vim.cmd("silent! %bwipeout!")
    H.eq(state.get_override(path, impl.key), nil, "override pruned on wipe")
  end)

  H.test("manage_fold_options = false leaves window fold options alone", function()
    local ownai = require("ownai")
    ownai.setup({ manage_fold_options = false })

    H.open_fixture()
    vim.wo.foldmethod = "manual"
    vim.cmd("OwnaiShow types")
    H.eq(vim.wo.foldmethod, "manual", "foldmethod is not clobbered")

    ownai.setup({})
  end)

  H.test("manage_fold_options = true sets foldminlines for one-line folds", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")
    H.eq(vim.wo.foldminlines, 0, "foldminlines is zeroed")
  end)
end
