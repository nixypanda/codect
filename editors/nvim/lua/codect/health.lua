-- codect.health: :checkhealth codect

local cli = require("codect.cli")

local M = {}

function M.check()
  local health = vim.health or require("vim.health")
  health.start("codect")

  local binary = cli.resolve_binary()
  if not binary then
    health.error("codect binary not found", {
      "Set `vim.g.codect_binary`, export `CODECT_BIN`, or put `codect` on `PATH`.",
      "The Neovim plugin and the `codect` binary are packaged separately.",
    })
    return
  end
  health.ok("codect binary: " .. binary)

  local version = cli.version(binary)
  if version then
    health.ok("codect version: " .. version)
  else
    health.warn("`codect --version` did not run successfully")
  end

  health.ok("expected schema: " .. cli.SCHEMA)
  health.info("the plugin validates `schema` on every projection and fails loudly on a mismatch")

  local source = "fn main() {}\n"
  local doc, err = cli.show({
    binary = binary,
    root = vim.fn.getcwd(),
    path = "codect-health-probe.rs",
    mode = "types",
    source = source,
  })
  if doc then
    health.ok("live projection probe returned schema " .. doc.schema)
  else
    health.warn("live projection probe failed (run :checkhealth from a Git repository)", {
      err or "unknown error",
    })
  end
end

return M
