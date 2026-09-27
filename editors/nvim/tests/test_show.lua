-- :OwnaiShow produces mode-correct folds on the Rust fixture.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local WIDGET = P .. "type::Widget"
local MAKE = P .. "fn::make"
local NEW = "impl Widget::method::new"

return function(H)
  H.test(":OwnaiShow types keeps types open and folds dropped signatures", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    H.eq(H.state().mode, "types")

    local widget = H.item_by_key(WIDGET)
    H.truthy(widget, "struct Widget is in the outline")
    H.falsy(vim.fn.foldclosed(widget.fold_start) == widget.fold_start, "struct stays open")

    local make = H.item_by_key(MAKE)
    H.truthy(make, "free function make is in the outline")
    H.eq(vim.fn.foldclosed(make.fold_start), make.fold_start, "make is folded in types")
    H.contains(H.fold_text(make), "hidden in types", "dropped fold text")
  end)

  H.test(":OwnaiShow signatures folds methods and shows their signatures", function()
    H.open_fixture()
    vim.cmd("OwnaiShow signatures")

    H.eq(H.state().mode, "signatures")

    local new = H.item_by_key(NEW)
    H.truthy(new, "method new is in the outline")
    H.eq(vim.fn.foldclosed(new.fold_start), new.fold_start, "method is folded")
    H.contains(H.fold_text(new), "pub fn new", "retained fold text")
    H.falsy(H.fold_text(new):find("hidden", 1, true), "retained fold text has no marker")
  end)

  H.test("the fold count and text differ between types and signatures", function()
    H.open_fixture()

    vim.cmd("OwnaiShow types")
    local types_count = #H.closed_starts()
    local types_make = H.fold_text(H.item_by_key(MAKE))

    vim.cmd("OwnaiShow signatures")
    local signatures_count = #H.closed_starts()
    local signatures_make = H.fold_text(H.item_by_key(MAKE))

    H.truthy(types_count > 0, "types folds something")
    H.truthy(signatures_count > 0, "signatures folds something")
    H.truthy(
      types_count ~= signatures_count,
      string.format("fold counts differ (types=%d signatures=%d)", types_count, signatures_count)
    )
    H.truthy(types_make ~= signatures_make, "fold text differs between modes")
    H.contains(types_make, "hidden in types")
    H.contains(signatures_make, "pub fn make")
  end)
end
