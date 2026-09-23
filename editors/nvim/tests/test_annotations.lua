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
end
