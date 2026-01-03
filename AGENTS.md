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

End-to-end tests are in `tests/e2e_test.rs`. They spawn the LSP server as a subprocess and communicate via stdin/stdout using the LSP protocol.

```bash
cargo test                        # Run all tests
cargo test --test e2e_test        # Run only e2e tests
cargo test test_initialization    # Run specific test
```

### Test Coverage

- `test_initialization`: Verifies LSP handshake and server capabilities
- `test_did_open_and_code_action`: Tests document tracking and code action generation
- `test_did_change`: Tests incremental document sync with text edits
- `test_completion_returns_null`: Verifies completion stub returns null
- `test_unknown_request_returns_error`: Verifies unknown methods return MethodNotFound error

### Ignored Tests (require backend CLI)

These tests are ignored by default because they require the configured backend CLI (amp or opencode) to be available:

```bash
cargo test --test e2e_test -- --ignored --nocapture  # Run ignored tests with output
cargo test test_execute_command_prints_modifications -- --ignored --nocapture
cargo test test_single_function_modification -- --ignored --nocapture
```

- `test_execute_command_prints_modifications`: Calls amp CLI and prints the workspace/applyEdit modifications for visual inspection
- `test_single_function_modification`: Verifies that only the targeted function is modified when there are multiple functions in a file
- `test_concurrent_implementations`: Tests concurrent function implementations across multiple files with job queue serialization

## Architecture

The server uses `lsp-server` crate (from rust-analyzer) with stdio transport and `lsp-types` for LSP protocol types.

### Modules

- **main.rs**: `Server` struct with `initialize()` and `run()` methods, message dispatch loop
- **handlers.rs**: `RequestHandler` and `NotificationHandler` for LSP message dispatch
- **document_store.rs**: `DocumentStore` with `Arc<Mutex<HashMap<Url, Document>>>` for tracking open files
- **job_queue.rs**: `JobQueue` for per-file serialization of concurrent implementations with automatic line tracking
- **backend.rs**: `Backend` trait for AI provider abstraction, `create_backend()` factory function
- **config.rs**: `BackendType` enum and `CURRENT_BACKEND` configuration constant
- **amp.rs**: `AmpClient` with `implement_function_streaming()` that reads `amp` CLI stdout line-by-line and calls progress callback
- **opencode.rs**: `OpenCodeClient` with `implement_function_streaming()` that reads `opencode` CLI stdout and calls progress callback
- **lsp_utils.rs**: `LspClient` (response helpers) and `WorkspaceEditBuilder` (workspace edits)
- **utils.rs**: Shared utility functions like `strip_markdown_code_block()`

### LSP Capabilities

- `textDocument/didOpen`, `textDocument/didChange`: INCREMENTAL sync to DocumentStore
- `textDocument/completion`: Stub (returns null)
- `textDocument/codeAction`: Returns "Implement function with Amp" command
- `workspace/executeCommand`: Handles `amp.implFunction`, calls Amp CLI, sends `workspace/applyEdit`
- `amp/implFunctionProgress`: Server-to-client notification with streaming preview text (params: `job_id`, `uri`, `line`, `preview`)

## Agent Interaction Protocol

The Agent interaction has been redesigned to be file-based to avoid buffer size limits and ensure robust merging:

1.  **Temp File Creation**: LSP creates a temporary file in the **same directory** as the source file (to avoid permission issues).
2.  **Prompting**: Agent is prompted to write the *full function implementation* (signature + body) directly to this temporary file.
3.  **Reading**: LSP reads the content of the temporary file after the Agent completes.
4.  **Merging**:
    *   **3-way Merge**: LSP uses `diffy` to perform a 3-way merge between:
        *   **Base**: File content when the job started
        *   **Yours**: Current file content (including any user edits made while Agent was running)
        *   **Theirs**: The Agent's implementation (applied to the Base)
    *   **Duplicate Prevention**: Heuristics detect if the Agent outputs the *entire file* instead of just the snippet, and handle it correctly.
    *   **Signature Matching**: Logic scans backwards to find the correct start of the function, ensuring even internal CodeAction triggers replace the full signature.

## Backend Selection

The server supports multiple AI backends. To switch backends, edit `src/config.rs`:

```rust
// Use Amp backend (default)
pub const CURRENT_BACKEND: BackendType = BackendType::Amp;

// Or use OpenCode backend
pub const CURRENT_BACKEND: BackendType = BackendType::OpenCode;
```

After changing the backend, rebuild the server with `cargo build`.

### Backend Requirements

- **Amp**: Requires `amp` CLI to be installed and authenticated
- **OpenCode**: Requires `opencode` CLI to be installed and authenticated

## Design Decisions

- **Language agnostic**: Server does NOT parse code. Passes cursor position and file contents to AI CLI, which determines function context.
- **Per-file serialization**: Uses `JobQueue` to serialize concurrent implementations within the same file, preventing race conditions when line numbers shift.
- **Line tracking**: Pending jobs have their line numbers automatically adjusted when earlier implementations are applied.
- **Versioned edits**: WorkspaceEdit includes `VersionedTextDocumentIdentifier` for concurrency safety.
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
