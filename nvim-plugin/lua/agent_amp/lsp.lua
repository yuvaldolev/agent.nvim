local LspClient = {}
LspClient.__index = LspClient

local DEFAULT_BACKEND_NAME = "Agent"

local function get_plugin_root()
    local source = debug.getinfo(1, "S").source:sub(2)
    local plugin_lua_dir = vim.fn.fnamemodify(source, ":h:h:h")
    return vim.fn.fnamemodify(plugin_lua_dir, ":h")
end

local function find_binary()
    local in_path = vim.fn.exepath("agent-lsp")
    if in_path ~= "" then
        return in_path
    end

    local project_root = get_plugin_root()
    local release_bin = project_root .. "/target/release/agent-lsp"
    if vim.fn.executable(release_bin) == 1 then
        return release_bin
    end

    local debug_bin = project_root .. "/target/debug/agent-lsp"
    if vim.fn.executable(debug_bin) == 1 then
        return debug_bin
    end

    return nil
end

function LspClient.new(opts)
    local self = setmetatable({}, LspClient)
    self.user_cmd = opts.cmd
    self.client_id = nil
    self.on_apply_edit = opts.on_apply_edit
    self.on_progress = opts.on_progress
    self.on_job_completed = opts.on_job_completed
    self.on_backend_info = opts.on_backend_info
    self.get_backend_name = opts.get_backend_name
    return self
end

function LspClient:_get_backend_name()
    if self.get_backend_name then
        return self.get_backend_name()
    end
    return DEFAULT_BACKEND_NAME
end

function LspClient:_resolve_cmd()
    if self.user_cmd then
        return self.user_cmd
    end

    local binary = find_binary()
    if binary then
        return { binary }
    end

    return nil
end

function LspClient:_create_client_config()
    local cmd = self:_resolve_cmd()
    if not cmd then
        return nil
    end

    local original_handler = vim.lsp.handlers["workspace/applyEdit"]

    return {
        name = "agent-lsp",
        cmd = cmd,
        root_dir = vim.fn.getcwd(),
        handlers = {
            ["workspace/applyEdit"] = function(err, result, ctx, config)
                if self.on_apply_edit then
                    self.on_apply_edit(err, result, ctx)
                end
                if original_handler then
                    return original_handler(err, result, ctx, config)
                end
                return { applied = true }
            end,
            ["agent/implFunctionProgress"] = function(_err, params, _ctx)
                if self.on_progress then
                    self.on_progress(params)
                end
            end,
            ["agent/jobCompleted"] = function(_err, params, _ctx)
                if self.on_job_completed then
                    self.on_job_completed(params)
                end
            end,
            ["agent/backendInfo"] = function(_err, params, _ctx)
                if self.on_backend_info then
                    self.on_backend_info(params)
                end
            end,
        },
    }
end

function LspClient:start(bufnr)
    if self.client_id and vim.lsp.get_client_by_id(self.client_id) then
        return self.client_id
    end

    local config = self:_create_client_config()
    if not config then
        vim.notify("[" .. self:_get_backend_name() .. "] LSP binary not found. Build with 'cargo build --release' or specify cmd in setup()", vim.log.levels.WARN)
        return nil
    end

    local start_opts = {
        reuse_client = function(client, cfg)
            return client.name == cfg.name
        end,
    }

    if bufnr then
        start_opts.bufnr = bufnr
    end

    local client_id = vim.lsp.start(config, start_opts)

    if not client_id then
        vim.notify("[" .. self:_get_backend_name() .. "] Failed to start LSP client", vim.log.levels.ERROR)
        return nil
    end

    self.client_id = client_id
    return client_id
end

function LspClient:attach_buffer(bufnr)
    if not self.client_id then
        return self:start(bufnr)
    end

    local client = vim.lsp.get_client_by_id(self.client_id)
    if not client then
        self.client_id = nil
        return self:start(bufnr)
    end

    if not vim.lsp.buf_is_attached(bufnr, self.client_id) then
        vim.lsp.buf_attach_client(bufnr, self.client_id)
    end

    return self.client_id
end

function LspClient:ensure_client(bufnr)
    if self.client_id and vim.lsp.get_client_by_id(self.client_id) then
        if not vim.lsp.buf_is_attached(bufnr, self.client_id) then
            vim.lsp.buf_attach_client(bufnr, self.client_id)
        end
        return self.client_id
    end

    return self:start(bufnr)
end

function LspClient:request_code_actions(bufnr, callback)
    local client_id = self:ensure_client(bufnr)
    if not client_id then
        callback(nil)
        return
    end

    local pos = vim.api.nvim_win_get_cursor(0)
    local line = pos[1] - 1
    local character = pos[2]

    local params = {
        textDocument = vim.lsp.util.make_text_document_params(bufnr),
        range = {
            start = { line = line, character = character },
            ["end"] = { line = line, character = character },
        },
        context = {
            diagnostics = {},
            only = { "quickfix" },
        },
    }

    vim.lsp.buf_request(bufnr, "textDocument/codeAction", params, function(err, result)
        if err then
            vim.notify("[" .. self:_get_backend_name() .. "] Code action request failed: " .. vim.inspect(err), vim.log.levels.ERROR)
            callback(nil)
            return
        end
        callback(result)
    end)
end

function LspClient:execute_command(bufnr, command)
    local client_id = self:ensure_client(bufnr)
    if not client_id then
        return
    end

    local client = vim.lsp.get_client_by_id(client_id)
    if not client then
        vim.notify("[" .. self:_get_backend_name() .. "] LSP client not found", vim.log.levels.ERROR)
        return
    end

    client.request("workspace/executeCommand", command, function(err, _result)
        if err then
            vim.notify("[" .. self:_get_backend_name() .. "] Execute command failed: " .. vim.inspect(err), vim.log.levels.ERROR)
        end
    end, bufnr)
end

function LspClient:get_client_id()
    return self.client_id
end

return LspClient
