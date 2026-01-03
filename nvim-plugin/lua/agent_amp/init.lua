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
    })
    return self
end

function AgentAmp:_on_apply_edit(_err, result, _ctx)
    if not result or not result.edit then
        return
    end

    local edit = result.edit
    local document_changes = edit.documentChanges or {}

    for _, change in ipairs(document_changes) do
        if change.textDocument and change.textDocument.uri then
            local uri = change.textDocument.uri
            for _, text_edit in ipairs(change.edits or {}) do
                local line = text_edit.range and text_edit.range.start and text_edit.range.start.line
                if line then
                    -- First try exact line match
                    local job_id = self.spinner_manager:find_job_by_uri_line(uri, line)
                    
                    -- If no match and this is a full-file edit (starts at line 0), find any job for this URI
                    if not job_id and line == 0 then
                        job_id = self.spinner_manager:find_any_job_by_uri(uri)
                    end
                    
                    if job_id then
                        self.spinner_manager:stop(job_id)
                        vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
                        return
                    end
                end
            end
        end
    end

    local changes = edit.changes or {}
    for uri, edits in pairs(changes) do
        for _, text_edit in ipairs(edits) do
            local line = text_edit.range and text_edit.range.start and text_edit.range.start.line
            if line then
                -- First try exact line match
                local job_id = self.spinner_manager:find_job_by_uri_line(uri, line)
                
                -- If no match and this is a full-file edit (starts at line 0), find any job for this URI
                if not job_id and line == 0 then
                    job_id = self.spinner_manager:find_any_job_by_uri(uri)
                end
                
                if job_id then
                    self.spinner_manager:stop(job_id)
                    vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
                    return
                end
            end
        end
    end

    if self.spinner_manager:has_running() then
        vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
    end
end

function AgentAmp:_on_progress(params)
    if not params or not params.job_id then
        return
    end

    local server_job_id = params.job_id
    local uri = params.uri
    local line = params.line

    if not self.spinner_manager:is_running(server_job_id) then
        local pending_job_id = self.spinner_manager:find_job_by_uri_line(uri, line)
        if pending_job_id and pending_job_id:match("^pending%-") then
            self.spinner_manager:stop(pending_job_id)
            self.pending_jobs[pending_job_id] = nil

            local bufnr = vim.uri_to_bufnr(uri)
            if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
                self.spinner_manager:start(server_job_id, bufnr, line)
            end
        end
    end

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
            if cmd and cmd.command == "amp.implFunction" then
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
