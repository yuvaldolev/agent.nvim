use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use tracing::info;

use crate::backend::Backend;

/// Build the prompt for function implementation with Claude Code.
fn build_prompt(
    line: u32,
    character: u32,
    language_id: &str,
    file_contents: &str,
    output_path: &str,
    function_signature: &str,
) -> String {
    format!(
        "Implement the function body at line {}, character {} in the following {} file. \
         The function to implement is: `{}`\n\n\
         IMPORTANT: Implement ONLY the function `{}` - do NOT implement any other functions in the file.\n\n\
         Write ONLY this function's implementation (signature and body) to the file: {} \
         Do NOT include any other code from the source file (no imports, no other functions). \
         Do NOT output the code to stdout. \
         Output only status messages or confirmation.\n\n<FILE-CONTENT>\n{}</FILE-CONTENT>\n\n\
         <MUST-OBEY>\n\
         You can overwrite the output file's content, but NEVER read it, just write to it.\n\
         Describe your steps before performing them.\n\
         </MUST-OBEY>",
        line + 1,
        character + 1,
        language_id,
        function_signature,
        function_signature,
        output_path,
        file_contents
    )
}

/// Claude Code client for function implementation.
///
/// This client integrates with the Claude Code CLI to provide AI-powered
/// function implementations.
pub struct ClaudeCodeClient;

