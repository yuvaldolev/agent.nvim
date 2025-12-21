local Spinner = require("agent_amp.spinner")
local LspClient = require("agent_amp.lsp")

local AgentAmp = {}
AgentAmp.__index = AgentAmp

local instance = nil

function AgentAmp.new(opts)
    local self = setmetatable({}, AgentAmp)
    self.opts = opts or {}
    self.spinner = Spinner.new()
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

function AgentAmp:_on_apply_edit(_err, _result, _ctx)
    if self.spinner:is_running() then
        self.spinner:stop()
        vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
    end
end

function AgentAmp:_on_progress(params)
    if self.spinner:is_running() and params and params.preview then
        self.spinner:set_preview(params.preview)
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

        self.spinner:start(bufnr, line)
        self.lsp_client:execute_command(bufnr, amp_action)
    end)
end

local M = {}

function M.setup(opts)
    opts = opts or {}
    instance = AgentAmp.new(opts)

    vim.api.nvim_create_user_command("AmpImplementFunction", function()
        M.implement_function()
    end, { desc = "Implement function with Amp AI" })

    -- Create augroup for AgentAmp autocmds
    local augroup = vim.api.nvim_create_augroup("AgentAmp", { clear = true })

    -- Start the LSP and attach to current buffer (deferred to allow Neovim to fully initialize)
    vim.schedule(function()
        local bufnr = vim.api.nvim_get_current_buf()
        if vim.api.nvim_buf_is_valid(bufnr) and vim.bo[bufnr].buftype == "" then
            instance.lsp_client:start(bufnr)
        else
            -- Start without a buffer if current buffer is not a normal file buffer
            instance.lsp_client:start()
        end
    end)

    -- Attach LSP to newly opened buffers
    vim.api.nvim_create_autocmd("BufEnter", {
        group = augroup,
        callback = function(args)
            -- Only attach to normal file buffers (not special buffers like terminals, quickfix, etc.)
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
