-- Schema guard and error surfaces.

local cli = require("codect.cli")
local view = require("codect.view")

return function(H)
  H.test("cli.check_schema accepts the exact schema", function()
    H.eq(cli.check_schema({ schema = "codect.show.v1" }), nil)
  end)

  H.test("cli.check_schema rejects a different major version loudly", function()
    local err = cli.check_schema({ schema = "codect.show.v2" })
    H.truthy(err, "major mismatch is an error")
    H.contains(err, "major version mismatch")
    H.contains(err, "codect.show.v2")
  end)

  H.test("cli.check_schema rejects an unknown schema", function()
    H.truthy(cli.check_schema({ schema = "other.thing.v1" }), "unknown schema is an error")
    H.truthy(cli.check_schema({}), "missing schema is an error")
  end)

  H.test(":CodectShow fails cleanly on a schema mismatch", function()
    H.open_fixture()
    local restore = H.with_stub_binary(
      [[printf '%s' '{"schema":"codect.show.v9","input":"stdin","revision":null,"mode":"types","files":[]}']]
    )
    local messages = H.capture_notify(function()
      vim.cmd("CodectShow types")
    end)
    restore()

    H.truthy(#messages > 0, "an error was reported")
    H.contains(messages[1], "schema", "error mentions the schema")
  end)

  H.test(":CodectShow surfaces a failing binary", function()
    H.open_fixture()
    local restore = H.with_stub_binary([[echo "unsupported source path" >&2; exit 1]])
    local messages = H.capture_notify(function()
      vim.cmd("CodectShow types")
    end)
    restore()

    H.truthy(#messages > 0, "an error was reported")
    H.contains(messages[1], "unsupported source path")
  end)

  H.test(":CodectShow refuses a non-file buffer", function()
    vim.cmd("silent! %bwipeout!")
    vim.cmd("enew")
    vim.bo.buftype = "nofile"
    local messages = H.capture_notify(function()
      vim.cmd("CodectShow types")
    end)
    H.truthy(#messages > 0, "an error was reported")
    H.contains(messages[1], "not a file")
  end)

  H.test("resolve_target rejects a buffer outside a Git repository", function()
    local path = vim.fn.tempname() .. ".rs"
    local file = assert(io.open(path, "w"))
    file:write("fn main() {}\n")
    file:close()
    vim.cmd("silent! %bwipeout!")
    vim.cmd("edit " .. vim.fn.fnameescape(path))
    local _, err = view.resolve_target(vim.api.nvim_get_current_buf())
    vim.fn.delete(path)
    H.truthy(err, "outside-repo file is rejected")
    H.contains(err, "Git repository")
  end)

  H.test("shared path decoding separates escaped literals from invalid bytes", function()
    local paths = require("codect.path")
    H.eq(paths.decode_display("plain/path.rs"), "plain/path.rs")
    H.eq(paths.decode_display("a\\\\xFF.rs"), "a\\xFF.rs")
    H.eq(paths.decode_display("a\\xFF.rs"), "a\xFF.rs")
    H.eq(paths.decode_display("a\\nb.rs"), "a\nb.rs")
    H.eq(paths.decode_display("a\\\\nb.rs"), "a\\nb.rs")
    H.eq(paths.decode_display("\\x41"), "A")
    H.eq(paths.decode_display("\\\\x41"), "\\x41")
  end)

  H.test("show path decoding only applies to the known encoding marker", function()
    local paths = require("codect.path")
    H.eq(paths.check_show({}), nil)
    H.eq(paths.check_show({ path_encoding = "codect.path.v1" }), nil)
    H.truthy(paths.check_show({ path_encoding = "codect.path.v9" }))
    H.contains(
      paths.check_show({ path_encoding = "codect.path.v9" }),
      "unsupported Codect path encoding"
    )
    local malformed = {
      path_encoding = "codect.path.v1",
      files = { { path = "bad\\q.rs" } },
    }
    H.contains(paths.check_show(malformed), "invalid Codect path encoding")
    H.eq(paths.decode_display("bad\\q.rs"), nil)
    H.eq(paths.decode_display("bad\\x4"), nil)
    H.eq(paths.decode_show({}, "lit\\name.rs"), "lit\\name.rs")
    H.eq(
      paths.decode_show({ path_encoding = "codect.path.v1" }, "lit\\\\name.rs"),
      "lit\\name.rs"
    )
  end)

  H.test("document cache is bounded, LRU, and skips oversized documents", function()
    local docs = require("codect.cache")
    local cache = docs.new(2, 10)
    H.truthy(cache.put("a", { id = "a" }, 4))
    H.truthy(cache.put("b", { id = "b" }, 4))
    H.eq(cache.get("a").id, "a")
    H.truthy(cache.put("c", { id = "c" }, 4))
    H.eq(cache.get("b"), nil)
    H.eq(cache.get("a").id, "a")
    H.eq(cache.get("c").id, "c")

    local bytes = docs.new(4, 8)
    H.truthy(bytes.put("a", {}, 4))
    H.truthy(bytes.put("b", {}, 4))
    H.truthy(bytes.put("c", {}, 1))
    H.eq(bytes.get("a"), nil)
    H.eq(bytes.get("b") ~= nil and bytes.get("c") ~= nil, true)
    H.eq(bytes.stats().bytes <= 8, true)

    local small = docs.new(2, 5)
    H.falsy(small.put("large", {}, 6))
    H.eq(small.stats().count, 0)
    H.eq(small.get("large"), nil)
  end)
end
