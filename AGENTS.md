# agent.nvim

An LSP server that integrates Amp AI code generation into any LSP-compatible editor.

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

### Ignored Tests (require amp CLI)

These tests are ignored by default because they require the `amp` CLI to be available:

```bash
cargo test --test e2e_test -- --ignored --nocapture  # Run ignored tests with output
cargo test test_execute_command_prints_modifications -- --ignored --nocapture
cargo test test_single_function_modification -- --ignored --nocapture
```

- `test_execute_command_prints_modifications`: Calls amp CLI and prints the workspace/applyEdit modifications for visual inspection
- `test_single_function_modification`: Verifies that only the targeted function is modified when there are multiple functions in a file

## Architecture

The server uses `lsp-server` crate (from rust-analyzer) with stdio transport and `lsp-types` for LSP protocol types.

### Modules

- **main.rs**: `Server` struct with `initialize()` and `run()` methods, message dispatch loop
- **handlers.rs**: `RequestHandler` and `NotificationHandler` for LSP message dispatch
- **document_store.rs**: `DocumentStore` with `Arc<Mutex<HashMap<Url, Document>>>` for tracking open files
- **amp.rs**: `AmpClient` with `implement_function_streaming()` that reads `amp` CLI stdout line-by-line and calls progress callback
- **lsp_utils.rs**: `LspClient` (response helpers) and `WorkspaceEditBuilder` (workspace edits)

### LSP Capabilities

- `textDocument/didOpen`, `textDocument/didChange`: INCREMENTAL sync to DocumentStore
- `textDocument/completion`: Stub (returns null)
- `textDocument/codeAction`: Returns "Implement function with Amp" command
- `workspace/executeCommand`: Handles `amp.implFunction`, calls Amp CLI with streaming, sends `workspace/applyEdit`
- `amp/implFunctionProgress`: Server-to-client notification with streaming preview text (params: `uri`, `line`, `preview`)

## Design Decisions

- **Language agnostic**: Server does NOT parse code. Passes cursor position and file contents to Amp CLI, which determines function context.
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