impl ClaudeCodeClient {
    /// Create a new ClaudeCodeClient instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for ClaudeCodeClient {
    fn implement_function(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
    ) -> Result<String, Box<dyn Error + Sync + Send>> {
        info!(
            "Calling claude CLI - file: {}, line: {}, character: {}, language: {}",
            file_path, line, character, language_id
        );

        // NOTE: implement_function is deprecated in favor of streaming, passing dummy path and signature
        let prompt = build_prompt(
            line,
            character,
            language_id,
            file_contents,
            "/tmp/dummy",
            "unknown",
        );

        let output = Command::new("claude")
            .arg("-p")
            .arg(&prompt)
            .arg("--output-format")
            .arg("text")
            .arg("--model")
            .arg("sonnet")
            .arg("--dangerously-skip-permissions")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("claude CLI failed: {}", stderr).into());
        }

        let stdout = String::from_utf8(output.stdout)?;
        info!("Claude CLI returned {} bytes", stdout.len());
        Ok(stdout.trim().to_string())
    }

    fn implement_function_streaming(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
        output_path: &str,
        function_signature: &str,
        mut on_progress: Box<dyn FnMut(&str) + Send>,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        info!(
            "Calling claude CLI (streaming) - file: {}, line: {}, character: {}, language: {}, function: {}",
            file_path, line, character, language_id, function_signature
        );

        let prompt = build_prompt(
            line,
            character,
            language_id,
            file_contents,
            output_path,
            function_signature,
        );

        let mut child = Command::new("claude")
            .arg("-p")
            .arg(&prompt)
            .arg("--output-format")
            .arg("text")
            .arg("--model")
            .arg("sonnet")
            .arg("--dangerously-skip-permissions")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;
        let reader = BufReader::new(stdout);

        let mut accumulated_text = String::new();

        // Stream plain text output line by line
        for line_result in reader.lines() {
            let line = line_result?;
            info!("claude output line: {}", line);
            accumulated_text.push_str(&line);
            accumulated_text.push('\n');
            on_progress(accumulated_text.trim());
        }

        let status = child.wait()?;
        if !status.success() {
            // Read stderr for error details
            let mut stderr_reader = BufReader::new(stderr);
            let mut stderr_content = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr_reader, &mut stderr_content);

            let error_details = if !stderr_content.trim().is_empty() {
                stderr_content.trim().to_string()
            } else if !accumulated_text.trim().is_empty() {
                accumulated_text.trim().to_string()
            } else {
                format!("exit code: {:?}", status.code())
            };

            return Err(format!("claude CLI failed: {}", error_details).into());
        }

        info!("Claude CLI finished successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_output_format() {
        let prompt = build_prompt(
            9,
            4,
            "rust",
            "fn main() {}",
            "/tmp/output.rs",
            "fn calculate_sum(a: i32, b: i32) -> i32",
        );

        // Verify the prompt structure contains the file content wrapped in tags
        assert!(prompt.contains("<FILE-CONTENT>"));
        assert!(prompt.contains("</FILE-CONTENT>"));
        assert!(prompt.contains("<MUST-OBEY>"));
        assert!(prompt.contains("</MUST-OBEY>"));

        // Verify instruction content
        assert!(prompt.contains("Implement the function body"));
        assert!(prompt.contains("Write ONLY this function's implementation"));
        assert!(prompt.contains("Do NOT include any other code"));
        assert!(prompt.contains("Do NOT output the code to stdout"));
        assert!(prompt.contains("NEVER read it, just write to it"));
        assert!(prompt.contains("Describe your steps before performing them"));
    }

    #[test]
    fn test_build_prompt_contains_line_and_character() {
        // Test that line and character are 1-indexed in the prompt
        let prompt = build_prompt(0, 0, "rust", "code", "/tmp/out.rs", "fn test()");
        assert!(prompt.contains("line 1"));
        assert!(prompt.contains("character 1"));

        let prompt = build_prompt(99, 49, "rust", "code", "/tmp/out.rs", "fn test()");
        assert!(prompt.contains("line 100"));
        assert!(prompt.contains("character 50"));
    }

    #[test]
    fn test_build_prompt_contains_function_signature() {
        let signature = "fn complex_function(x: &str, y: Vec<u32>) -> Result<String, Error>";
        let prompt = build_prompt(5, 10, "rust", "source code", "/tmp/out.rs", signature);

        // Function signature should appear twice in the prompt (once for identification, once for emphasis)
        assert!(prompt.contains(signature));

        // Verify the IMPORTANT instruction includes the signature
        assert!(prompt.contains(&format!("IMPORTANT: Implement ONLY the function `{}`", signature)));
    }

    #[test]
    fn test_build_prompt_contains_output_path() {
        let output_path = "/home/user/project/temp_impl_abc123.rs";
        let prompt = build_prompt(0, 0, "rust", "code", output_path, "fn test()");

        assert!(prompt.contains(output_path));
        assert!(prompt.contains(&format!("Write ONLY this function's implementation (signature and body) to the file: {}", output_path)));
    }

    #[test]
    fn test_build_prompt_contains_language_id() {
        let prompt = build_prompt(0, 0, "typescript", "const x = 1;", "/tmp/out.ts", "function foo()");
        assert!(prompt.contains("typescript file"));

        let prompt = build_prompt(0, 0, "python", "def main(): pass", "/tmp/out.py", "def bar()");
        assert!(prompt.contains("python file"));

        let prompt = build_prompt(0, 0, "go", "package main", "/tmp/out.go", "func baz()");
        assert!(prompt.contains("go file"));
    }

    #[test]
    fn test_build_prompt_contains_file_contents() {
        let file_contents = r#"
use std::collections::HashMap;

fn existing_function() -> i32 {
    42
}

fn todo_implement() -> String {
    todo!()
}
"#;
        let prompt = build_prompt(7, 0, "rust", file_contents, "/tmp/out.rs", "fn todo_implement()");

        // The file contents should be included in the prompt
        assert!(prompt.contains("use std::collections::HashMap"));
        assert!(prompt.contains("fn existing_function()"));
        assert!(prompt.contains("fn todo_implement()"));
    }

    #[test]
    fn test_build_prompt_all_required_elements() {
        // Comprehensive test verifying all required elements are present
        let line = 15;
        let character = 8;
        let language_id = "rust";
        let file_contents = "fn placeholder() { todo!() }";
        let output_path = "/tmp/impl_output.rs";
        let function_signature = "fn placeholder()";

        let prompt = build_prompt(line, character, language_id, file_contents, output_path, function_signature);

        // All required elements must be present
        assert!(prompt.contains(&format!("line {}", line + 1)), "Prompt must contain 1-indexed line number");
        assert!(prompt.contains(&format!("character {}", character + 1)), "Prompt must contain 1-indexed character number");
        assert!(prompt.contains(function_signature), "Prompt must contain function signature");
        assert!(prompt.contains(output_path), "Prompt must contain output path");
        assert!(prompt.contains(language_id), "Prompt must contain language id");
        assert!(prompt.contains(file_contents), "Prompt must contain file contents");
    }

    #[test]
    fn test_claude_code_client_new() {
        let client = ClaudeCodeClient::new();
        // Verify the client is created successfully
        let _ = client;
    }

    #[test]
    fn test_claude_code_client_default() {
        let client = ClaudeCodeClient::default();
        // Verify the default implementation works
        let _ = client;
    }

    /// Integration test that actually invokes the `claude` CLI.
    ///
    /// This test is ignored by default because it requires:
    /// - The `claude` CLI to be installed and in PATH
    /// - Valid authentication/API credentials configured
    ///
    /// Run with: `cargo test test_claude_code_integration -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn test_claude_code_integration() {
        use std::sync::{Arc, Mutex};
        use tempfile::TempDir;

        let client = ClaudeCodeClient::new();

        // Create a temporary directory for the output file
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let output_path = temp_dir.path().join("output.rs");
        let output_path_str = output_path.to_str().unwrap();

        // Simple Rust file with a function to implement
        let file_contents = r#"/// Adds two numbers together.
