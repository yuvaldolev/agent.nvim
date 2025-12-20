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
    })
    return self
end

function AgentAmp:_on_apply_edit(_err, _result, _ctx)
    if self.spinner:is_running() then
        self.spinner:stop()
        vim.notify("[AgentAmp] Implementation applied", vim.log.levels.INFO)
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
