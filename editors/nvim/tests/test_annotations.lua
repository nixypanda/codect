-- Fold starts over multi-line leading annotations (D4).

local FIX = "editors/nvim/tests/fixtures/"

return function(H)
  H.test("multi-line leading block annotations are folded in", function()
    H.open_path(FIX .. "annotations.rs")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("block_doc").fold_start, 1, "Rust /** */ block")
    H.eq(H.item_named("multi_attr").fold_start, 8, "Rust multi-line #[cfg(...)]")
    H.eq(H.item_named("PLAIN").fold_start, 15, "Rust /* */ block")
    H.eq(H.item_named("INNER").fold_start, 18, "Rust /*! */ block")

    H.open_path(FIX .. "annotations.elm")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("Thing").fold_start, 1, "Elm {-| -} block")
    H.eq(H.item_named("f").fold_start, 8, "Elm {- -} block")

    H.open_path(FIX .. "annotations.hs")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("Annotated").fold_start, 1, "Haskell {-# #-} block")

    H.open_path(FIX .. "annotations.py")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("health").fold_start, 12, "Python multi-line decorator")
    H.eq(H.item_named("simple").fold_start, 20, "Python single-line decorator")
  end)

  H.test("shebangs and previous declarations' comments are not folded in", function()
    H.open_path(FIX .. "annotations.py")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("first").fold_start, 5, "Python shebang is not claimed")
    H.eq(H.item_named("second").fold_start, 8, "Python trailing comment is not claimed")

    H.open_path(FIX .. "annotations.rs")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("first").fold_start, 21, "Rust first declaration")
    H.eq(H.item_named("second").fold_start, 23, "Rust trailing comment is not claimed")
  end)

  H.test("brackets inside strings do not unbalance attributes (F1)", function()
    H.open_path(FIX .. "edge_annotations.rs")
    vim.cmd("OwnaiShow types")
    H.eq(
      H.item_named("bracket_in_string").fold_start,
      1,
      "a `]` inside a string does not reject the attribute"
    )
    H.eq(
      H.item_named("open_bracket_in_string").fold_start,
      4,
      "a `[` inside a string does not reject the attribute"
    )
  end)

  H.test("forward-attaching annotations cross a preceding declaration (F2)", function()
    H.open_path(FIX .. "edge_annotations.rs")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("b").fold_start, 9, "`#[inline]` attaches to the next method")
    H.eq(H.item_named("c").fold_start, 11, "`///` attaches to the next method")
    H.eq(
      H.item_named("after_trailing").fold_start,
      29,
      "a trailing `//` comment still stops at the declaration boundary"
    )
  end)

  H.test("blank-interior and nested block annotations fold in (F3)", function()
    H.open_path(FIX .. "edge_annotations.rs")
    vim.cmd("OwnaiShow types")
    H.eq(
      H.item_named("blank_interior").fold_start,
      15,
      "a blank line inside `/** */` is not a separator"
    )
    H.eq(
      H.item_named("nested_block").fold_start,
      22,
      "nested `/* /* */ */` folds from the outer opener"
    )

    H.open_path(FIX .. "edge_annotations.elm")
    vim.cmd("OwnaiShow types")
    H.eq(H.item_named("Nested").fold_start, 1, "nested `{- {- -} -}` folds from the outer opener")

    H.open_path(FIX .. "edge_annotations.py")
    vim.cmd("OwnaiShow types")
    H.eq(
      H.item_named("blank_decorator").fold_start,
      1,
      "a blank line inside a continued decorator is not a separator"
    )
  end)
end
