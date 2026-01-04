# CLAUDE.md - AI Assistant Guide for agent.nvim

## Project Overview

**agent.nvim** is an AI-powered function implementation tool for Neovim that provides seamless integration with AI coding assistants. It consists of two main components:

1. **agent-lsp** - A Rust-based LSP (Language Server Protocol) server that bridges Neovim and AI backends
2. **nvim-plugin** - A Lua plugin providing the user interface and LSP client management

The project enables developers to implement function placeholders (like `todo!()`) using AI assistance with real-time streaming progress, concurrent execution, and smart 3-way merging.

## Repository Structure

```
agent.nvim/
├── src/                    # Rust LSP server implementation
│   ├── main.rs            # Server entry point, initialization, message loop
│   ├── handlers.rs        # LSP request/notification handlers
│   ├── document_store.rs  # In-memory document tracking
│   ├── job_queue.rs       # Per-file job serialization with line tracking
│   ├── backend.rs         # Backend trait abstraction
│   ├── amp.rs            # Amp backend implementation
│   ├── opencode.rs       # OpenCode backend implementation
│   ├── config.rs         # Backend selection and configuration
│   ├── lsp_utils.rs      # LSP response helpers and workspace edit builder
│   └── utils.rs          # Shared utility functions
├── nvim-plugin/           # Neovim Lua plugin
│   ├── plugin/
│   │   └── agent_amp.lua # Auto-loads on startup
│   └── lua/agent_amp/
│       ├── init.lua      # Main module, setup(), user commands
│       ├── lsp.lua       # LSP client management
│       └── spinner.lua   # Animated progress indicators
├── tests/
│   ├── e2e_test.rs       # End-to-end LSP protocol tests
│   └── manual_test.sh    # Manual testing script
├── .github/
│   └── workflows/
│       └── opencode.yml  # GitHub Actions for OpenCode integration
├── Cargo.toml            # Rust dependencies
├── Cargo.lock            # Locked dependency versions
├── README.md             # User-facing documentation
├── AGENTS.md             # Backend-specific documentation
└── LICENSE               # Project license

Build artifacts:
├── target/               # Cargo build output (gitignored)
│   ├── debug/           # Debug builds
│   │   └── agent-lsp   # Debug LSP server binary
│   └── release/         # Release builds
│       └── agent-lsp   # Release LSP server binary
```

## Architecture Overview

### High-Level Data Flow

```
User Command (:AmpImplementFunction)
    ↓
Neovim Plugin (Lua)
    ↓ [LSP Protocol over stdio]
agent-lsp Server (Rust)
    ↓ [CLI execution]
AI Backend (Amp/OpenCode)
    ↓ [Streaming responses]
agent-lsp Server
    ↓ [workspace/applyEdit]
Neovim Plugin
    ↓
Code Inserted into Buffer
```

### Key Design Principles

1. **Language Agnostic**: The LSP server does NOT parse code syntax. It passes cursor position and file contents to the AI backend, which determines function context.

2. **Per-File Serialization**: The `JobQueue` ensures that concurrent function implementations within the same file execute serially to prevent race conditions when line numbers shift.

3. **Line Tracking**: Pending jobs automatically adjust their line numbers when earlier implementations insert/remove lines.

4. **3-Way Merge**: Uses `diffy` crate to merge:
   - **Base**: Original file content when job started
   - **Yours**: Current file content (including user edits during AI processing)
   - **Theirs**: AI-generated implementation applied to Base

5. **Incremental Sync**: Uses `TextDocumentSyncKind::INCREMENTAL` for efficient document updates.

6. **Versioned Edits**: `WorkspaceEdit` includes `VersionedTextDocumentIdentifier` for concurrency safety.

## Component Details

### Rust LSP Server (`src/`)

#### main.rs
- `Server` struct manages LSP connection, document store, and job queue
- `initialize()` negotiates capabilities with client
- `run()` implements message dispatch loop
- Logging via `tracing` to stderr (stdout reserved for LSP protocol)

#### handlers.rs
- `RequestHandler`: Handles LSP requests (completion, code actions, execute command)
- `NotificationHandler`: Handles LSP notifications (didOpen, didChange)
- `COMMAND_IMPL_FUNCTION`: Command ID for "amp.implFunction"
- `NOTIFICATION_IMPL_FUNCTION_PROGRESS`: Progress notification for streaming updates

