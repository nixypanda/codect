-- ownai.health: :checkhealth ownai

local cli = require("ownai.cli")

local M = {}

function M.check()
  local health = vim.health or require("vim.health")
  health.start("ownai")

  local binary = cli.resolve_binary()
  if not binary then
    health.error("ownai binary not found", {
      "Set `vim.g.ownai_binary`, export `OWNAI_BIN`, or put `ownai` on `PATH`.",
      "The Neovim plugin and the `ownai` binary are packaged separately.",
    })
    return
  end
  health.ok("ownai binary: " .. binary)

  local version = cli.version(binary)
  if version then
    health.ok("ownai version: " .. version)
  else
    health.warn("`ownai --version` did not run successfully")
  end

  health.ok("expected schema: " .. cli.SCHEMA)
  health.info("the plugin validates `schema` on every projection and fails loudly on a mismatch")

  local source = "fn main() {}\n"
  local doc, err = cli.show({
    binary = binary,
    root = vim.fn.getcwd(),
    path = "ownai-health-probe.rs",
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
