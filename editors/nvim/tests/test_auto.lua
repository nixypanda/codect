-- Global auto-fold mode: enable/disable/toggle, autocmd folding, eligible skips.

local ownai = require("ownai")
local state = require("ownai.state")

local OTHER = "editors/nvim/tests/fixtures/one_liners.rs"

--- Wait for `buf` to be folded to `mode`; returns true on success.
local function wait_folded(buf, mode)
  return vim.wait(5000, function()
    local st = state.get(buf)
    return st ~= nil and st.active and st.mode == mode
  end, 20)
end

--- Pump scheduled events for `ms` without waiting for a condition.
local function pump(ms)
  vim.wait(ms, function()
    return false
  end, 20)
end

return function(H)
  H.test("enable folds the current buffer and is_enabled reflects it", function()
    H.open_fixture()
    ownai.disable()

    local messages = H.capture_notify(function()
      ownai.enable("types")
    end)

    H.truthy(ownai.is_enabled(), "is_enabled is true after enable")
    H.eq(H.state().mode, "types", "the current buffer is folded to types")
    H.truthy(#H.closed_starts() > 0, "the current buffer has closed folds")
    H.contains(messages[1], "auto-fold enabled")
    H.contains(messages[1], "types")

    ownai.disable()
  end)

  H.test("a newly opened file buffer is folded on open", function()
    H.open_fixture()
    ownai.disable()
    ownai.enable("types")

    H.open_path(OTHER)
    local buf = vim.api.nvim_get_current_buf()
    H.truthy(wait_folded(buf, "types"), "the newly opened buffer folds on BufReadPost")
    H.truthy(#H.closed_starts() > 0, "the newly opened buffer has closed folds")

    ownai.disable()
  end)

  H.test("a buffer already open before enable folds when it enters a window", function()
    H.open_path(OTHER)
    local buf = vim.api.nvim_get_current_buf()
    vim.cmd("enew")
    ownai.disable()

    ownai.enable("signatures")
    H.falsy(state.is_active(buf), "the other buffer is not folded just by enabling")

    vim.api.nvim_set_current_buf(buf)
    H.truthy(wait_folded(buf, "signatures"), "the pre-opened buffer folds on BufWinEnter")

    ownai.disable()
  end)

  H.test("disable unfolds active buffers and stops folding new files", function()
    H.open_fixture()
    ownai.disable()
    ownai.enable("types")
    H.truthy(#H.closed_starts() > 0, "buffer is folded while enabled")

    local messages = H.capture_notify(function()
      ownai.disable()
    end)
    H.falsy(ownai.is_enabled(), "is_enabled is false after disable")
    H.eq(H.state().mode, "full", "the folded buffer is switched to full")
    H.eq(#H.closed_starts(), 0, "no folds remain after disable")
    H.contains(messages[1], "auto-fold disabled")

    H.open_path(OTHER)
    local buf = vim.api.nvim_get_current_buf()
    pump(300)
    H.falsy(state.is_active(buf), "a buffer opened after disable is not folded")
  end)

  H.test("toggle flips state and behavior", function()
    H.open_fixture()
    ownai.disable()
    H.falsy(ownai.is_enabled(), "starts disabled")

    ownai.toggle("signatures")
    H.truthy(ownai.is_enabled(), "toggle turns it on")
    H.eq(H.state().mode, "signatures", "the current buffer is folded")
    H.truthy(#H.closed_starts() > 0, "folds are applied")

    ownai.toggle()
    H.falsy(ownai.is_enabled(), "toggle turns it off")
    H.eq(#H.closed_starts(), 0, "folds are cleared")
  end)

  H.test("an ineligible buffer is skipped without an error", function()
    ownai.disable()
    vim.cmd("silent! %bwipeout!")
    vim.cmd("enew")
    vim.bo.buftype = "nofile"

    local messages = H.capture_notify(function()
      ownai.enable("types")
    end)
    H.truthy(ownai.is_enabled(), "the toggle stays on for an ineligible buffer")
    H.eq(#messages, 1, "only the enabled notification, no error")
    H.contains(messages[1], "auto-fold enabled")

    -- The toggle is not broken: an eligible buffer still folds afterwards.
    H.open_fixture()
    local buf = vim.api.nvim_get_current_buf()
    H.truthy(wait_folded(buf, "types"), "an eligible buffer still folds after a skip")

    ownai.disable()
  end)

  H.test("enable rejects full and unknown modes", function()
    H.open_fixture()
    ownai.disable()

    local full_messages = H.capture_notify(function()
      ownai.enable("full")
    end)
    H.falsy(ownai.is_enabled(), "full does not enable")
    H.truthy(#full_messages > 0, "full reports an error")
    H.contains(full_messages[1], "full")

    local bogus_messages = H.capture_notify(function()
      ownai.enable("bogus")
    end)
    H.falsy(ownai.is_enabled(), "an unknown mode does not enable")
    H.contains(bogus_messages[1], "bogus")
  end)

  H.test("enable defaults to the configured mode", function()
    H.open_fixture()
    ownai.disable()

    local messages = H.capture_notify(function()
      ownai.enable()
    end)
    H.truthy(ownai.is_enabled(), "enabled")
    H.eq(H.state().mode, ownai.config.default_mode, "uses config.default_mode")
    H.contains(messages[1], ownai.config.default_mode)

    ownai.disable()
  end)
end