#### document_store.rs
- `DocumentStore`: Thread-safe `Arc<Mutex<HashMap<Url, Document>>>`
- Tracks open documents and their content
- Updated via `didOpen` and `didChange` notifications
- Provides `get_document()` and `update_document()` methods

#### job_queue.rs
- `JobQueue`: Manages concurrent function implementations
- **Per-file serialization**: Only one active job per file
- `acquire(uri, job_id, line)`: Blocks until slot available, returns adjusted line
- `release(uri, job_id)`: Releases slot, promotes next pending job
- `adjust_pending_lines(uri, edit_line, lines_delta)`: Updates line numbers for pending jobs

#### backend.rs
- `Backend` trait: Abstraction for AI providers
  - `implement_function()`: Synchronous implementation
  - `implement_function_streaming()`: Streaming with progress callbacks
- `create_backend()`: Factory function based on `CURRENT_BACKEND` config

#### amp.rs
- `AmpClient`: Implements `Backend` trait for Amp CLI
- Executes: `amp --execute "<prompt>" --stream-json`
- Parses JSON-lines output for progress updates
- AI writes implementation to temporary file

#### opencode.rs
- `OpenCodeClient`: Implements `Backend` trait for OpenCode CLI
- Executes: `opencode --execute "<prompt>" --stream-json`
- Similar streaming protocol to Amp
- Currently configured as default backend

#### config.rs
- `BackendType` enum: `Amp` or `OpenCode`
- `CURRENT_BACKEND`: Compile-time backend selection (default: `OpenCode`)
- `DELETE_TEMP_FILES`: Whether to clean up temp files (default: `false` for debugging)

#### lsp_utils.rs
- `LspClient`: Helper for sending LSP responses
  - `send_success()`, `send_error()`, `send_method_not_found()`
  - `send_notification()` for progress updates
- `WorkspaceEditBuilder`: Constructs versioned workspace edits

#### utils.rs
- `strip_markdown_code_block()`: Removes markdown fences from AI output
- Shared utilities used across modules

### Neovim Plugin (`nvim-plugin/`)

#### plugin/agent_amp.lua
- Auto-loaded by Neovim on startup
- Calls `require("agent_amp").setup()` if not already configured

#### lua/agent_amp/init.lua
- `AgentAmp` class: Main plugin coordinator
- `setup(opts)`: Initializes plugin with optional configuration
- `implement_function()`: Entry point for `:AmpImplementFunction` command
- `_on_apply_edit()`: Handles workspace edits, stops spinners
- `_on_progress()`: Handles streaming progress, updates ghost text
- Manages pending jobs and spinner lifecycle

#### lua/agent_amp/lsp.lua
- `LspClient` class: Manages LSP server lifecycle
- `_resolve_cmd()`: Automatic binary discovery:
  1. User-specified `cmd` from `setup()`
  2. PATH lookup via `vim.fn.exepath("agent-lsp")`
  3. `target/release/agent-lsp` (relative to plugin root)
  4. `target/debug/agent-lsp` (fallback)
- `start()`: Spawns LSP server process
- `stop()`: Terminates LSP server
- Handles `workspace/applyEdit` and `amp/implFunctionProgress` notifications

#### lua/agent_amp/spinner.lua
- `SpinnerManager` class: Manages multiple concurrent spinners
- `start(job_id, bufnr, line)`: Creates animated spinner at line
- `stop(job_id)`: Removes spinner and ghost text
- `update_preview(job_id, preview_text)`: Updates ghost text
- 40-second timeout per spinner
- Tracks jobs by URI and line number for proper cleanup

## LSP Protocol Implementation

### Capabilities Advertised

```json
{
  "textDocumentSync": "INCREMENTAL",
  "completionProvider": { "triggerCharacters": ["."] },
  "codeActionProvider": { "codeActionKinds": ["quickfix"] },
  "executeCommandProvider": { "commands": ["amp.implFunction"] }
}
```

### Request Flow: Function Implementation

1. **User triggers**: `:AmpImplementFunction`
2. **Plugin sends**: `textDocument/codeAction` request
3. **Server responds**: `CodeAction` with command `amp.implFunction`
4. **Plugin starts**: Spinner at cursor line
5. **Plugin sends**: `workspace/executeCommand` with params:
   ```json
   {
     "command": "amp.implFunction",
     "arguments": [{
       "textDocument": { "uri": "file://..." },
       "position": { "line": 42, "character": 0 }
     }]
   }
   ```
