/// Available backend types for function implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    /// Use Amp CLI for function implementation.
    Amp,
    /// Use OpenCode CLI for function implementation.
    OpenCode,
    /// Use Claude Code CLI for function implementation.
    ClaudeCode,
}

/// The currently selected backend for function implementation.
///
/// Change this constant to switch between backends.
pub const CURRENT_BACKEND: BackendType = BackendType::OpenCode;

/// Whether to delete temporary agent implementation files after use.
///
/// When false, temporary files will be preserved in the same directory as the source file.
/// This is useful for debugging agent output.
///
/// Default: true (delete temp files)
pub const DELETE_TEMP_FILES: bool = false;
