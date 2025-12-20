local Spinner = {}
Spinner.__index = Spinner

local FRAMES = { "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏" }
local INTERVAL_MS = 80
local TIMEOUT_MS = 40000

function Spinner.new()
    local self = setmetatable({}, Spinner)
    self.timer = nil
    self.bufnr = nil
    self.line = nil
    self.ns_id = vim.api.nvim_create_namespace("agent_amp_spinner")
    self.frame_idx = 1
    self.extmark_id = nil
    self.timeout_timer = nil
    return self
end

function Spinner:start(bufnr, line)
    self:stop()

    self.bufnr = bufnr
    self.line = line
    self.frame_idx = 1

    self.timer = vim.loop.new_timer()
    self.timer:start(0, INTERVAL_MS, vim.schedule_wrap(function()
        self:_update()
    end))

    self.timeout_timer = vim.loop.new_timer()
    self.timeout_timer:start(TIMEOUT_MS, 0, vim.schedule_wrap(function()
        vim.notify("[AgentAmp] Request timed out", vim.log.levels.WARN)
        self:stop()
    end))
end

function Spinner:stop()
    if self.timer then
        self.timer:stop()
        self.timer:close()
        self.timer = nil
    end

    if self.timeout_timer then
        self.timeout_timer:stop()
        self.timeout_timer:close()
        self.timeout_timer = nil
    end

    if self.bufnr and vim.api.nvim_buf_is_valid(self.bufnr) then
        vim.api.nvim_buf_clear_namespace(self.bufnr, self.ns_id, 0, -1)
    end

    self.bufnr = nil
    self.line = nil
    self.extmark_id = nil
end

function Spinner:is_running()
    return self.timer ~= nil
end

function Spinner:_update()
    if not self.bufnr or not vim.api.nvim_buf_is_valid(self.bufnr) then
        self:stop()
        return
    end

    local line_count = vim.api.nvim_buf_line_count(self.bufnr)
    if self.line >= line_count then
        self:stop()
        return
    end

    vim.api.nvim_buf_clear_namespace(self.bufnr, self.ns_id, 0, -1)

    local frame = FRAMES[self.frame_idx]
    self.extmark_id = vim.api.nvim_buf_set_extmark(self.bufnr, self.ns_id, self.line, 0, {
        virt_text = { { " " .. frame .. " Implementing with Amp...", "Comment" } },
        virt_text_pos = "eol",
    })

    self.frame_idx = (self.frame_idx % #FRAMES) + 1
end

return Spinner
