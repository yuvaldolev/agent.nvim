# agent.nvim

An LSP server that integrates AI code generation into any LSP-compatible editor. Supports multiple AI backends including Amp and OpenCode.

## Build & Run

```bash
cargo build           # Debug build
cargo build --release # Release build
cargo run             # Run server (connects via stdio)
cargo check           # Type check
cargo test            # Run all tests
```

## Testing

### Rust Tests

End-to-end tests are in `tests/e2e_test.rs`. They spawn the LSP server as a subprocess and communicate via stdin/stdout using the LSP protocol.

```bash
cargo test                        # Run all tests
cargo test --test e2e_test        # Run only e2e tests
cargo test test_initialization    # Run specific test
```

### Lua Tests

The Neovim plugin has comprehensive unit tests for concurrent implementation logic:

```bash
./nvim-plugin/tests/run_tests.sh    # Run all plugin tests
lua nvim-plugin/tests/spinner_spec.lua  # Run spinner tests
lua nvim-plugin/tests/init_spec.lua     # Run init tests
```

### Test Coverage

**Rust Tests (43 total):**

- `test_initialization`: Verifies LSP handshake and server capabilities
- `test_did_open_and_code_action`: Tests document tracking and code action generation
- `test_did_change`: Tests incremental document sync with text edits
- `test_completion_returns_null`: Verifies completion stub returns null
- `test_unknown_request_returns_error`: Verifies unknown methods return MethodNotFound error
- `test_max_concurrent_jobs_limit`: Verifies max 10 concurrent jobs per file limit

**Lua Tests (28 total):**

- `spinner_spec.lua` (15 tests): SpinnerManager operations, line tracking, multiple concurrent spinners
- `init_spec.lua` (13 tests): Job completion handling, progress updates, concurrent workflow simulation

### Ignored Tests (require backend CLI)

These tests are ignored by default because they require the configured backend CLI (amp, opencode, or claude) to be available:

```bash
cargo test --test e2e_test -- --ignored --nocapture  # Run ignored tests with output
cargo test test_execute_command_prints_modifications -- --ignored --nocapture
cargo test test_single_function_modification -- --ignored --nocapture
cargo test test_claude_code_integration -- --ignored --nocapture
```

- `test_execute_command_prints_modifications`: Calls backend CLI and prints the workspace/applyEdit modifications for visual inspection
- `test_single_function_modification`: Verifies that only the targeted function is modified when there are multiple functions in a file
- `test_concurrent_implementations`: Tests concurrent function implementations across multiple files
- `test_concurrent_same_file_implementations`: Tests multiple concurrent implementations in the same file
- `test_claude_code_integration`: Tests the ClaudeCodeClient directly by invoking the `claude` CLI (requires claude CLI installed)

## Architecture

The server uses `lsp-server` crate (from rust-analyzer) with stdio transport and `lsp-types` for LSP protocol types.

### Modules

- **main.rs**: `Server` struct with `initialize()` and `run()` methods, message dispatch loop
- **handlers.rs**: `RequestHandler` and `NotificationHandler` for LSP message dispatch, spawns concurrent worker threads
- **document_store.rs**: `DocumentStore` with `Arc<Mutex<HashMap<Url, Document>>>` for tracking open files
- **job_tracker.rs**: `JobTracker` for concurrent job tracking with automatic line adjustments (up to 10 jobs per file)
- **backend.rs**: `Backend` trait for AI provider abstraction, `create_backend()` factory function
- **config.rs**: `BackendType` enum, `CURRENT_BACKEND` configuration constant, `DELETE_TEMP_FILES` option, and `MAX_CONCURRENT_JOBS_PER_FILE`
- **amp.rs**: `AmpClient` with `implement_function_streaming()` that reads `amp` CLI stdout line-by-line and calls progress callback
- **opencode.rs**: `OpenCodeClient` with `implement_function_streaming()` that reads CLI stdout and calls progress callback, captures stderr for error reporting
- **lsp_utils.rs**: `LspClient` (response helpers) and `WorkspaceEditBuilder` (workspace edits)
- **utils.rs**: Shared utility functions including `replace_function_in_document()`

### LSP Capabilities