6. **Server spawns**: Background thread for AI execution
7. **Server streams**: `amp/implFunctionProgress` notifications with preview text
8. **Plugin updates**: Ghost text in real-time
9. **Server sends**: `workspace/applyEdit` with final implementation
10. **Plugin applies**: Text edits and stops spinner

### Concurrent Execution

Multiple function implementations can run simultaneously:
- **Across different files**: Fully parallel execution
- **Within same file**: Serialized via `JobQueue`, with automatic line tracking

## Development Workflow

### Building

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Type checking only
cargo check
```

### Testing

```bash
# Run all tests
cargo test

# Run only e2e tests
cargo test --test e2e_test

# Run specific test
cargo test test_initialization

# Run ignored tests (require AI backend CLI)
cargo test --test e2e_test -- --ignored --nocapture
```

### Test Coverage

| Test | Type | Description |
|------|------|-------------|
| `test_initialization` | Unit | LSP handshake and capabilities |
| `test_did_open_and_code_action` | Unit | Document tracking and code actions |
| `test_did_change` | Unit | Incremental sync with text edits |
| `test_completion_returns_null` | Unit | Completion stub behavior |
| `test_unknown_request_returns_error` | Unit | Error handling |
| `test_execute_command_prints_modifications` | Ignored | Calls real AI CLI |
| `test_single_function_modification` | Ignored | Verifies targeted modification |
| `test_concurrent_implementations` | Ignored | Tests parallel execution |

### Running the Server Manually

```bash
# Run server in debug mode (logs to stderr)
cargo run

# The server expects LSP protocol on stdin/stdout
# Use an LSP client to communicate with it
```

### Debugging

1. **Enable verbose logging**: Logs go to stderr automatically
2. **Preserve temp files**: Set `DELETE_TEMP_FILES = false` in `src/config.rs`
3. **Check temp files**: Generated in same directory as source files
4. **Neovim logs**: `:messages` shows plugin notifications
5. **LSP logs**: Check Neovim's LSP log (`:LspLog` if available)

## Code Conventions

### Rust Code Style

1. **Module Organization**:
   - All modules use structs with methods, not free functions
   - Each file exports a single primary type (e.g., `DocumentStore`, `JobQueue`)

2. **Error Handling**:
   - Use `Result<T, Box<dyn Error + Sync + Send>>` for all fallible operations
   - Propagate errors with `?` operator
   - Log errors with `tracing::error!` before returning

3. **Concurrency**:
   - Use `Arc<Mutex<T>>` for shared mutable state
   - Prefer message passing (`crossbeam-channel`) for async communication
   - Always lock mutexes for minimal duration

4. **Method Signatures**:
   - Handlers take `&self` references (immutable)
   - Use `Arc::clone()` explicitly when cloning shared state
   - Avoid unnecessary ownership transfers

5. **Logging**:
   - Use `tracing` crate (`info!`, `error!`, `debug!`)
   - All logs go to stderr (stdout reserved for LSP)
   - Include context: URIs, line numbers, job IDs

### Lua Code Style

1. **Class Pattern**:
   ```lua
   local MyClass = {}
   MyClass.__index = MyClass

   function MyClass.new(opts)
       local self = setmetatable({}, MyClass)
       -- initialization
       return self
   end
   ```

2. **Error Handling**:
   - Use `pcall()` for operations that might fail
   - Show user-facing errors with `vim.notify(msg, vim.log.levels.ERROR)`

3. **LSP Communication**:
   - Use `vim.lsp.buf_request()` for synchronous requests
   - Use `vim.lsp.buf_notify()` for asynchronous notifications

### Git Commit Conventions

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]
```

**Types**:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `refactor`: Code restructuring without behavior change
- `test`: Adding or updating tests
- `chore`: Maintenance (dependencies, CI, etc.)
- `ci`: CI/CD changes

**Scopes**:
- `lsp`: LSP server (Rust code)
- `nvim`: Neovim plugin (Lua code)
- `test`: Test infrastructure
- `docs`: Documentation files

**Examples**:
```
feat(lsp): add support for OpenCode backend
fix(nvim): resolve spinner timeout race condition
docs(readme): update installation instructions
refactor(lsp): extract merge logic to separate module
test(lsp): add concurrent implementation test
ci: add OpenCode workflow
```

