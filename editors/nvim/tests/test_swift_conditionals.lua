-- Swift conditional compilation folds: guard headers, nested groups, empty
-- branches, leading attributes, and @Suite/@Test declarations.

local FIXTURE = "editors/nvim/tests/fixtures/conditionals.swift"

local function by_suffix(st, suffix)
  for _, item in ipairs(st.items or {}) do
    if item.key:sub(-#suffix) == suffix then
      return item
    end
  end
  return nil
end

return function(H)
  H.test("Swift conditionals keep every guard as an independent fold row", function()
    H.open_path(FIXTURE)
    vim.cmd("CodectShow types")
    local st = H.state()
    local outer = by_suffix(st, "::cond::if:DEBUG")
    local outer_branch = by_suffix(st, "::cond::if:DEBUG::branch::0::if:DEBUG")
    local inner = by_suffix(st, "::cond::if:os(iOS)")
    local inner_branch = by_suffix(st, "::cond::if:os(iOS)::branch::0::if:os(iOS)")
    local suite = H.item_named("MathTests")
    local elseif_branch = by_suffix(st, "::branch::1::elseif:RELEASE")
    local else_branch = by_suffix(st, "::branch::2::else")
    H.truthy(outer and outer_branch and inner and inner_branch, "nested groups are outlined")
    H.truthy(suite and elseif_branch and else_branch, "branches and members are outlined")

    -- GROUP and the first BRANCH share a guard line; later guards do not.
    H.eq(outer.fold_start, 3, "outer #if guard")
    H.eq(outer_branch.fold_start, 3, "first BRANCH shares the #if guard")
    H.eq(inner.fold_start, 4, "nested #if guard")
    H.eq(inner_branch.fold_start, 4, "nested first BRANCH shares it")
    H.eq(suite.fold_start, 5, "@Suite folds into MathTests, not onto #if os(iOS)")
    H.eq(elseif_branch.fold_start, 10, "empty #elseif guard stays visible")
    H.eq(else_branch.fold_start, 11, "#else guard stays visible")

    local at3, at4 = 0, 0
    for _, item in ipairs(st.items) do
      if item.fold_start == 3 then at3 = at3 + 1 end
      if item.fold_start == 4 then at4 = at4 + 1 end
    end
    H.eq(at3, 2, "only the outer GROUP and its first BRANCH start at #if DEBUG")
    H.eq(at4, 2, "only the nested GROUP and its first BRANCH start at #if os(iOS)")
  end)

  H.test("a duplicated GROUP/BRANCH guard start renders one meaningful group row", function()
    H.open_path(FIXTURE)
    vim.cmd("CodectShow types")
    local buf = vim.api.nvim_get_current_buf()
    local st = H.state()
    local outer = by_suffix(st, "::cond::if:DEBUG")
    local row = st.start_to_item[3]
    H.truthy(row, "line 3 has a fold row")
    H.contains(row.signature, "#if DEBUG", "the row shows the guard, not a child")
    H.eq(row.key, by_suffix(st, "::cond::if:DEBUG::branch::0::if:DEBUG").key)

    require("codect.folds").set_open(buf, outer, false)
    H.eq(vim.fn.foldclosed(3), 3, "the group folds at the guard line")
    H.contains(vim.fn.foldtextresult(3), "#if DEBUG", "fold text shows the guard")
    H.falsy(vim.fn.foldtextresult(3):find("MathTests", 1, true), "child text does not replace the guard")
  end)

  H.test("folding a child declaration never hides a conditional guard", function()
    H.open_path(FIXTURE)
    vim.cmd("CodectShow types")
    local buf = vim.api.nvim_get_current_buf()
    local st = H.state()
    local suite = H.item_named("MathTests")
    local inner = by_suffix(st, "::cond::if:os(iOS)")
    local else_branch = by_suffix(st, "::branch::2::else")

    require("codect.folds").set_open(buf, suite, false)
    H.eq(vim.fn.foldclosed(5), 5, "MathTests folds at its own line")
    H.eq(vim.fn.foldclosed(3), -1, "#if DEBUG stays visible")
    H.eq(vim.fn.foldclosed(4), -1, "#if os(iOS) stays visible")

    require("codect.folds").set_open(buf, inner, false)
    H.eq(vim.fn.foldclosed(4), 4, "the nested group folds at its guard")
    H.eq(vim.fn.foldclosed(3), -1, "the outer guard is still visible")

    require("codect.folds").set_open(buf, else_branch, false)
    H.eq(vim.fn.foldclosed(11), 11, "the #else branch folds at its guard")
    H.contains(vim.fn.foldtextresult(11), "#else", "#else guard is shown")
  end)

  H.test("@Suite/@Test members fold without swallowing guards in signatures", function()
    H.open_path(FIXTURE)
    vim.cmd("CodectShow signatures")
    local st = H.state()
    local suite = H.item_named("MathTests")
    local test = H.item_named("add()")
    H.truthy(suite and test, "the suite and test are outlined")
    H.truthy(test.retained, "@Test is retained in signatures")
    H.eq(suite.fold_start, 5, "@Suite attaches to the suite")
    H.eq(test.fold_start, 7, "@Test attaches to the test")
    H.contains(vim.fn.foldtextresult(7), "@Test func add()", "the test row shows its signature")
    H.eq(st.start_to_item[4].key, by_suffix(st, "::cond::if:os(iOS)::branch::0::if:os(iOS)").key)
  end)
end