- `textDocument/didOpen`, `textDocument/didChange`: INCREMENTAL sync to DocumentStore
- `textDocument/completion`: Stub (returns null)
- `textDocument/codeAction`: Returns "Implement function with Amp" command
- `workspace/executeCommand`: Handles `amp.implFunction`, spawns concurrent worker threads (non-blocking)
- `amp/implFunctionProgress`: Server-to-client notification with streaming preview text and line updates (params: `job_id`, `uri`, `line`, `preview`)
- `amp/jobCompleted`: Server-to-client notification when implementation finishes (params: `job_id`, `uri`, `success`, `error?`)

## Agent Interaction Protocol

The Agent interaction is file-based to avoid buffer size limits and support concurrent implementations:

1.  **Temp File Path Generation**: LSP generates a unique temporary file path in the **same directory** as the source file (to avoid permission issues). The file is NOT pre-created, allowing the agent to create it directly without reading an empty file first.
2.  **Prompting**: Agent is prompted to write the *full function implementation* (signature + body) directly to this temporary file.
3.  **Reading**: LSP reads the content of the temporary file after the Agent completes.
4.  **Cleanup**: By default, temporary files are deleted after use. Set `DELETE_TEMP_FILES = false` in `src/config.rs` to preserve them for debugging.
5.  **Function Replacement**:
    *   **Direct replacement**: Always uses latest agent output for the specific function, overriding any user edits within that function
    *   **Preserves other code**: All other functions and code outside the target function remain unchanged
    *   **Signature matching**: Logic scans backwards to find the correct start of the function, ensuring even internal CodeAction triggers replace the full signature
6.  **Concurrent handling**:
    *   **Up to 10 parallel jobs per file**: Each with its own temp file and worker thread
    *   **Line tracking**: All active jobs have their line numbers adjusted when other implementations complete
    *   **Live updates**: Each implementation applies immediately when done, no waiting for other jobs

## Configuration

The server supports multiple configuration options in `src/config.rs`:

### Backend Selection

```rust
// Use Amp backend
pub const CURRENT_BACKEND: BackendType = BackendType::Amp;

// Or use OpenCode backend (default)
pub const CURRENT_BACKEND: BackendType = BackendType::OpenCode;
```

### Temporary File Cleanup

```rust
// Delete temporary files after use (default)
pub const DELETE_TEMP_FILES: bool = true;

// Preserve temporary files for debugging
pub const DELETE_TEMP_FILES: bool = false;
```

### Concurrent Job Limit

```rust
// Maximum concurrent implementations per file (default: 10)
pub const MAX_CONCURRENT_JOBS_PER_FILE: usize = 10;
```

After changing any configuration, rebuild the server with `cargo build`.

### Backend Requirements

- **Amp**: Requires `amp` CLI to be installed and authenticated
- **OpenCode**: Requires `opencode` CLI to be installed and authenticated
- **ClaudeCode**: Requires `claude` CLI to be installed and authenticated

## Design Decisions

- **Language agnostic**: Server does NOT parse code. Passes cursor position and file contents to AI CLI, which determines function context.
- **Parallel execution**: Supports up to 10 concurrent implementations per file with non-blocking worker threads.
- **Line tracking**: Active jobs have their line numbers automatically adjusted when other implementations complete.
- **Function-only replacement**: Always uses latest agent output for specific function, preserving other functions and code.
- **Per-job timeout**: Plugin enforces 120-second timeout per implementation (configurable).
- **Versioned edits**: WorkspaceEdit includes `VersionedTextDocumentIdentifier` for concurrency safety.
- **Error reporting**: OpenCode backend captures stderr for meaningful error messages.
- **Logging**: Uses `tracing` to stderr (required since stdio is used for LSP transport).

## Code Style

- All modules use structs with methods (not free functions)
- Error handling via `Box<dyn Error + Sync + Send>`
- Handlers take `&self` references to avoid ownership issues

## Git Conventions

Use [Conventional Commits](https://www.conventionalcommits.org/) format for all commit messages:

```
<type>(<scope>): <description>
```

### Types

- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

### Scopes

- `lsp`: LSP server (Rust code in `src/`)
- `nvim`: Neovim plugin (Lua code in `nvim-plugin/`)
- `test`: Test infrastructure
- `docs`: Documentation

### Examples

```
feat(lsp): add code action for function implementation
fix(nvim): resolve spinner cleanup on error
docs(readme): update installation instructions
refactor(lsp): extract handler dispatch logic
test(lsp): add e2e test for incremental sync
```