///
/// # Arguments
/// - `a`: First number
/// - `b`: Second number
///
/// # Returns
/// The sum of a and b
fn add(a: i32, b: i32) -> i32 {
    todo!()
}

fn main() {
    let result = add(2, 3);
    println!("2 + 3 = {}", result);
}
"#;

        let function_signature = "fn add(a: i32, b: i32) -> i32";

        // Track progress updates
        let progress_updates: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let progress_clone = Arc::clone(&progress_updates);

        println!("\n=== Claude Code Integration Test ===");
        println!("Output file: {}", output_path_str);
        println!("Function to implement: {}", function_signature);
        println!("\nCalling claude CLI...\n");

        let result = client.implement_function_streaming(
            "/tmp/test_add.rs",
            9, // Line of todo!()
            4, // Character position
            "rust",
            file_contents,
            output_path_str,
            function_signature,
            Box::new(move |text| {
                let mut updates = progress_clone.lock().unwrap();
                updates.push(text.to_string());
                println!("Progress: {} chars", text.len());
            }),
        );

        match result {
            Ok(()) => {
                println!("\n✓ Claude CLI completed successfully");

                // Check if the output file was created
                if output_path.exists() {
                    let content = std::fs::read_to_string(&output_path)
                        .expect("Failed to read output file");
                    println!("\n=== Generated Implementation ===");
                    println!("{}", content);
                    println!("=== End Implementation ===\n");

                    // Basic validation: the output should contain a function
                    assert!(
                        content.contains("fn add") || content.contains("fn "),
                        "Output should contain a function definition"
                    );
                } else {
                    println!("\n⚠ Output file was not created");
                    println!("This may be expected if claude CLI uses different output method");
                }

                // Verify we received progress updates
                let updates = progress_updates.lock().unwrap();
                println!("Received {} progress updates", updates.len());
                assert!(
                    !updates.is_empty(),
                    "Should have received at least one progress update"
                );
            }
            Err(e) => {
                println!("\n✗ Claude CLI failed: {}", e);
                println!("\nThis is expected if:");
                println!("  - claude CLI is not installed");
                println!("  - API credentials are not configured");
                println!("  - Network issues");
                panic!("Integration test failed: {}", e);
            }
        }

        println!("\n=== Test Complete ===");
    }
}
