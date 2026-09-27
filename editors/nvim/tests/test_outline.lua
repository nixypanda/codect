-- :OwnaiOutline picker and ]f/[f navigation.

local view = require("ownai.view")

return function(H)
  H.test(":OwnaiOutline lists declarations and jumps to the chosen one", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local offered
    local original = vim.ui.select
    vim.ui.select = function(items, _, on_choice)
      offered = items
      for _, label in ipairs(items) do
        if label:find("make", 1, true) then
          on_choice(label)
          return
        end
      end
      on_choice(nil)
    end
    vim.cmd("OwnaiOutline")
    vim.ui.select = original

    H.truthy(offered and #offered > 0, "picker offered declarations")
    local make = H.item_by_key("editors/nvim/tests/fixtures/sample.rs::fn::make")
    H.eq(vim.api.nvim_win_get_cursor(0)[1], make.fold_start, "cursor jumped to the chosen declaration")
  end)

  H.test("]f and [f move between declarations", function()
    H.open_fixture()
    vim.cmd("OwnaiShow types")

    local struct = H.item_by_key("editors/nvim/tests/fixtures/sample.rs::type::Widget")
    local enum = H.item_by_key("editors/nvim/tests/fixtures/sample.rs::type::Kind")

    vim.api.nvim_win_set_cursor(0, { struct.fold_start, 0 })
    view.goto_declaration(1)
    H.eq(vim.api.nvim_win_get_cursor(0)[1], enum.fold_start, "next declaration")

    view.goto_declaration(-1)
    H.eq(vim.api.nvim_win_get_cursor(0)[1], struct.fold_start, "previous declaration")
  end)

  H.test("the first :OwnaiFold attaches buffer keymaps (F4)", function()
    H.open_fixture()
    H.falsy(vim.fn.maparg("]f", "n", false, true).buffer, "no buffer-local map before folding")

    vim.cmd("OwnaiFold types")
    H.eq(vim.fn.maparg("]f", "n", false, true).buffer, 1, "buffer-local map attached after folding")
    H.eq(vim.fn.maparg("[f", "n", false, true).buffer, 1, "the previous-declaration map is attached too")
  end)
end
