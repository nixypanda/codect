-- codect.cli: binary discovery, invocation, JSON decoding, schema guard.
--
-- The plugin never links against the binary; it resolves an executable and
-- speaks the `codect.show.v1` JSON contract over stdin. All projection input is
-- read-only: the binary reads stdin (or the worktree) and writes nothing.

local M = {}

--- The schema this plugin understands. A different value (or major version)
--- is a fatal, explicit mismatch.
M.SCHEMA = "codect.show.v1"

--- Resolve the `codect` executable.
---
--- Order: `vim.g.codect_binary`, the `CODECT_BIN` environment variable, then
--- `codect` on `PATH`. Returns the path, or nil when nothing is found.
function M.resolve_binary()
  local override = vim.g.codect_binary
  if type(override) == "string" and override ~= "" then
    return override
  end

  local env = vim.env.CODECT_BIN
  if type(env) == "string" and env ~= "" then
    return env
  end

  local on_path = vim.fn.exepath("codect")
  if type(on_path) == "string" and on_path ~= "" then
    return on_path
  end

  return nil
end

--- Validate the document's `schema` field.
---
--- Returns nil when the schema is the expected exact value, or a human-readable
--- error string otherwise. A different major version is called out explicitly.
function M.check_schema(doc)
  if type(doc) ~= "table" then
    return "codect returned a non-object JSON document"
  end
  if type(doc.schema) ~= "string" then
    return "codect returned a document without a `schema` field"
  end
  if doc.schema == M.SCHEMA then
    return nil
  end

  local major = doc.schema:match("^codect%.show%.v(%d+)")
  if major and tonumber(major) ~= 1 then
    return string.format(
      "codect schema major version mismatch: plugin expects %s, binary emitted %q",
      M.SCHEMA,
      doc.schema
    )
  end

  return string.format("unexpected codect schema %q (plugin expects %s)", doc.schema, M.SCHEMA)
end

--- Project `source` through `codect show --format json --stdin`.
---
--- opts:
---   * binary  (string?)  explicit executable; resolved when omitted
---   * root    (string)   repository root used as the subprocess cwd
---   * path    (string)   repository-relative path of the buffer's file
---   * mode    ("types"|"signatures")
---   * source  (string)   exact buffer bytes
---   * timeout (number?)  milliseconds, default 15000
---
--- Returns the decoded document, or nil plus a clear error string.
function M.show(opts)
  local binary = opts.binary or M.resolve_binary()
  if not binary then
    return nil,
      "codect binary not found; set vim.g.codect_binary or CODECT_BIN, or put `codect` on PATH"
  end

  local cmd = {
    binary,
    "show",
    "--format",
    "json",
    "--stdin",
    "--path",
    opts.path,
    "--mode",
    opts.mode,
  }

  local ok, obj = pcall(vim.system, cmd, {
    cwd = opts.root,
    stdin = opts.source,
    text = true,
    timeout = opts.timeout or 15000,
  })
  if not ok then
    return nil, string.format("failed to run %s: %s", binary, tostring(obj))
  end

  local res = obj:wait()
  if res.code ~= 0 then
    local detail = res.stderr or ""
    if detail == "" then
      detail = string.format("codect exited with code %d", res.code)
    end
    return nil, vim.trim(detail)
  end

  local decoded_ok, doc = pcall(vim.json.decode, res.stdout)
  if not decoded_ok then
    return nil, "codect returned invalid JSON: " .. tostring(doc)
  end

  local schema_err = M.check_schema(doc)
  if schema_err then
    return nil, schema_err
  end

  return doc
end

--- Run `codect --version` and return the trimmed output, or nil.
function M.version(binary)
  binary = binary or M.resolve_binary()
  if not binary then
    return nil
  end
  local ok, obj = pcall(vim.system, { binary, "--version" }, { text = true, timeout = 5000 })
  if not ok then
    return nil
  end
  local res = obj:wait()
  if res.code ~= 0 then
    return nil
  end
  return vim.trim(res.stdout or "")
end

return M
