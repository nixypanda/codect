-- A declaration dropped by the mode stays in the outline and gets a marker.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local MAKE = P .. "fn::make"
local NEW = "impl Widget::method::new"

return function(H)
  H.test("a function dropped in types is still outlined and folded with a marker", function()
    H.open_fixture()
    vim.cmd("CodectShow types")

    local make = H.item_by_key(MAKE)
    H.truthy(make, "dropped function is present in the outline")
    H.eq(make.kind, "function")
    H.falsy(make.retained, "function is marked dropped in types")
    H.eq(vim.fn.foldclosed(make.fold_start), make.fold_start, "dropped function is folded")

    local text = H.fold_text(make)
    H.contains(text, "hidden in types", "dropped marker")
    H.contains(text, "make", "dropped marker names the declaration")
  end)

  H.test("the outline is mode-independent", function()
    H.open_fixture()

    vim.cmd("CodectShow types")
    local types_items = vim.deepcopy(H.state().items)
    local types_keys = {}
    for _, item in ipairs(types_items) do
      types_keys[item.key] = true
    end

    vim.cmd("CodectShow signatures")
    local signatures_items = H.state().items

    H.eq(#types_items, #signatures_items, "same number of declarations in both modes")
    for _, item in ipairs(signatures_items) do
      H.truthy(types_keys[item.key], "outline item present in both modes: " .. item.key)
    end
  end)

  H.test("a method dropped in types is hidden under its container", function()
    H.open_fixture()
    vim.cmd("CodectShow types")

    local new = H.item_by_key(NEW)
    H.truthy(new)
    H.falsy(new.retained, "method is dropped in types")
    -- The inherent impl is closed, so the method is inside a closed fold and
    -- its own text is not directly visible.
    local impl = H.item_by_key("impl Widget")
    H.eq(vim.fn.foldclosed(impl.fold_start), impl.fold_start, "inherent impl is folded")
    H.contains(H.fold_text(impl), "hidden in types")
  end)
end
