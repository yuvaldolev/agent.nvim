# PRD: Visual Selection Implementation

## Introduction

Add the ability to implement AI-generated code for a visual selection within a file, complementing the existing function implementation feature. Users can select any code region (within or across functions) and request AI to generate a complete replacement. This provides more granular control over AI-assisted code generation, enabling use cases like implementing loop bodies, conditionals, refactoring selected code, or any arbitrary code block.

**Linear Ticket:** [YDO-20](https://linear.app/ydolev/issue/YDO-20/add-option-to-implement-a-visual-selection)

## Goals

- Provide a separate API/command for implementing visual selections (distinct from function implementation)
- Support any arbitrary selection without syntax validation (AI handles incomplete constructs)
- Generate complete replacement code for the selected region
- Maintain feature parity with function implementation (streaming progress, concurrent jobs, line tracking)
- Full-featured implementation with proper validation, error handling, and edge cases

## User Stories

### US-001: Add LSP command for selection implementation
**Description:** As a developer, I need an LSP command that accepts a selection range so the server can process visual selection requests.

**Acceptance Criteria:**
- [ ] New command `agent.implSelection` registered in LSP server capabilities
- [ ] Command accepts arguments: uri, start_line, start_character, end_line, end_character, version, language_id
- [ ] Command is distinct from existing `agent.implFunction` command
- [ ] Typecheck passes (`cargo check`)

### US-002: Generate code action for selection implementation
**Description:** As a user, I want to see an "Implement selection" code action when I have a visual selection so I can trigger AI implementation.

**Acceptance Criteria:**
- [ ] Code action "Implement selection with [Backend]" appears when range.start != range.end
- [ ] Code action "Implement function with [Backend]" appears when range.start == range.end (existing behavior)
- [ ] Code action includes full selection range in command arguments
- [ ] Typecheck passes (`cargo check`)
- [ ] Tests pass (`cargo test`)

### US-003: Extract selected text for AI prompt
**Description:** As a developer, I need the LSP server to extract the selected text from the document so it can be included in the AI prompt.

**Acceptance Criteria:**
- [ ] New utility function `extract_selection_text(content, start_line, start_char, end_line, end_char)` in utils.rs
- [ ] Correctly handles single-line selections
- [ ] Correctly handles multi-line selections
- [ ] Returns the exact text within the selection boundaries
- [ ] Unit tests for extraction logic
- [ ] Typecheck passes (`cargo check`)

### US-004: Update backend trait for selection implementation
**Description:** As a developer, I need the Backend trait to support selection-based implementation so all backends can handle visual selections.

**Acceptance Criteria:**
- [ ] New method `implement_selection_streaming()` added to Backend trait
- [ ] Method signature includes: file_path, selection_text, start_line, start_char, end_line, end_char, language_id, file_contents, output_path, progress callback
- [ ] All backend implementations (AmpClient, OpenCodeClient, ClaudeCodeClient) implement the new method
- [ ] Typecheck passes (`cargo check`)

### US-005: Create selection-specific prompts for backends
**Description:** As a developer, I need backend prompts tailored for selection replacement so the AI understands the task context.

**Acceptance Criteria:**
- [ ] Prompt includes the selected text with clear markers
- [ ] Prompt includes surrounding context (full file contents)
- [ ] Prompt specifies line numbers of the selection
- [ ] Prompt instructs AI to write complete replacement code to output file
- [ ] Prompt works for all backends (Amp, OpenCode, ClaudeCode)
- [ ] Typecheck passes (`cargo check`)

### US-006: Implement selection replacement logic
**Description:** As a developer, I need a function to replace a specific selection range in the document so the AI output can be applied correctly.

**Acceptance Criteria:**
- [ ] New function `replace_selection_in_document()` in utils.rs
- [ ] Correctly replaces single-line selections
- [ ] Correctly replaces multi-line selections
- [ ] Preserves content before and after the selection
- [ ] Returns the replacement as a TextEdit with correct range
- [ ] Unit tests for replacement logic
- [ ] Typecheck passes (`cargo check`)

### US-007: Handle execute command for selection implementation
**Description:** As a developer, I need the LSP server to handle the `agent.implSelection` command and orchestrate the implementation workflow.

**Acceptance Criteria:**
- [ ] Parse command arguments (uri, start_line, start_char, end_line, end_char, version, language_id, pending_id)
- [ ] Extract selection text from document
- [ ] Generate unique temp file path for output
- [ ] Spawn worker thread for non-blocking execution
- [ ] Call backend's `implement_selection_streaming()` method
- [ ] Apply replacement via `workspace/applyEdit` when complete
- [ ] Send `amp/jobCompleted` notification on success/failure
- [ ] Typecheck passes (`cargo check`)

### US-008: Add job tracking for selection implementations
**Description:** As a developer, I need selection implementations to integrate with the existing job tracker so concurrent jobs work correctly.

**Acceptance Criteria:**
- [ ] Selection jobs registered in JobTracker with correct line range
- [ ] Line adjustments applied when other jobs complete
- [ ] Job limit (10 per file) enforced for combined function + selection jobs
- [ ] Job cleanup on completion/failure
- [ ] Typecheck passes (`cargo check`)

### US-009: Stream progress for selection implementations
**Description:** As a user, I want to see streaming progress while the AI implements my selection so I know work is happening.

**Acceptance Criteria:**
- [ ] `amp/implFunctionProgress` notifications sent during implementation
- [ ] Progress includes job_id, uri, line (start of selection), and preview text
- [ ] Progress updates work the same as function implementation
- [ ] Typecheck passes (`cargo check`)

### US-010: Add Neovim plugin API for selection implementation
**Description:** As a Neovim user, I want a Lua function to implement visual selections so I can trigger the feature from visual mode.

**Acceptance Criteria:**
- [ ] New function `require('agent_amp').implement_selection()` added
- [ ] Function captures current visual selection range (using `'<` and `'>` marks)
- [ ] Function converts Vim 1-indexed positions to LSP 0-indexed positions
- [ ] Function sends code action request with full range to LSP
- [ ] Function handles case when no selection exists (shows error message)
- [ ] Typecheck passes (Lua syntax valid)

### US-011: Add Neovim user command for selection implementation
**Description:** As a Neovim user, I want a `:AgentImplementSelection` command so I can easily trigger selection implementation.

**Acceptance Criteria:**
- [ ] Command `:AgentImplementSelection` registered in setup()
- [ ] Command works from visual mode (accepts range)
- [ ] Command calls `implement_selection()` with captured range
- [ ] Command shows appropriate error if called without selection
- [ ] Lua tests pass

### US-012: Update LSP client to support range-based code actions
**Description:** As a developer, I need the Lua LSP client to support sending arbitrary ranges for code action requests.

**Acceptance Criteria:**
- [ ] `LspClient:request_code_actions()` accepts optional range parameter
- [ ] When range provided, uses it instead of point range
- [ ] Backward compatible with existing function implementation
- [ ] Lua tests pass

### US-013: Handle spinner and progress for selection jobs
**Description:** As a Neovim user, I want to see spinner and progress indicators for selection implementations.

**Acceptance Criteria:**
- [ ] Spinner starts at selection start line when implementation begins
- [ ] Progress updates show streaming preview text
- [ ] Spinner removed and content applied on completion
- [ ] Error handling shows appropriate message on failure
- [ ] Lua tests pass

### US-014: Add end-to-end test for selection implementation
**Description:** As a developer, I need an e2e test that verifies the full selection implementation workflow.

**Acceptance Criteria:**
- [ ] Test creates a file with a function containing a TODO selection
- [ ] Test sends code action request with selection range
- [ ] Test verifies correct code action is returned
- [ ] Test executes command and verifies workspace edit is applied
- [ ] Test verifies only selected region is replaced
- [ ] Test passes (`cargo test`)

### US-015: Add ignored e2e test with real backend
**Description:** As a developer, I need an ignored test that exercises the full workflow with a real AI backend.

**Acceptance Criteria:**
- [ ] Test marked with `#[ignore]` attribute
- [ ] Test implements a selected code block (e.g., loop body)
- [ ] Test verifies the replacement is syntactically valid
- [ ] Test documents how to run with `--ignored` flag
- [ ] Test passes when run manually with backend available

### US-016: Handle edge case - empty selection
**Description:** As a user, I want appropriate feedback when I trigger selection implementation without a valid selection.

**Acceptance Criteria:**
- [ ] Plugin detects when selection is empty or invalid
- [ ] User sees clear error message: "No selection found"
- [ ] No LSP request is made for empty selections
- [ ] Lua tests pass

### US-017: Handle edge case - selection at end of file
**Description:** As a developer, I need the system to handle selections that extend to the last line of the file.

**Acceptance Criteria:**
- [ ] Selection extraction works when selection ends at EOF
- [ ] Replacement works when selection ends at EOF
- [ ] No off-by-one errors at file boundaries
- [ ] Unit tests cover EOF edge case
- [ ] Typecheck passes (`cargo check`)

### US-018: Handle edge case - overlapping concurrent selections
**Description:** As a developer, I need proper handling when a user starts multiple selection implementations that overlap.

**Acceptance Criteria:**
- [ ] System detects overlapping selection ranges
- [ ] Overlapping jobs are rejected with clear error message
- [ ] Non-overlapping selections in same file work concurrently
- [ ] JobTracker updated to track selection ranges, not just lines
- [ ] Typecheck passes (`cargo check`)

### US-019: Update AGENTS.md documentation
**Description:** As a developer, I need the AGENTS.md documentation updated to reflect the new visual selection feature.

**Acceptance Criteria:**
- [ ] AGENTS.md documents new `agent.implSelection` command
- [ ] AGENTS.md documents new `amp/implSelectionProgress` notification (if different from function)
- [ ] AGENTS.md lists new plugin API function and command
- [ ] AGENTS.md explains difference between function and selection implementation

### US-020: Update README documentation
**Description:** As a user, I need the README updated with visual selection feature documentation so I can learn how to use it.

**Acceptance Criteria:**
- [ ] README documents the new `:AgentImplementSelection` command
- [ ] README documents the `implement_selection()` Lua API function
- [ ] README includes usage example for visual selection workflow
- [ ] README explains when to use selection vs function implementation
- [ ] README includes suggested keymaps for visual mode (e.g., `<leader>is`)

## Functional Requirements

- FR-1: The system must provide a new LSP command `agent.implSelection` that accepts selection range coordinates
- FR-2: The system must generate appropriate code actions based on whether a selection or point is provided
- FR-3: The system must extract the exact text within a selection for inclusion in AI prompts
- FR-4: The system must replace only the selected region with AI-generated code, preserving surrounding content
- FR-5: The system must support streaming progress updates during selection implementation
- FR-6: The system must track selection implementation jobs for concurrent execution (up to 10 per file combined with function jobs)
- FR-7: The system must adjust line numbers for active jobs when selection implementations complete
- FR-8: The system must reject overlapping concurrent selection implementations
- FR-9: The Neovim plugin must provide `implement_selection()` API function
- FR-10: The Neovim plugin must provide `:AgentImplementSelection` user command
- FR-11: The system must handle edge cases: empty selection, EOF selection, multi-line selection

## Non-Goals

- No syntax validation of selections (AI handles incomplete constructs)
- No automatic selection expansion to nearest complete construct
- No selection-specific UI beyond spinner (reuse existing progress display)
- No changes to function implementation behavior
- No support for multiple disjoint selections in a single request
- No integration with Vim's visual block mode (only line/character visual modes)

## Technical Considerations

### LSP Protocol
- Reuse existing notification types (`amp/implFunctionProgress`, `amp/jobCompleted`)
- New command `agent.implSelection` with 7 arguments + optional pending_id
- Code action kind remains `CodeActionKind::QUICKFIX`

### Job Tracking
- Extend JobTracker to store selection ranges (start_line, end_line) instead of just line
- Overlap detection based on line ranges
- Line adjustment considers selection span, not just single line

### Backend Prompts
- Selection-specific prompt template that includes:
  - Selected text with clear delimiters
  - Line range of selection
  - Full file contents for context
  - Instructions to write complete replacement

### Plugin Integration
- Use Vim's `'<` and `'>` marks to get visual selection boundaries
- Convert from 1-indexed Vim to 0-indexed LSP coordinates
- Handle visual line mode vs visual character mode

### Temp File Strategy
- Same approach as function implementation: temp file in same directory
- Unique naming to support concurrent selection and function jobs

## Success Metrics

- Users can implement a visual selection in under 3 seconds of triggering (excluding AI response time)
- Selection implementations complete without affecting other code in the file
- Concurrent selection + function implementations work without conflicts
- No increase in error rate compared to function implementation
- All existing tests continue to pass

## Open Questions

1. Should selection implementation support a "context" parameter for additional user instructions?
2. Should we add a keymap suggestion in documentation (e.g., `<leader>is` for implement selection)?
3. Should the progress notification include selection range or just start line?
4. Should we support visual block mode selections in the future?