## Configuration Guide

### Changing AI Backend

Edit `src/config.rs`:

```rust
// Use OpenCode (default)
pub const CURRENT_BACKEND: BackendType = BackendType::OpenCode;

// Or use Amp
pub const CURRENT_BACKEND: BackendType = BackendType::Amp;
```

Then rebuild: `cargo build --release`

### Backend Requirements

- **Amp**: Requires `amp` CLI installed and authenticated
- **OpenCode**: Requires `opencode` CLI installed and authenticated

Verify installation:
```bash
which amp      # or which opencode
amp --version  # or opencode --version
```

### Temporary File Management

Edit `src/config.rs`:

```rust
// Delete temp files after use (production)
pub const DELETE_TEMP_FILES: bool = true;

// Preserve temp files (debugging)
pub const DELETE_TEMP_FILES: bool = false;
```

Temp files are created in the same directory as the source file with naming pattern:
`<original-filename>.tmp_<uuid>.<ext>`

### Neovim Plugin Configuration

```lua
require("agent_amp").setup({
    -- Optional: override automatic binary detection
    cmd = { "/custom/path/to/agent-lsp" },
})
```

Default binary resolution order:
1. User-specified `cmd`
2. `agent-lsp` in PATH
3. `target/release/agent-lsp` (relative to plugin)
4. `target/debug/agent-lsp` (fallback)

## Common Development Tasks

### Adding a New Backend

1. **Create backend implementation** in `src/newbackend.rs`:
   ```rust
   use crate::backend::Backend;

   pub struct NewBackendClient;

   impl NewBackendClient {
       pub fn new() -> Self {
           Self
       }
   }

   impl Backend for NewBackendClient {
       fn implement_function_streaming(...) -> Result<(), Box<dyn Error + Sync + Send>> {
           // Implementation
       }
   }
   ```

2. **Add to config** in `src/config.rs`:
   ```rust
   pub enum BackendType {
       Amp,
       OpenCode,
       NewBackend,  // Add here
   }
   ```

3. **Update factory** in `src/backend.rs`:
   ```rust
   pub fn create_backend() -> Box<dyn Backend> {
       match CURRENT_BACKEND {
           BackendType::Amp => Box::new(AmpClient::new()),
           BackendType::OpenCode => Box::new(OpenCodeClient::new()),
           BackendType::NewBackend => Box::new(NewBackendClient::new()),
       }
   }
   ```

4. **Add module** to `src/main.rs`:
   ```rust
   mod newbackend;
   ```

5. **Test**: Write e2e test in `tests/e2e_test.rs`

6. **Document**: Update `AGENTS.md` with backend-specific details

### Modifying LSP Capabilities

1. **Update capability declaration** in `src/main.rs`:
   ```rust
   fn initialize(&self) -> Result<serde_json::Value, Box<dyn Error + Sync + Send>> {
       let capabilities = ServerCapabilities {
           // Add new capability here
           ..Default::default()
       };
   }
   ```

2. **Add handler** in `src/handlers.rs`:
   ```rust
   fn handle(&self, req: &Request) -> Result<(), Box<dyn Error + Sync + Send>> {
       match req.method.as_str() {
           NewRequest::METHOD => self.handle_new_request(req, &lsp_client),
           // ...
       }
   }
   ```

3. **Update Neovim plugin** if client needs to send new requests

4. **Test**: Add e2e test for new capability

### Adding Progress Indicators

Progress notifications use `amp/implFunctionProgress`:

**Server side** (`src/handlers.rs`):
```rust
lsp_client.send_notification::<ImplFunctionProgressParams>(
    NOTIFICATION_IMPL_FUNCTION_PROGRESS,
    ImplFunctionProgressParams {
        job_id: job_id.clone(),
        uri: uri.to_string(),
        line,
        preview: "Analyzing function...".to_string(),
    },
)?;
```

**Client side** (`nvim-plugin/lua/agent_amp/init.lua`):
```lua
function AgentAmp:_on_progress(params)
    self.spinner_manager:update_preview(params.job_id, params.preview)
end
```

## Important Files for AI Assistants

When making changes, these files are most frequently modified:

### High-Impact Files (modify with care)

