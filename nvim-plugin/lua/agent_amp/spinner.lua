local Spinner = {}
Spinner.__index = Spinner

local FRAMES = { "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏" }
local INTERVAL_MS = 80
local TIMEOUT_MS = 40000

function Spinner.new(ns_id)
    local self = setmetatable({}, Spinner)
    self.timer = nil
    self.bufnr = nil
    self.line = nil
    self.ns_id = ns_id
    self.frame_idx = 1
    self.extmark_id = nil
    self.timeout_timer = nil
    self.preview_lines = nil
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

    if self.bufnr and self.extmark_id and vim.api.nvim_buf_is_valid(self.bufnr) then
        pcall(vim.api.nvim_buf_del_extmark, self.bufnr, self.ns_id, self.extmark_id)
    end

    self.bufnr = nil
    self.line = nil
    self.extmark_id = nil
    self.preview_lines = nil
end

function Spinner:is_running()
    return self.timer ~= nil
end

function Spinner:set_preview(text)
    if not text or text == "" then
        self.preview_lines = nil
        return
    end
    self.preview_lines = vim.split(text, "\n", { plain = true })
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

    if self.extmark_id then
        pcall(vim.api.nvim_buf_del_extmark, self.bufnr, self.ns_id, self.extmark_id)
    end

    local frame = FRAMES[self.frame_idx]
    local extmark_opts = {
        virt_text = { { " " .. frame .. " Implementing with Amp...", "Comment" } },
        virt_text_pos = "eol",
    }

    if self.preview_lines and #self.preview_lines > 0 then
        local virt_lines = {}
        for _, line in ipairs(self.preview_lines) do
            table.insert(virt_lines, { { line, "Comment" } })
        end
        extmark_opts.virt_lines = virt_lines
    end

    self.extmark_id = vim.api.nvim_buf_set_extmark(self.bufnr, self.ns_id, self.line, 0, extmark_opts)

    self.frame_idx = (self.frame_idx % #FRAMES) + 1
end

local SpinnerManager = {}
SpinnerManager.__index = SpinnerManager

function SpinnerManager.new()
    local self = setmetatable({}, SpinnerManager)
    self.ns_id = vim.api.nvim_create_namespace("agent_amp_spinner")
    self.spinners = {}
    return self
end

function SpinnerManager:start(job_id, bufnr, line)
    if self.spinners[job_id] then
        self.spinners[job_id]:stop()
    end

    local spinner = Spinner.new(self.ns_id)
    spinner:start(bufnr, line)
    self.spinners[job_id] = spinner
end

function SpinnerManager:stop(job_id)
    local spinner = self.spinners[job_id]
    if spinner then
        spinner:stop()
        self.spinners[job_id] = nil
    end
end

function SpinnerManager:stop_all()
    for job_id, spinner in pairs(self.spinners) do
        spinner:stop()
        self.spinners[job_id] = nil
    end
end

function SpinnerManager:is_running(job_id)
    local spinner = self.spinners[job_id]
    return spinner and spinner:is_running()
end

function SpinnerManager:has_running()
    for _, spinner in pairs(self.spinners) do
        if spinner:is_running() then
            return true
        end
    end
    return false
end

function SpinnerManager:set_preview(job_id, text)
    local spinner = self.spinners[job_id]
    if spinner and spinner:is_running() then
        spinner:set_preview(text)
    end
end

function SpinnerManager:find_job_by_uri_line(uri, line)
    for job_id, spinner in pairs(self.spinners) do
        if spinner:is_running() and spinner.line == line then
            local bufnr = spinner.bufnr
            if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
                local buf_uri = vim.uri_from_bufnr(bufnr)
                if buf_uri == uri then
                    return job_id
                end
            end
        end
    end
    return nil
end

return SpinnerManager
