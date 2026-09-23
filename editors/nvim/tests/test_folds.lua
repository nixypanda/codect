-- :OwnaiFold depth semantics and nesting.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local WIDGET = P .. "type::Widget"
local NEW = "impl Widget::method::new"

return function(H)
  H.test(":OwnaiFold full unfolds everything and switching back re-folds", function()
    H.open_fixture()

    vim.cmd("OwnaiShow types")
    H.truthy(#H.closed_starts() > 0, "types folds signatures")

    vim.cmd("OwnaiFold full")
    H.eq(H.state().mode, "full")
    H.eq(#H.closed_starts(), 0, "full leaves nothing folded")

    vim.cmd("OwnaiFold types")
    H.eq(H.state().mode, "types")
    H.truthy(#H.closed_starts() > 0, "switching back to types re-folds")
  end)

  H.test(":OwnaiFold does not call the CLI", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")

    -- A stub binary that always fails proves no CLI call happens.
    local restore = H.with_stub_binary("exit 7")
    local ok = pcall(function()
      vim.cmd("OwnaiFold types")
      vim.cmd("OwnaiFold full")
      vim.cmd("OwnaiFold signatures")
    end)
    restore()
    H.truthy(ok, "local re-fold never invoked the binary")
  end)

  H.test("an impl nests its methods", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")

    local impl = H.item_by_key("impl Widget")
    local method = H.item_by_key(NEW)
    H.truthy(impl and method)

    H.eq(method.parent, impl.key, "method's parent is the impl")
    H.truthy(#impl.children >= 2, "impl has its methods as children")
    H.truthy(method.level > impl.level, "method folds deeper than its container")

    H.eq(vim.fn.foldlevel(impl.fold_start), impl.level)
    H.eq(vim.fn.foldlevel(method.fold_start), method.level)

    -- The container is open, the method inside it is closed: a genuine nested fold.
    H.falsy(vim.fn.foldclosed(impl.fold_start) == impl.fold_start, "impl stays open")
    H.eq(vim.fn.foldclosed(method.fold_start), method.fold_start, "method is closed")
  end)

  H.test("fold starts extend upward over attributes and doc comments", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local widget = H.item_by_key(WIDGET)
    H.truthy(widget.fold_start < widget.span.start_line, "struct fold start precedes the node span")

    local lines = vim.api.nvim_buf_get_lines(0, widget.fold_start - 1, widget.span.start_line - 1, false)
    H.contains(lines[1], "///", "fold start includes the doc comment")
    H.contains(lines[#lines], "#[derive", "fold start includes the attribute")

    local method = H.item_by_key(NEW)
    local method_first = vim.api.nvim_buf_get_lines(0, method.fold_start - 1, method.fold_start, false)[1]
    H.contains(method_first, "///", "method fold start includes its doc comment")
  end)
end
