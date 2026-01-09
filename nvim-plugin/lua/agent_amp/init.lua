local SpinnerManager = require("agent_amp.spinner")
local LspClient = require("agent_amp.lsp")

local AgentAmp = {}
AgentAmp.__index = AgentAmp

local instance = nil

function AgentAmp.new(opts)
    local self = setmetatable({}, AgentAmp)
    self.opts = opts or {}
    self.spinner_manager = SpinnerManager.new()
    self.pending_jobs = {}
    self.lsp_client = LspClient.new({
        cmd = self.opts.cmd,
        on_apply_edit = function(err, result, ctx)
            self:_on_apply_edit(err, result, ctx)
        end,
        on_progress = function(params)
            self:_on_progress(params)
        end,
        on_job_completed = function(params)
            self:_on_job_completed(params)
        end,
    })
    return self
end

function AgentAmp:_on_apply_edit(_err, _result, _ctx)
    -- Spinner cleanup is now handled by _on_job_completed notification.
    -- This handler just lets the LSP apply the edit via the default handler.
    -- We keep this method for potential future use (e.g., logging, metrics).
end

function AgentAmp:_on_job_completed(params)
    if not params or not params.job_id then
        return
    end

    local job_id = params.job_id

    -- Stop the spinner for this job
    self.spinner_manager:stop(job_id)

    -- Clean up pending job using pending_id if available (direct match)
    if params.pending_id and self.pending_jobs[params.pending_id] then
        self.pending_jobs[params.pending_id] = nil
    else
        -- Fall back to searching by server_job_id
        for pending_id, pending_info in pairs(self.pending_jobs) do
            if pending_info.server_job_id == job_id then
                self.pending_jobs[pending_id] = nil
                break
            end
        end
    end

    -- Show notification based on success/failure
    if params.success then
        vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
    else
        local msg = params.error or "Implementation failed"
        vim.notify("[AgentAmp] " .. msg, vim.log.levels.ERROR)
    end
end

function AgentAmp:_on_progress(params)
    if not params or not params.job_id then
        return
    end

    local server_job_id = params.job_id
    local uri = params.uri
    local line = params.line

    -- Check if this is a new job that needs to replace a pending placeholder
    if not self.spinner_manager:is_running(server_job_id) then
        local pending_job_id = nil

        -- PRIORITY 1: Use pending_id from server for direct, unambiguous matching
        if params.pending_id and self.pending_jobs[params.pending_id] then
            pending_job_id = params.pending_id
        end

        -- PRIORITY 2: Fall back to line-based matching if no pending_id
        if not pending_job_id then
            -- Find any pending job at the original line or nearby
            pending_job_id = self.spinner_manager:find_job_by_uri_line(uri, line)

            -- Only accept the primary match if it's actually a pending job
            -- (find_job_by_uri_line can return already-transitioned server jobs too)
            if pending_job_id and not pending_job_id:match("^pending%-") then
                pending_job_id = nil
            end

            -- If no pending match found, find the pending job with the closest line number
            if not pending_job_id then
                local best_match = nil
                local best_line_diff = math.huge

                for pid, pinfo in pairs(self.pending_jobs) do
                    if pid:match("^pending%-") then
                        local bufnr = pinfo.bufnr
                        if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
                            local buf_uri = vim.uri_from_bufnr(bufnr)
                            if buf_uri == uri then
                                -- Find the pending job with the closest line number
                                local line_diff = math.abs(pinfo.line - line)
                                if line_diff < best_line_diff then
                                    best_line_diff = line_diff
                                    best_match = pid
                                end
                            end
                        end
                    end
                end

                pending_job_id = best_match
            end
        end

        if pending_job_id and pending_job_id:match("^pending%-") then
            -- Store the mapping from pending to server job_id
            local pending_info = self.pending_jobs[pending_job_id]
            if pending_info then
                pending_info.server_job_id = server_job_id
            end

            self.spinner_manager:stop(pending_job_id)
            self.pending_jobs[pending_job_id] = nil

            local bufnr = vim.uri_to_bufnr(uri)
            if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
                self.spinner_manager:start(server_job_id, bufnr, line)
            end
        end
    end

    -- Update line position if it changed (due to other job completing)
    self.spinner_manager:update_job_line(server_job_id, line)

    -- Update preview text
    if params.preview then
        self.spinner_manager:set_preview(server_job_id, params.preview)
    end
end

function AgentAmp:implement_function()
    local bufnr = vim.api.nvim_get_current_buf()
    local pos = vim.api.nvim_win_get_cursor(0)
    local line = pos[1] - 1

    local client_id = self.lsp_client:ensure_client(bufnr)
    if not client_id then
        return
    end

    self.lsp_client:request_code_actions(bufnr, function(actions)
        if not actions or #actions == 0 then
            vim.notify("[AgentAmp] No code actions available at cursor", vim.log.levels.INFO)
            return
        end

        local amp_action = nil
        for _, action in ipairs(actions) do
            local cmd = action.command
            if not cmd and action.edit == nil then
                cmd = action
            end
            if cmd and cmd.command == "agent.implFunction" then
                amp_action = cmd
                break
            end
        end

        if not amp_action then
            vim.notify("[AgentAmp] No 'Implement function with Amp' action found", vim.log.levels.INFO)
            return
        end

        local job_id = self:_generate_pending_job_id()
        self.pending_jobs[job_id] = {
            bufnr = bufnr,
            line = line,
        }

        -- Add pending job ID as 6th argument so server can correlate responses
        amp_action.arguments = amp_action.arguments or {}
        table.insert(amp_action.arguments, job_id)

        self.spinner_manager:start(job_id, bufnr, line)
        self.lsp_client:execute_command(bufnr, amp_action)
    end)
end

function AgentAmp:_generate_pending_job_id()
    return string.format("pending-%d-%d", vim.loop.now(), math.random(1000000))
end

local M = {}

function M.setup(opts)
    opts = opts or {}
    instance = AgentAmp.new(opts)

    vim.api.nvim_create_user_command("AmpImplementFunction", function()
        M.implement_function()
    end, { desc = "Implement function with Amp AI" })

    local augroup = vim.api.nvim_create_augroup("AgentAmp", { clear = true })

    vim.schedule(function()
        local bufnr = vim.api.nvim_get_current_buf()
        if vim.api.nvim_buf_is_valid(bufnr) and vim.bo[bufnr].buftype == "" then
            instance.lsp_client:start(bufnr)
        else
            instance.lsp_client:start()
        end
    end)

    vim.api.nvim_create_autocmd("BufEnter", {
        group = augroup,
        callback = function(args)
            if vim.bo[args.buf].buftype == "" then
                instance.lsp_client:attach_buffer(args.buf)
            end
        end,
        desc = "Attach AgentAmp LSP to buffer",
    })
end

function M.implement_function()
    if not instance then
        vim.notify("[AgentAmp] Plugin not initialized. Call require('agent_amp').setup() first", vim.log.levels.ERROR)
        return
    end
    instance:implement_function()
end

function M.get_instance()
    return instance
end

return M
