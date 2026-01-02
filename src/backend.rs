use std::error::Error;

use crate::amp::AmpClient;
use crate::config::{BackendType, CURRENT_BACKEND};
use crate::opencode::OpenCodeClient;

/// Trait for AI backends that can implement functions.
/// 
/// This abstraction allows switching between different AI providers
/// (e.g., Amp, OpenCode) for function implementation.
pub trait Backend: Send + Sync {
    /// Implement a function at the given location.
    /// 
    /// Returns the function body implementation as a string.
    fn implement_function(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
    ) -> Result<String, Box<dyn Error + Sync + Send>>;

    /// Implement a function with streaming progress updates.
    /// 
    /// The `on_progress` callback is called with intermediate results
    /// as the implementation is being generated.
    fn implement_function_streaming(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
        on_progress: Box<dyn FnMut(&str) + Send>,
    ) -> Result<String, Box<dyn Error + Sync + Send>>;
}

/// Create a backend instance based on the current configuration.
///
/// Returns a boxed trait object implementing the `Backend` trait.
/// The specific implementation is determined by `CURRENT_BACKEND` in config.
pub fn create_backend() -> Box<dyn Backend> {
    match CURRENT_BACKEND {
        BackendType::Amp => Box::new(AmpClient::new()),
        BackendType::OpenCode => Box::new(OpenCodeClient::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_backend_returns_configured_backend() {
        // This test verifies that create_backend() returns a valid backend
        // The actual type depends on CURRENT_BACKEND configuration
        let backend = create_backend();
        
        // We can't easily test the exact type, but we can verify it's valid
        // by checking that the trait object was created successfully
        let _ = backend;
    }
}
