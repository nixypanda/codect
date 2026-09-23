-- :OwnaiFold depth semantics and nesting.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local WIDGET = P .. "type::Widget"
local NEW = "impl Widget::method::new"

--- One fixture per language, covering containers, dropped members, and modes.
local MODE_FIXTURES = {
  "fixtures/rust/nested-modules/input.rs",
  "fixtures/rust/implementations/input.rs",
  "fixtures/elm/ports-and-infix/input.elm",
  "fixtures/haskell/classes-instances/input.hs",
  "fixtures/python/decorators/input.py",
}

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

  H.test(":OwnaiFold reuses a cached document without calling the CLI", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")

    local marker = vim.fn.tempname()
    vim.fn.delete(marker)

    -- Cached: same content and mode, so the CLI must not run.
    local restore = H.with_marker_binary(marker)
    vim.cmd("OwnaiFold signatures")
    restore()
    H.falsy(vim.fn.filereadable(marker) == 1, "cached :OwnaiFold must not call the CLI")

    -- Uncached: a different mode must be fetched, proving the marker works.
    vim.fn.delete(marker)
    local restore_uncached = H.with_marker_binary(marker)
    H.capture_notify(function()
      vim.cmd("OwnaiFold types")
    end)
    restore_uncached()
    H.truthy(vim.fn.filereadable(marker) == 1, "uncached :OwnaiFold must call the CLI")

    vim.fn.delete(marker)
  end)

  H.test(":OwnaiFold matches a fresh :OwnaiShow for every mode and language", function()
    for _, fixture in ipairs(MODE_FIXTURES) do
      for _, mode in ipairs({ "types", "signatures" }) do
        local other = (mode == "types") and "signatures" or "types"

        H.open_path(fixture)
        vim.cmd("OwnaiShow " .. mode)
        local fresh = H.snapshot()

        H.open_path(fixture)
        vim.cmd("OwnaiShow " .. other)
        vim.cmd("OwnaiFold " .. mode)
        local folded = H.snapshot()

        H.eq(folded, fresh, string.format("%s [%s]: :OwnaiFold matches :OwnaiShow", fixture, mode))
      end
    end
  end)

  H.test("a dropped container nested in a retained container does not collapse it", function()
    local rel = "fixtures/rust/nested-modules/input.rs"
    H.open_path(rel)
    vim.cmd("OwnaiShow types")

    local inner = H.item_by_key(rel .. "::mod::outer::mod::inner")
    H.truthy(inner, "inner module is outlined")
    H.truthy(inner.retained, "inner module is retained in types")
    H.falsy(vim.fn.foldclosed(inner.fold_start) == inner.fold_start, "retained container stays open")

    local impl = H.item_by_key("impl Deep")
    H.truthy(impl, "nested impl is outlined")
    H.falsy(impl.retained, "nested impl is dropped in types")
    H.eq(vim.fn.foldclosed(impl.fold_start), impl.fold_start, "dropped nested impl is folded")
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
