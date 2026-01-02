/// Available backend types for function implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    /// Use Amp CLI for function implementation.
    Amp,
    /// Use OpenCode CLI for function implementation.
    OpenCode,
}

/// The currently selected backend for function implementation.
///
/// Change this constant to switch between backends.
pub const CURRENT_BACKEND: BackendType = BackendType::OpenCode;
