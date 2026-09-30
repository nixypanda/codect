-- Single-line declarations must fold (D3).

local FIX = "editors/nvim/tests/fixtures/"

return function(H)
  H.test("single-line declarations fold in signatures mode", function()
    local cases = {
      { file = "one_liners.rs", name = "one" },
      { file = "one_liners.rs", name = "LIMIT" },
      { file = "one_liners.rs", name = "NAME" },
      { file = "one_liners.elm", name = "receive" },
      { file = "one_liners.elm", name = "</>" },
      { file = "one_liners.hs", name = "hidden" },
      { file = "one_liners.py", name = "LIMIT" },
      { file = "one_liners.py", name = "NAME" },
      { file = "one_liners.py", name = "one" },
    }
    for _, case in ipairs(cases) do
      local path = FIX .. case.file
      H.open_path(path)
      vim.cmd("CodectShow signatures")

      local item = H.item_named(case.name)
      H.truthy(item, string.format("%s: %s is outlined", case.file, case.name))
      H.eq(
        vim.fn.foldclosed(item.fold_start),
        item.fold_start,
        string.format("%s: single-line %s folds", case.file, case.name)
      )
      H.truthy(vim.fn.foldtextresult(item.fold_start) ~= "", case.file .. ": fold text is shown")
    end
  end)

  H.test("single-line declarations dropped in types show a marker", function()
    H.open_path(FIX .. "one_liners.rs")
    vim.cmd("CodectShow types")

    local limit = H.item_named("LIMIT")
    H.truthy(limit, "one-line const is outlined")
    H.falsy(limit.retained, "one-line const is dropped in types")
    H.eq(vim.fn.foldclosed(limit.fold_start), limit.fold_start, "one-line const folds")
    H.contains(H.fold_text(limit), "hidden in types", "dropped marker")
  end)
end
