-- :CodectShow produces mode-correct folds on the Rust fixture.

local P = "editors/nvim/tests/fixtures/sample.rs::"
local WIDGET = P .. "type::Widget"
local MAKE = P .. "fn::make"
local NEW = "impl Widget::method::new"

local function open_abs(path)
  vim.cmd("silent! %bwipeout!")
  local buf = vim.fn.bufadd(path)
  vim.fn.bufload(buf)
  vim.api.nvim_set_current_buf(buf)
  return buf
end

local function stub_show(doc)
  return "printf '%s' " .. vim.fn.shellescape(vim.json.encode(doc))
end

local function show_doc(marker, correct_path)
  local function file(path)
    return {
      path = path,
      language = "rust",
      projection = { text = "", items = {} },
      outline = {},
    }
  end
  local doc = {
    schema = "codect.show.v1",
    input = "stdin",
    revision = vim.NIL,
    mode = "signatures",
    files = { file("decoy.rs"), file(correct_path) },
  }
  if marker then doc.path_encoding = marker end
  return doc
end

return function(H)
  H.test(":CodectShow types keeps types open and folds dropped signatures", function()
    H.open_fixture()
    vim.cmd("CodectShow types")

    H.eq(H.state().mode, "types")

    local widget = H.item_by_key(WIDGET)
    H.truthy(widget, "struct Widget is in the outline")
    H.falsy(vim.fn.foldclosed(widget.fold_start) == widget.fold_start, "struct stays open")

    local make = H.item_by_key(MAKE)
    H.truthy(make, "free function make is in the outline")
    H.eq(vim.fn.foldclosed(make.fold_start), make.fold_start, "make is folded in types")
    H.contains(H.fold_text(make), "hidden in types", "dropped fold text")
  end)

  H.test(":CodectShow signatures folds methods and shows their signatures", function()
    H.open_fixture()
    vim.cmd("CodectShow signatures")

    H.eq(H.state().mode, "signatures")

    local new = H.item_by_key(NEW)
    H.truthy(new, "method new is in the outline")
    H.eq(vim.fn.foldclosed(new.fold_start), new.fold_start, "method is folded")
    H.contains(H.fold_text(new), "pub fn new", "retained fold text")
    H.falsy(H.fold_text(new):find("hidden", 1, true), "retained fold text has no marker")
  end)

  H.test("the fold count and text differ between types and signatures", function()
    H.open_fixture()

    vim.cmd("CodectShow types")
    local types_count = #H.closed_starts()
    local types_make = H.fold_text(H.item_by_key(MAKE))

    vim.cmd("CodectShow signatures")
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

  H.test("show path selection follows the encoding marker and keeps encoded labels", function()
    local root = vim.fn.tempname()
    vim.fn.mkdir(root, "p")
    root = vim.fn.resolve(root)
    local init = vim.fn.system({ "git", "-C", root, "init", "-q" })
    assert(vim.v.shell_error == 0, init)

    local raw_path = "lit\\name.rs"
    local display_path = "lit\\\\name.rs"
    local absolute = root .. "/" .. raw_path
    local file = assert(io.open(absolute, "wb"))
    file:write("fn main() {}\n")
    file:close()

    local function run_with(doc)
      open_abs(absolute)
      require("codect.state").clear_overrides(raw_path)
      local restore = H.with_stub_binary(stub_show(doc))
      local messages = H.capture_notify(function()
        vim.cmd("CodectShow signatures")
      end)
      restore()
      return messages
    end

    local messages = run_with(show_doc("codect.path.v1", display_path))
    H.eq(#messages, 0, table.concat(messages, "\n"))
    local st = H.state()
    H.truthy(st, ":CodectShow did not install state for the marked path")
    H.eq(st.path, raw_path, "state path is not the raw byte path")
    H.eq(st.file_entry.path, display_path, "the encoded label was not preserved")

    messages = run_with(show_doc(nil, raw_path))
    H.eq(#messages, 0, table.concat(messages, "\n"))
    st = H.state()
    H.truthy(st, ":CodectShow did not select an unmarked legacy path")
    H.eq(st.path, raw_path, "legacy raw path was decoded")
    H.eq(st.file_entry.path, raw_path, "legacy path label changed")

    messages = run_with(show_doc("codect.path.v9", display_path))
    H.truthy(#messages > 0, "unknown path encoding did not fail")
    H.contains(messages[1], "unsupported Codect path encoding")

    messages = run_with(show_doc("codect.path.v1", "bad\\q.rs"))
    H.truthy(#messages > 0, "malformed marked path did not fail")
    H.contains(messages[1], "invalid Codect path encoding")

    local sole = show_doc("codect.path.v1", "decoy.rs")
    sole.files = { sole.files[1] }
    messages = run_with(sole)
    H.truthy(#messages > 0, "marked documents still used the single-file fallback")
    H.contains(messages[1], "no projection")

    sole = show_doc(nil, "decoy.rs")
    sole.files = { sole.files[1] }
    messages = run_with(sole)
    H.eq(#messages, 0, "legacy single-file fallback stopped working")
    st = H.state()
    H.truthy(st, "legacy single-file fallback did not install state")
    H.eq(st.file_entry.path, "decoy.rs", "legacy fallback selected the wrong file")

    vim.cmd("silent! %bwipeout!")
    vim.fn.delete(root, "rf")
  end)
end
