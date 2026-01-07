-- Unit tests for spinner.lua
-- Run with: nvim --headless -u NONE -c "lua dofile('nvim-plugin/tests/spinner_spec.lua')" -c "qa!"

-- Mock vim API for headless testing
if not vim then
    vim = {
        api = {
            nvim_create_namespace = function(_) return 1 end,
            nvim_buf_is_valid = function(_) return true end,
            nvim_buf_line_count = function(_) return 100 end,
            nvim_buf_set_extmark = function(_, _, _, _, _) return 1 end,
            nvim_buf_del_extmark = function(_, _, _) return true end,
        },
        loop = {
            new_timer = function()
                return {
                    start = function(_, _, _, _) end,
                    stop = function(_) end,
                    close = function(_) end,
                }
            end,
        },
        schedule_wrap = function(fn) return fn end,
        notify = function(_, _) end,
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
    }
end

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

local function assert_nil(value, msg)
    if value ~= nil then
        error(msg or "Expected nil, got " .. tostring(value))
    end
end

local function assert_not_nil(value, msg)
    if value == nil then
        error(msg or "Expected non-nil value")
    end
end

-- Load the module
package.path = package.path .. ";nvim-plugin/lua/?.lua;nvim-plugin/lua/?/init.lua"
local SpinnerManager = require("agent_amp.spinner")

local tests_passed = 0
local tests_failed = 0

local function run_test(name, test_fn)
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

-- Tests

run_test("SpinnerManager.new creates empty manager", function()
    local manager = SpinnerManager.new()
    assert_not_nil(manager)
    assert_not_nil(manager.spinners)
    assert_not_nil(manager.ns_id)
end)

run_test("SpinnerManager:start creates a spinner", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    assert_true(manager:is_running("job-1"))
end)

run_test("SpinnerManager:stop removes a spinner", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    manager:stop("job-1")
    assert_false(manager:is_running("job-1"))
end)

run_test("SpinnerManager:stop_all removes all spinners", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    manager:start("job-2", 1, 20)
    manager:start("job-3", 2, 5)
    manager:stop_all()
    assert_false(manager:has_running())
end)

run_test("SpinnerManager:has_running returns true when spinners exist", function()
    local manager = SpinnerManager.new()
    assert_false(manager:has_running())
    manager:start("job-1", 1, 10)
    assert_true(manager:has_running())
end)

run_test("SpinnerManager:update_job_line updates spinner line", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    
    -- Verify initial line
    local initial_line = manager:get_job_line("job-1")
    assert_equals(10, initial_line, "Initial line should be 10")
    
    -- Update line
    manager:update_job_line("job-1", 15)
    
    -- Verify updated line
    local updated_line = manager:get_job_line("job-1")
    assert_equals(15, updated_line, "Updated line should be 15")
end)

run_test("SpinnerManager:update_job_line does nothing for non-existent job", function()
    local manager = SpinnerManager.new()
    -- Should not error
    manager:update_job_line("non-existent", 10)
end)

run_test("SpinnerManager:update_job_line does nothing when line unchanged", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    
    -- Update to same line
    manager:update_job_line("job-1", 10)
    
    -- Should still be running
    assert_true(manager:is_running("job-1"))
    assert_equals(10, manager:get_job_line("job-1"))
end)

run_test("SpinnerManager:get_job_line returns nil for non-existent job", function()
    local manager = SpinnerManager.new()
    local line = manager:get_job_line("non-existent")
    assert_nil(line)
end)

run_test("SpinnerManager:find_job_by_uri_line finds correct job", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    manager:start("job-2", 1, 20)
    manager:start("job-3", 2, 10)  -- Different buffer
    
    local job_id = manager:find_job_by_uri_line("file:///test/file_1.rs", 10)
    assert_equals("job-1", job_id)
    
    job_id = manager:find_job_by_uri_line("file:///test/file_1.rs", 20)
    assert_equals("job-2", job_id)
    
    job_id = manager:find_job_by_uri_line("file:///test/file_2.rs", 10)
    assert_equals("job-3", job_id)
end)

run_test("SpinnerManager:find_job_by_uri_line returns nil when no match", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    
    local job_id = manager:find_job_by_uri_line("file:///test/file_1.rs", 99)
    assert_nil(job_id)
end)

run_test("SpinnerManager:find_any_job_by_uri finds first job for URI", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    manager:start("job-2", 1, 20)
    manager:start("job-3", 2, 10)
    
    local job_id = manager:find_any_job_by_uri("file:///test/file_1.rs")
    assert_true(job_id == "job-1" or job_id == "job-2", "Should find a job for buffer 1")
    
    job_id = manager:find_any_job_by_uri("file:///test/file_2.rs")
    assert_equals("job-3", job_id)
end)

run_test("Multiple concurrent spinners can exist", function()
    local manager = SpinnerManager.new()
    
    -- Start 5 concurrent jobs in same buffer
    for i = 1, 5 do
        manager:start("job-" .. i, 1, i * 10)
    end
    
    -- All should be running
    for i = 1, 5 do
        assert_true(manager:is_running("job-" .. i), "job-" .. i .. " should be running")
    end
    
    -- Stop one, others should continue
    manager:stop("job-3")
    assert_false(manager:is_running("job-3"))
    assert_true(manager:is_running("job-1"))
    assert_true(manager:is_running("job-5"))
end)

run_test("SpinnerManager:set_preview updates spinner preview", function()
    local manager = SpinnerManager.new()
    manager:start("job-1", 1, 10)
    
    -- Should not error
    manager:set_preview("job-1", "fn foo() {\n    println!(\"hello\");\n}")
end)

run_test("Line tracking across multiple updates", function()
    local manager = SpinnerManager.new()
    
    -- Simulate concurrent jobs
    manager:start("job-a", 1, 5)   -- Function at line 5
    manager:start("job-b", 1, 15)  -- Function at line 15
    manager:start("job-c", 1, 25)  -- Function at line 25
    
    -- Job A completes and adds 3 lines
    -- Server should send line updates to B and C
    manager:update_job_line("job-b", 18)  -- 15 + 3
    manager:update_job_line("job-c", 28)  -- 25 + 3
    
    assert_equals(5, manager:get_job_line("job-a"))
    assert_equals(18, manager:get_job_line("job-b"))
    assert_equals(28, manager:get_job_line("job-c"))
    
    -- Job B completes and removes 2 lines
    manager:update_job_line("job-c", 26)  -- 28 - 2
    
    assert_equals(26, manager:get_job_line("job-c"))
end)

-- Summary
print("")
print(string.format("Tests: %d passed, %d failed", tests_passed, tests_failed))

if tests_failed > 0 then
    os.exit(1)
end
