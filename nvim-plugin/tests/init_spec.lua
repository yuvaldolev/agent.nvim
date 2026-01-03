-- Unit tests for init.lua (AgentAmp core logic)
-- Run with: lua nvim-plugin/tests/init_spec.lua

-- Mock vim API for headless testing
local notifications = {}
local created_spinners = {}
local stopped_spinners = {}
local updated_lines = {}

vim = {
    api = {
        nvim_create_namespace = function(_) return 1 end,
        nvim_buf_is_valid = function(_) return true end,
        nvim_buf_line_count = function(_) return 100 end,
        nvim_buf_set_extmark = function(_, _, _, _, _) return 1 end,
        nvim_buf_del_extmark = function(_, _, _) return true end,
        nvim_get_current_buf = function() return 1 end,
        nvim_win_get_cursor = function(_) return {10, 0} end,
        nvim_create_user_command = function(_, _, _) end,
        nvim_create_augroup = function(_, _) return 1 end,
        nvim_create_autocmd = function(_, _) end,
    },
    bo = setmetatable({}, {
        __index = function(_, _)
            return { buftype = "" }
        end
    }),
    fn = {
        getcwd = function() return "/test" end,
        exepath = function(_) return "" end,
        executable = function(_) return 0 end,
        fnamemodify = function(path, _) return path end,
    },
    loop = {
        new_timer = function()
            return {
                start = function(_, _, _, _) end,
                stop = function(_) end,
                close = function(_) end,
            }
        end,
        now = function() return 12345 end,
    },
    lsp = {
        handlers = {},
        start = function(_) return 1 end,
        get_client_by_id = function(_) return { request = function() end } end,
        buf_is_attached = function(_, _) return true end,
        buf_attach_client = function(_, _) end,
        util = {
            make_text_document_params = function(_) return {} end,
        },
        buf_request = function(_, _, _, _) end,
    },
    schedule = function(fn) fn() end,
    schedule_wrap = function(fn) return fn end,
    notify = function(msg, level) 
        table.insert(notifications, {msg = msg, level = level})
    end,
    split = function(str, sep, opts)
        local result = {}
        for match in (str .. sep):gmatch("(.-)" .. sep) do
            table.insert(result, match)
        end
        return result
    end,
    uri_from_bufnr = function(bufnr)
        return "file:///test/file_" .. bufnr .. ".rs"
    end,
    uri_to_bufnr = function(uri)
        local num = uri:match("file_%d+")
        if num then
            return tonumber(num:sub(6))
        end
        return 1
    end,
    log = {
        levels = {
            DEBUG = 0,
            INFO = 1,
            WARN = 2,
            ERROR = 3,
        },
    },
}

-- Test utilities
local function assert_equals(expected, actual, msg)
    if expected ~= actual then
        error(string.format("%s: expected %s, got %s", msg or "Assertion failed", tostring(expected), tostring(actual)))
    end
end

local function assert_true(value, msg)
    if not value then
        error(msg or "Expected true, got false")
    end
end

local function assert_false(value, msg)
    if value then
        error(msg or "Expected false, got true")
    end
end

local function assert_contains(str, substr, msg)
    if not str:find(substr, 1, true) then
        error(string.format("%s: '%s' not found in '%s'", msg or "String not found", substr, str))
    end
end

local function clear_state()
    notifications = {}
    created_spinners = {}
    stopped_spinners = {}
    updated_lines = {}
end

-- Load the modules
package.path = package.path .. ";nvim-plugin/lua/?.lua;nvim-plugin/lua/?/init.lua"

-- We need to reset and reload modules for each test
local function create_fresh_instance()
    package.loaded["agent_amp.spinner"] = nil
    package.loaded["agent_amp.lsp"] = nil
    package.loaded["agent_amp"] = nil
    
    local SpinnerManager = require("agent_amp.spinner")
    
    -- Create a test-friendly AgentAmp instance
    local AgentAmp = {}
    AgentAmp.__index = AgentAmp
    
    function AgentAmp.new()
        local self = setmetatable({}, AgentAmp)
        self.spinner_manager = SpinnerManager.new()
        self.pending_jobs = {}
        return self
    end
    
    -- Copy the actual methods from init.lua
    function AgentAmp:_on_job_completed(params)
        if not params or not params.job_id then
            return
        end

        local job_id = params.job_id
        self.spinner_manager:stop(job_id)

        for pending_id, pending_info in pairs(self.pending_jobs) do
            if pending_info.server_job_id == job_id then
                self.pending_jobs[pending_id] = nil
                break
            end
        end

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

        if not self.spinner_manager:is_running(server_job_id) then
            local pending_job_id = self.spinner_manager:find_job_by_uri_line(uri, line)
            
            if not pending_job_id then
                for pid, pinfo in pairs(self.pending_jobs) do
                    if pid:match("^pending%-") then
                        local bufnr = pinfo.bufnr
                        if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
                            local buf_uri = vim.uri_from_bufnr(bufnr)
                            if buf_uri == uri then
                                pending_job_id = pid
                                break
                            end
                        end
                    end
                end
            end
            
            if pending_job_id and pending_job_id:match("^pending%-") then
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

        self.spinner_manager:update_job_line(server_job_id, line)

        if params.preview then
            self.spinner_manager:set_preview(server_job_id, params.preview)
        end
    end
    
    return AgentAmp.new()
