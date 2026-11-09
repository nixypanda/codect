-- codect.cache: bounded LRU cache for immutable JSON documents.

local M = {}

---@param max_entries integer
---@param max_bytes integer
---@return table
function M.new(max_entries, max_bytes)
  local entries, order = {}, {}
  local bytes = 0

  local function remove(key)
    if not entries[key] then return end
    bytes = bytes - entries[key].bytes
    entries[key] = nil
    for i, item in ipairs(order) do
      if item == key then
        table.remove(order, i)
        break
      end
    end
  end

  local function get(key)
    local entry = entries[key]
    if not entry then return nil end
    for i, item in ipairs(order) do
      if item == key then
        table.remove(order, i)
        break
      end
    end
    order[#order + 1] = key
    return entry.doc
  end

  local function put(key, doc, size)
    size = size or 0
    if max_entries <= 0 or max_bytes <= 0 or size > max_bytes then
      return false
    end
    remove(key)
    entries[key] = { doc = doc, bytes = size }
    bytes = bytes + size
    order[#order + 1] = key
    while #order > max_entries or bytes > max_bytes do
      remove(order[1])
    end
    return true
  end

  local function clear()
    entries, order = {}, {}
    bytes = 0
  end

  local function stats()
    return { count = #order, bytes = bytes }
  end

  return { get = get, put = put, clear = clear, stats = stats }
end

return M
