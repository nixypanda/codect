-- codect.path: display-path decoding shared by show.v1 and diff.v2.
--
-- Ordinary UTF-8 paths are unchanged. New show documents opt in with
-- `path_encoding = "codect.path.v1"`; the marker is absent on legacy documents,
-- whose paths are passed through unchanged (old producers left literal
-- backslashes raw and escaped only invalid UTF-8 bytes). diff.v2 always uses
-- the encoding and needs no marker.

local M = {}

M.SHOW_ENCODING = "codect.path.v1"

---Decode one Codect display path into its exact raw byte string.
---@param value string
---@return string? raw
function M.decode_display(value)
  local out, i = {}, 1
  while i <= #value do
    local char = value:sub(i, i)
    if char ~= "\\" then
      out[#out + 1] = char
      i = i + 1
    else
      local next_char = value:sub(i + 1, i + 1)
      if next_char == "\\" then
        out[#out + 1] = "\\"
        i = i + 2
      elseif next_char == "n" then
        out[#out + 1] = "\n"
        i = i + 2
      elseif next_char == "r" then
        out[#out + 1] = "\r"
        i = i + 2
      elseif next_char == "t" then
        out[#out + 1] = "\t"
        i = i + 2
      elseif next_char == "x" then
        local hex = value:sub(i + 2, i + 3)
        if #hex ~= 2 or not hex:match("^%x%x$") then return nil end
        out[#out + 1] = string.char(tonumber(hex, 16))
        i = i + 4
      else
        return nil
      end
    end
  end
  return table.concat(out)
end

---Validate the optional show-document encoding marker and every marked path.
---@param doc table
---@return string? err
function M.check_show(doc)
  local encoding = doc.path_encoding
  if encoding == nil then
    return nil
  end
  if encoding ~= M.SHOW_ENCODING then
    return string.format("unsupported Codect path encoding %q", tostring(encoding))
  end
  for _, file in ipairs(doc.files or {}) do
    if type(file) == "table" and type(file.path) == "string" then
      local raw = M.decode_display(file.path)
      if not raw or raw == "" then
        return "invalid Codect path encoding in show document: " .. file.path
      end
    end
  end
  return nil
end

---Decode a show-document path according to its marker. Unmarked documents are
---legacy and already contain raw paths, so they must not be decoded.
---@param doc table
---@param value string
---@return string? raw
function M.decode_show(doc, value)
  if doc.path_encoding == M.SHOW_ENCODING then
    return M.decode_display(value)
  end
  return value
end

return M