end

local tests_passed = 0
local tests_failed = 0

local function run_test(name, test_fn)
    clear_state()
    local ok, err = pcall(test_fn)
    if ok then
        print("PASS: " .. name)
        tests_passed = tests_passed + 1
    else
        print("FAIL: " .. name)
        print("  Error: " .. tostring(err))
        tests_failed = tests_failed + 1
    end
end

-- Tests for _on_job_completed

run_test("_on_job_completed ignores nil params", function()
    local agent = create_fresh_instance()
    agent:_on_job_completed(nil)
    assert_equals(0, #notifications, "Should not notify on nil params")
end)

run_test("_on_job_completed ignores params without job_id", function()
    local agent = create_fresh_instance()
    agent:_on_job_completed({ success = true })
    assert_equals(0, #notifications, "Should not notify without job_id")
end)

run_test("_on_job_completed stops spinner and shows success notification", function()
    local agent = create_fresh_instance()
    
    -- Start a spinner
    agent.spinner_manager:start("job-123", 1, 10)
    assert_true(agent.spinner_manager:is_running("job-123"))
    
    -- Complete the job
    agent:_on_job_completed({ job_id = "job-123", success = true })
    
    -- Spinner should be stopped
    assert_false(agent.spinner_manager:is_running("job-123"))
    
    -- Should have success notification
    assert_equals(1, #notifications)
    assert_contains(notifications[1].msg, "Implementation applied")
    assert_equals(vim.log.levels.INFO, notifications[1].level)
end)

run_test("_on_job_completed shows error notification on failure", function()
    local agent = create_fresh_instance()
    
    agent.spinner_manager:start("job-456", 1, 10)
    agent:_on_job_completed({ 
        job_id = "job-456", 
        success = false, 
        error = "Backend timeout" 
    })
    
    assert_false(agent.spinner_manager:is_running("job-456"))
    assert_equals(1, #notifications)
    assert_contains(notifications[1].msg, "Backend timeout")
    assert_equals(vim.log.levels.ERROR, notifications[1].level)
end)

run_test("_on_job_completed shows default error message when none provided", function()
    local agent = create_fresh_instance()
    
    agent.spinner_manager:start("job-789", 1, 10)
    agent:_on_job_completed({ job_id = "job-789", success = false })
    
    assert_equals(1, #notifications)
    assert_contains(notifications[1].msg, "Implementation failed")
end)

run_test("_on_job_completed cleans up pending job mapping", function()
    local agent = create_fresh_instance()
    
    -- Simulate a pending job that was transitioned
    agent.pending_jobs["pending-1"] = {
        bufnr = 1,
        line = 10,
        server_job_id = "server-job-1"
    }
    agent.spinner_manager:start("server-job-1", 1, 10)
    
    agent:_on_job_completed({ job_id = "server-job-1", success = true })
    
    -- Pending job should be cleaned up
    assert_equals(nil, agent.pending_jobs["pending-1"])
end)

-- Tests for _on_progress

run_test("_on_progress ignores nil params", function()
    local agent = create_fresh_instance()
    agent:_on_progress(nil)
    -- Should not error
end)

run_test("_on_progress ignores params without job_id", function()
    local agent = create_fresh_instance()
    agent:_on_progress({ uri = "file:///test.rs", line = 10 })
    -- Should not error
end)

run_test("_on_progress transitions pending job to server job", function()
    local agent = create_fresh_instance()
    
    -- Simulate pending job
    local pending_id = "pending-12345-999"
    agent.pending_jobs[pending_id] = { bufnr = 1, line = 10 }
    agent.spinner_manager:start(pending_id, 1, 10)
    
    -- Receive progress from server
    agent:_on_progress({
        job_id = "server-uuid-123",
        uri = "file:///test/file_1.rs",
        line = 10,
        preview = "fn foo() {"
    })
    
    -- Pending spinner should be stopped
    assert_false(agent.spinner_manager:is_running(pending_id))
    
    -- Server job spinner should be running
    assert_true(agent.spinner_manager:is_running("server-uuid-123"))
    
    -- Pending job should be cleaned up
    assert_equals(nil, agent.pending_jobs[pending_id])
end)

run_test("_on_progress updates line for existing job", function()
    local agent = create_fresh_instance()
    
    -- Start a server job spinner
    agent.spinner_manager:start("job-1", 1, 10)
    
    -- Receive progress with updated line (due to other job completing)
    agent:_on_progress({
        job_id = "job-1",
        uri = "file:///test/file_1.rs",
        line = 15,  -- Line shifted
        preview = "still working..."
    })
    
    -- Line should be updated
    assert_equals(15, agent.spinner_manager:get_job_line("job-1"))
end)

run_test("_on_progress finds pending job by URI when line doesn't match", function()
    local agent = create_fresh_instance()
    
    -- Simulate pending job at line 10
    local pending_id = "pending-99-88"
    agent.pending_jobs[pending_id] = { bufnr = 1, line = 10 }
    agent.spinner_manager:start(pending_id, 1, 10)
    
    -- Server sends progress with adjusted line (maybe other job shifted it)
    agent:_on_progress({
        job_id = "server-job-x",
        uri = "file:///test/file_1.rs",
        line = 13,  -- Different from original 10
        preview = "..."
    })
    
    -- Should have transitioned to server job
    assert_true(agent.spinner_manager:is_running("server-job-x"))
    assert_false(agent.spinner_manager:is_running(pending_id))
end)

run_test("Multiple concurrent jobs receive independent progress", function()
    local agent = create_fresh_instance()
    
    -- Start multiple server job spinners
    agent.spinner_manager:start("job-a", 1, 5)
    agent.spinner_manager:start("job-b", 1, 15)
    agent.spinner_manager:start("job-c", 1, 25)
    
    -- Job A completes, server sends line updates for B and C
    agent:_on_progress({
        job_id = "job-b",
        uri = "file:///test/file_1.rs",
        line = 18,  -- Shifted by 3
    })
    
    agent:_on_progress({
        job_id = "job-c",
        uri = "file:///test/file_1.rs",
        line = 28,  -- Shifted by 3
    })
    
    -- Verify lines updated
    assert_equals(5, agent.spinner_manager:get_job_line("job-a"))   -- Unchanged
    assert_equals(18, agent.spinner_manager:get_job_line("job-b"))  -- Updated
    assert_equals(28, agent.spinner_manager:get_job_line("job-c"))  -- Updated
end)

run_test("Full concurrent workflow simulation", function()
    local agent = create_fresh_instance()
    
    -- User triggers 3 implementations
    local pending1 = "pending-1-1"
    local pending2 = "pending-2-2"
    local pending3 = "pending-3-3"
    
    agent.pending_jobs[pending1] = { bufnr = 1, line = 5 }
    agent.pending_jobs[pending2] = { bufnr = 1, line = 15 }
    agent.pending_jobs[pending3] = { bufnr = 1, line = 25 }
    
    agent.spinner_manager:start(pending1, 1, 5)
    agent.spinner_manager:start(pending2, 1, 15)
    agent.spinner_manager:start(pending3, 1, 25)
    
    -- Server acknowledges all 3
    agent:_on_progress({ job_id = "srv-1", uri = "file:///test/file_1.rs", line = 5 })
    agent:_on_progress({ job_id = "srv-2", uri = "file:///test/file_1.rs", line = 15 })
    agent:_on_progress({ job_id = "srv-3", uri = "file:///test/file_1.rs", line = 25 })
    
    -- All should be running with server IDs
    assert_true(agent.spinner_manager:is_running("srv-1"))
    assert_true(agent.spinner_manager:is_running("srv-2"))
    assert_true(agent.spinner_manager:is_running("srv-3"))
    
    -- Job 1 completes (adds 3 lines)
    agent:_on_job_completed({ job_id = "srv-1", success = true })
    
    -- Server sends line updates to remaining jobs
    agent:_on_progress({ job_id = "srv-2", uri = "file:///test/file_1.rs", line = 18 })
    agent:_on_progress({ job_id = "srv-3", uri = "file:///test/file_1.rs", line = 28 })
    
    assert_false(agent.spinner_manager:is_running("srv-1"))
    assert_equals(18, agent.spinner_manager:get_job_line("srv-2"))
    assert_equals(28, agent.spinner_manager:get_job_line("srv-3"))
    
    -- Job 2 completes
    agent:_on_job_completed({ job_id = "srv-2", success = true })
    
    assert_false(agent.spinner_manager:is_running("srv-2"))
    assert_true(agent.spinner_manager:is_running("srv-3"))
    
    -- Job 3 completes
    agent:_on_job_completed({ job_id = "srv-3", success = true })
    
    assert_false(agent.spinner_manager:has_running())
    
    -- Should have 3 success notifications
    local success_count = 0
    for _, n in ipairs(notifications) do
        if n.msg:find("Implementation applied") then
            success_count = success_count + 1
        end
    end
    assert_equals(3, success_count, "Should have 3 success notifications")
end)

-- Summary
print("")
print(string.format("Tests: %d passed, %d failed", tests_passed, tests_failed))

if tests_failed > 0 then
    os.exit(1)
end