| File | Purpose | Caution |
|------|---------|---------|
| `src/handlers.rs` | Core LSP logic | Changes affect protocol compliance |
| `src/job_queue.rs` | Concurrency control | Race conditions possible |
| `nvim-plugin/lua/agent_amp/init.lua` | Plugin coordination | User-facing behavior |
| `src/config.rs` | Build-time configuration | Requires rebuild |

### Safe to Modify

| File | Purpose | Notes |
|------|---------|-------|
| `src/amp.rs` | Amp backend | Isolated from other backends |
| `src/opencode.rs` | OpenCode backend | Isolated from other backends |
| `src/utils.rs` | Utility functions | No global state |
| `nvim-plugin/lua/agent_amp/spinner.lua` | UI only | Visual feedback |
| `README.md` | User documentation | Always keep in sync |
| `AGENTS.md` | Backend docs | Update when backends change |

### Test Files

| File | Purpose | When to Update |
|------|---------|----------------|
| `tests/e2e_test.rs` | Integration tests | Add tests for new features |
| `tests/manual_test.sh` | Manual testing | Update for workflow changes |

## AI Assistant Guidelines

### When Reviewing Code

1. **Check concurrency safety**:
   - Verify `Arc<Mutex<T>>` usage is correct
   - Ensure job queue serialization is maintained
   - Look for potential race conditions in line tracking

2. **Verify LSP compliance**:
   - Response IDs must match request IDs
   - Versioned edits must include correct document versions
   - Capabilities must match advertised server capabilities

3. **Test error handling**:
   - All `Result` types should propagate errors correctly
   - User-facing errors should be informative
   - Backend failures shouldn't crash the server

4. **Validate merge logic**:
   - 3-way merge should handle concurrent user edits
   - Function boundary detection should be robust
   - Full-file outputs should be detected and handled

### When Implementing Features

1. **Start with tests**: Write e2e test first (TDD approach)

2. **Maintain language agnosticism**: Don't add language-specific parsing

3. **Preserve concurrency guarantees**: Use `JobQueue` for file-level serialization

4. **Update documentation**: Modify README.md, AGENTS.md, and this file

5. **Follow commit conventions**: Use conventional commit format

6. **Consider backwards compatibility**: LSP clients rely on stable protocol

### When Debugging Issues

1. **Check logs first**:
   - Server logs: stderr output from `cargo run`
   - Neovim logs: `:messages` in Neovim
   - Temp files: Set `DELETE_TEMP_FILES = false`

2. **Isolate the component**:
   - LSP server: Run `cargo test`
   - Backend: Run ignored tests with `--nocapture`
   - Plugin: Check Lua errors in `:messages`

3. **Reproduce with minimal case**:
   - Single file, single function
   - Disable concurrent operations
   - Use manual test script

4. **Verify external dependencies**:
   - Backend CLI installed and authenticated
   - Neovim version >= 0.10
   - Rust toolchain available

### Common Pitfalls to Avoid

1. **Don't parse code in LSP server**: Rely on AI backend for context

2. **Don't modify files outside job queue**: Race conditions will occur

3. **Don't assume line numbers are stable**: They shift during edits

4. **Don't use stdout for logging**: Reserved for LSP protocol

5. **Don't hardcode file paths**: Use relative paths or configuration

6. **Don't skip version checks**: LSP edits need correct versions

7. **Don't ignore temp file cleanup**: Set `DELETE_TEMP_FILES` appropriately

## Resources

- **LSP Specification**: https://microsoft.github.io/language-server-protocol/
- **Neovim LSP Guide**: https://neovim.io/doc/user/lsp.html
- **rust-analyzer LSP server**: https://github.com/rust-lang/rust-analyzer (reference implementation)
- **lsp-server crate**: https://docs.rs/lsp-server/
- **Conventional Commits**: https://www.conventionalcommits.org/

## Project Status

- **Current version**: 0.1.0
- **Default backend**: OpenCode
- **Stability**: Active development
- **Test coverage**: Core functionality covered, backends require manual testing

## Future Considerations

Potential areas for enhancement (not currently planned):

- Multiple backend configuration without rebuild
- Configuration file support (`.agent-lsp.toml`)
- Custom AI prompt templates
- Caching of AI responses
- Telemetry and analytics
- Support for additional LSP clients beyond Neovim
- Plugin for other editors (VS Code, IntelliJ)

---

**Last Updated**: 2026-01-04
**Maintained By**: Project contributors
**For Questions**: See GitHub issues at https://github.com/yuvaldolev/agent.nvim
