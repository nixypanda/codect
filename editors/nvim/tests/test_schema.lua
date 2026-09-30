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
end
