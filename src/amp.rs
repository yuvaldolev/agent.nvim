use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use serde::Deserialize;
use tracing::info;

use crate::backend::Backend;
use crate::utils::strip_markdown_code_block;

#[derive(Debug, Deserialize)]
struct AmpMessage {
    #[serde(rename = "type")]
    msg_type: String,
    result: Option<String>,
    is_error: Option<bool>,
    message: Option<AssistantMessage>,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// Build the prompt for function implementation with Amp.
fn build_prompt(
    line: u32,
    character: u32,
    language_id: &str,
    file_contents: &str,
    output_path: &str,
) -> String {
    format!(
        "Implement the function body at line {}, character {} in the following {} file. \
         Write ONLY the function implementation (signature and body) to the file: {} \
         Do NOT include any other code from the source file (no imports, no other functions). \
         Do NOT output the code to stdout. \
         Output only status messages or confirmation.\n\n{}",
        line + 1,
        character + 1,
        language_id,
        output_path,
        file_contents
    )
}

pub struct AmpClient;

impl AmpClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AmpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for AmpClient {
    fn implement_function(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
    ) -> Result<String, Box<dyn Error + Sync + Send>> {
        info!(
            "Calling amp CLI - file: {}, line: {}, character: {}, language: {}",
            file_path, line, character, language_id
        );

        // NOTE: implement_function is deprecated in favor of streaming, using dummy path
        let prompt = build_prompt(line, character, language_id, file_contents, "/tmp/dummy");

        let output = Command::new("amp")
            .arg("--execute")
            .arg(&prompt)
            .arg("--stream-json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("amp CLI failed: {}", stderr).into());
        }

        let stdout = String::from_utf8(output.stdout)?;

        for line in stdout.lines().rev() {
            if let Ok(msg) = serde_json::from_str::<AmpMessage>(line) {
                if msg.msg_type == "result" {
                    if msg.is_error == Some(true) {
                        return Err(format!(
                            "amp returned error: {}",
                            msg.result.unwrap_or_default()
                        )
                        .into());
                    }
                    let result = msg.result.unwrap_or_default();
                    let result = strip_markdown_code_block(&result);
                    info!("Amp CLI returned {} bytes", result.len());
                    return Ok(result.trim().to_string());
                }
            }
        }

        Err("No result found in amp output".into())
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
            "Calling amp CLI (streaming) - file: {}, line: {}, character: {}, language: {}, function: {}",
            file_path, line, character, language_id, function_signature
        );

        // TODO: Include function_signature in the prompt for Amp as well
        let prompt = build_prompt(line, character, language_id, file_contents, output_path);

        let mut child = Command::new("amp")
            .arg("--execute")
            .arg(&prompt)
            .arg("--stream-json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let reader = BufReader::new(stdout);

        let mut accumulated_text = String::new();

        for line_result in reader.lines() {
            let line = line_result?;

            // Assume amp streams JSON objects with "content" field
            // But if it's chatting, it might just be text blocks.
            // Existing logic parsed ToolUse/ToolResult.
            // We'll keep parsing valid JSON, but ignore the "Function implementation" extraction logic
            // since we don't expect the code in stdout anymore.

            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(content) = json_val.get("content").and_then(|c| c.as_str()) {
                    accumulated_text.push_str(content);
                    on_progress(accumulated_text.trim());
                }
                // Handle tool uses if needed?
                // If amp CLI handles tool execution internally, we just see output.
            }
        }

        let status = child.wait()?;
        if !status.success() {
            return Err("amp CLI failed".into());
        }

        info!("Amp CLI finished successfully");
        Ok(())
    }
}
