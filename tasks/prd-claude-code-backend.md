# PRD: Claude Code Backend

## Introduction

Add a Claude Code backend to agent.nvim that uses the `claude` CLI to implement functions. This provides an alternative AI backend alongside the existing Amp and OpenCode backends, enabling users who prefer Claude Code to use it for function implementation within their editor.

## Goals

- Implement a new `ClaudeCodeClient` that conforms to the existing `Backend` trait
- Support function implementation via the `claude` CLI with plain text streaming
- Achieve feature parity with existing Amp and OpenCode backends
- Integrate seamlessly with the existing backend selection mechanism in config.rs

## User Stories

### US-001: Add ClaudeCode variant to BackendType enum
**Description:** As a developer, I need a new backend type variant so that users can select Claude Code as their backend.

**Acceptance Criteria:**
- [ ] Add `ClaudeCode` variant to `BackendType` enum in `src/config.rs`
- [ ] Typecheck passes (`cargo check`)

### US-002: Create ClaudeCodeClient struct
**Description:** As a developer, I need a client struct that encapsulates Claude Code CLI interactions.

**Acceptance Criteria:**
- [ ] Create `src/claude_code.rs` module
- [ ] Implement `ClaudeCodeClient` struct with `new()` constructor
- [ ] Implement `Default` trait for `ClaudeCodeClient`
- [ ] Add module to `src/main.rs` module declarations
- [ ] Typecheck passes (`cargo check`)

### US-003: Implement Backend trait for ClaudeCodeClient
**Description:** As a developer, I need the client to implement the Backend trait so it can be used interchangeably with other backends.

**Acceptance Criteria:**
- [ ] Implement `implement_function()` method (can be basic/deprecated version)
- [ ] Implement `implement_function_streaming()` method with plain text stdout streaming
- [ ] Progress callback receives accumulated text from stdout
- [ ] Typecheck passes (`cargo check`)

### US-004: Build prompt for Claude Code
**Description:** As a developer, I need a prompt builder that instructs Claude Code to implement the target function.

**Acceptance Criteria:**
- [ ] Create `build_prompt()` function following existing pattern from amp.rs/opencode.rs
- [ ] Prompt includes line number, character position, function signature, and file contents
- [ ] Prompt instructs Claude Code to write output to the specified temp file path
- [ ] Prompt explicitly tells Claude Code NOT to output code to stdout
- [ ] Typecheck passes (`cargo check`)

### US-005: Invoke claude CLI with correct arguments
**Description:** As a developer, I need the client to spawn the claude CLI process correctly.

**Acceptance Criteria:**
- [ ] Use `std::process::Command` to spawn `claude` CLI
- [ ] Pass prompt via `--print` flag (or appropriate flag for non-interactive mode)
- [ ] Capture stdout for streaming progress updates
- [ ] Capture stderr for error reporting
- [ ] Handle non-zero exit codes as errors
- [ ] Typecheck passes (`cargo check`)

### US-006: Wire ClaudeCode backend into factory function
**Description:** As a developer, I need the backend factory to support creating Claude Code clients.

**Acceptance Criteria:**
- [ ] Update `create_backend()` in `src/backend.rs` to handle `BackendType::ClaudeCode`
- [ ] Import `ClaudeCodeClient` in backend.rs
- [ ] Typecheck passes (`cargo check`)

### US-007: Add unit tests for ClaudeCodeClient
**Description:** As a developer, I need tests to verify the client's behavior.

**Acceptance Criteria:**
- [ ] Add test for `build_prompt()` output format
- [ ] Add test verifying prompt contains required elements (line, character, function signature, output path)
- [ ] All tests pass (`cargo test`)

### US-008: Add integration test (ignored by default)
**Description:** As a developer, I need an integration test that actually calls the claude CLI for manual verification.

**Acceptance Criteria:**
- [ ] Add `#[ignore]` test that invokes claude CLI
- [ ] Test verifies basic end-to-end flow
- [ ] Test is documented in CLAUDE.md under "Ignored Tests" section
- [ ] Test passes when run with `cargo test -- --ignored` (requires claude CLI installed)

## Functional Requirements

- FR-1: Add `ClaudeCode` variant to `BackendType` enum
- FR-2: Create `ClaudeCodeClient` struct implementing the `Backend` trait
- FR-3: Implement `build_prompt()` function that generates implementation instructions
- FR-4: Spawn `claude` CLI process with appropriate arguments for non-interactive execution
- FR-5: Stream stdout line-by-line, calling progress callback with accumulated text
- FR-6: Capture and report stderr content on CLI failure
- FR-7: Return error if CLI exits with non-zero status
- FR-8: Update `create_backend()` factory to support `BackendType::ClaudeCode`

## Non-Goals

- No MCP tool integration beyond file writing
- No session persistence or conversation continuity
- No custom authentication handling (defer to claude CLI's default mechanism)
- No JSON streaming format support (use plain text only)
- No special model selection (use claude CLI defaults)

## Technical Considerations

### Claude CLI Interface

The `claude` CLI should be invoked in a non-interactive mode. Based on typical CLI patterns:

```bash
claude --print "prompt here"
# or
claude -p "prompt here"
```

The exact flag should be verified against Claude Code documentation. The CLI should:
- Accept the prompt as an argument
- Output responses to stdout (plain text)
- Use stderr for errors
- Exit with 0 on success, non-zero on failure

### Module Structure

```
src/
├── claude_code.rs  # New file: ClaudeCodeClient implementation
├── backend.rs      # Update: add ClaudeCode to factory
├── config.rs       # Update: add ClaudeCode variant
└── main.rs         # Update: add mod claude_code
```

### Prompt Format

Follow the existing pattern from opencode.rs:

```
Implement the function body at line {line}, character {character} in the following file.
The function to implement is: `{function_signature}`

IMPORTANT: Implement ONLY the function `{function_signature}` - do NOT implement any other functions in the file.

Write ONLY this function's implementation (signature and body) to the file: {output_path}
Do NOT include any other code from the source file (no imports, no other functions).
Do NOT output the code to stdout.
Output only status messages or confirmation.

<FILE-CONTENT>
{file_contents}
</FILE-CONTENT>
```

### Error Handling

- Capture stderr on failure for meaningful error messages
- Check exit status before returning success
- Include stderr content in error message if available

## Success Metrics

- Claude Code backend compiles without errors
- Existing tests continue to pass
- Backend can be selected via `CURRENT_BACKEND` config
- Manual testing with `claude` CLI successfully implements a function

## Open Questions

- What is the exact CLI flag for non-interactive/print mode in claude CLI? (verify with `claude --help`)
- Does claude CLI support any streaming indicators we should watch for?
- Should we add a model selection config option for future enhancement?
