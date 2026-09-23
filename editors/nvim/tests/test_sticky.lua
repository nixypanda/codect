-- Sticky expansion: a user's open/closed choice survives refresh and mode switch.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local WIDGET = P .. "type::Widget"
local NEW = "impl Widget::method::new"

local folds = require("ownai.folds")
local state = require("ownai.state")
local view = require("ownai.view")

return function(H)
  local function put_cursor(line)
    vim.api.nvim_win_set_cursor(0, { line, 0 })
  end

  H.test("an opened declaration stays open across a refresh", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")
    local buf = vim.api.nvim_get_current_buf()

    local new = H.item_by_key(NEW)
    -- The method is hidden under the closed inherent impl; open the impl first.
    local impl = H.item_by_key("impl Widget")
    put_cursor(impl.fold_start)
    folds.user_action("open")
    H.falsy(vim.fn.foldclosed(impl.fold_start) == impl.fold_start, "impl opened")

    -- Edit the buffer so a refresh really re-projects.
    vim.api.nvim_buf_set_lines(buf, -1, -1, false, { "// sticky edit" })
    view.refresh(buf)

    local impl_after = H.item_by_key("impl Widget")
    H.falsy(vim.fn.foldclosed(impl_after.fold_start) == impl_after.fold_start, "impl stays open after refresh")
    H.eq(state.get_override(H.state().path, impl_after.key), true, "override recorded as open")

    -- The method can now be opened on its own and must stay open too.
    put_cursor(new.fold_start)
    folds.user_action("open")
    view.refresh(buf)
    H.falsy(vim.fn.foldclosed(new.fold_start) == new.fold_start, "method stays open after refresh")
  end)

  H.test("a closed declaration stays closed across a refresh", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")
    local buf = vim.api.nvim_get_current_buf()

    local widget = H.item_by_key(WIDGET)
    put_cursor(widget.fold_start)
    folds.user_action("close")
    H.eq(vim.fn.foldclosed(widget.fold_start), widget.fold_start, "struct closed")

    vim.api.nvim_buf_set_lines(buf, -1, -1, false, { "// another edit" })
    view.refresh(buf)

    local widget_after = H.item_by_key(WIDGET)
    H.eq(vim.fn.foldclosed(widget_after.fold_start), widget_after.fold_start, "struct stays closed")
    H.eq(state.get_override(H.state().path, widget_after.key), false, "override recorded as closed")
  end)

  H.test("sticky state survives a local mode switch", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local impl = H.item_by_key("impl Widget")
    put_cursor(impl.fold_start)
    folds.user_action("open")

    local widget = H.item_by_key(WIDGET)
    put_cursor(widget.fold_start)
    folds.user_action("close")

    vim.cmd("OwnaiFold signatures")

    local impl_after = H.item_by_key("impl Widget")
    local widget_after = H.item_by_key(WIDGET)
    H.falsy(vim.fn.foldclosed(impl_after.fold_start) == impl_after.fold_start, "impl stays open")
    H.eq(vim.fn.foldclosed(widget_after.fold_start), widget_after.fold_start, "struct stays closed")
  end)

  H.test("overrides are keyed by (path, stable_key)", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")

    local widget = H.item_by_key(WIDGET)
    put_cursor(widget.fold_start)
    folds.user_action("close")

    H.eq(state.get_override(H.fixture_rel, WIDGET), false)
    H.eq(state.get_override(H.fixture_rel, "some::other::key"), nil)
  end)
end
