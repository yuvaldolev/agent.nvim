use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use serde::Deserialize;
use tracing::info;

use crate::backend::Backend;
use crate::utils::strip_markdown_code_block;

/// OpenCode JSON event structure.
///
/// OpenCode outputs newline-delimited JSON events when run with `--format json`.
/// Each event has a `type` field at the top level indicating the event kind.
///
/// Example format:
/// ```json
/// {"type":"text","timestamp":...,"sessionID":"...","part":{"id":"...","type":"text","text":"content"}}
/// {"type":"step_start",...}
/// {"type":"step_finish",...}
/// ```
#[derive(Debug, Deserialize)]
struct OpenCodeEvent {
    /// Event type: "text", "step_start", "step_finish", etc.
    #[serde(rename = "type")]
    event_type: String,

    /// Part data containing the actual content
    part: Option<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    /// Part type: "text", "step-start", "step-finish", etc.
    #[serde(rename = "type")]
    part_type: Option<String>,

    /// Text content for text parts
    text: Option<String>,
}

/// Build the prompt for function implementation with OpenCode.
fn build_prompt(
    line: u32,
    character: u32,
    language_id: &str,
    file_contents: &str,
    output_path: &str,
) -> String {
    format!(
        "Implement the function body at line {}, character {} in the following file. \
         Write ONLY the function implementation (signature and body) to the file: {} \
         Do NOT include any other code from the source file (no imports, no other functions). \
         Do NOT output the code to stdout. \
         Output only status messages or confirmation.\n\n<FILE-CONTENT>\n{}</FILE-CONTENT> \n\n\
         <MUST-OBEY>\n\
        You can overwrite the output file's content, but NEVER read it, just write to it.\n\
Describe your steps before performing them.\n\
</MUST-OBEY> \
         ",
        line + 1,
        character + 1,
        output_path,
        file_contents
    )
}

pub struct OpenCodeClient;

impl OpenCodeClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenCodeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for OpenCodeClient {
    fn implement_function(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
    ) -> Result<String, Box<dyn Error + Sync + Send>> {
        info!(
            "Calling opencode CLI - file: {}, line: {}, character: {}, language: {}",
            file_path, line, character, language_id
        );

        // NOTE: implement_function is deprecated in favor of streaming, passing dummy path
        let prompt = build_prompt(line, character, language_id, file_contents, "/tmp/dummy");

        let output = Command::new("opencode")
            .arg("run")
            .arg("--format")
            .arg("json")
            .arg("--attach")
            .arg("http://localhost:1337")
            .arg(&prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("opencode CLI failed: {}", stderr).into());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let result = extract_text_from_events(&stdout)?;

        let result = strip_markdown_code_block(&result);
        info!("OpenCode CLI returned {} bytes", result.len());
        Ok(result.trim().to_string())
    }

    fn implement_function_streaming(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
        language_id: &str,
        file_contents: &str,
        output_path: &str,
        mut on_progress: Box<dyn FnMut(&str) + Send>,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        info!(
            "Calling opencode CLI (streaming) - file: {}, line: {}, character: {}, language: {}",
            file_path, line, character, language_id
        );

        let prompt = build_prompt(line, character, language_id, file_contents, output_path);

        let mut child = Command::new("opencode")
            .arg("run")
            // .arg("--format")
            // .arg("json")
            // .arg("--attach")
            // .arg("http://localhost:1337")
            .arg("--model")
            .arg("opencode/claude-sonnet-4-5")
            .arg(&prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let reader = BufReader::new(stdout);

        let mut accumulated_text = String::new();

        for line_result in reader.lines() {
            let line = line_result?;
            info!("opencode output line: {}", line);
            accumulated_text.push_str(&line);
            accumulated_text.push('\n');
            on_progress(accumulated_text.trim());

            // if let Some(text) = extract_text_from_line(&line) {
            //     accumulated_text.push_str(&text);
            //     let preview = strip_markdown_code_block(&accumulated_text);
            //     on_progress(preview.trim());
            // }
        }

        let status = child.wait()?;
        if !status.success() {
            return Err(format!("opencode CLI failed: {}", accumulated_text).into());
        }

        info!("OpenCode CLI finished successfully");
        Ok(())
    }
}

/// Extract text content from a single JSON line.
///
/// OpenCode JSON format:
/// ```json
/// {"type":"text","part":{"type":"text","text":"content"}}
/// ```
fn extract_text_from_line(line: &str) -> Option<String> {
    let event: OpenCodeEvent = serde_json::from_str(line).ok()?;

    // Only process "text" events
    if event.event_type != "text" {
        return None;
    }

    // Extract text from the part
    if let Some(part) = event.part {
        if part.part_type.as_deref() == Some("text") {
            return part.text;
        }
    }

    None
}

/// Extract all text content from newline-delimited JSON events.
fn extract_text_from_events(output: &str) -> Result<String, Box<dyn Error + Sync + Send>> {
    let mut accumulated_text = String::new();

    for line in output.lines() {
        if let Some(text) = extract_text_from_line(line) {
            accumulated_text.push_str(&text);
        }
    }

    if accumulated_text.is_empty() {
        return Err("No text content found in opencode output".into());
    }

    Ok(accumulated_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_from_line_with_text_event() {
        // Actual OpenCode JSON format
        let json = r#"{"type":"text","timestamp":1766840249580,"sessionID":"ses_abc","part":{"id":"prt_123","sessionID":"ses_abc","messageID":"msg_456","type":"text","text":"hello world"}}"#;
        assert_eq!(
            extract_text_from_line(json),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_extract_text_from_line_step_start_event() {
        // step_start events should not return text
        let json = r#"{"type":"step_start","timestamp":1766840240795,"sessionID":"ses_abc","part":{"id":"prt_123","type":"step-start","snapshot":"abc123"}}"#;
        assert_eq!(extract_text_from_line(json), None);
    }

    #[test]
    fn test_extract_text_from_line_step_finish_event() {
        // step_finish events should not return text
        let json = r#"{"type":"step_finish","timestamp":1766840249620,"sessionID":"ses_abc","part":{"id":"prt_123","type":"step-finish","reason":"stop"}}"#;
        assert_eq!(extract_text_from_line(json), None);
    }

    #[test]
    fn test_extract_text_from_line_invalid_json() {
        let json = "not valid json";
        assert_eq!(extract_text_from_line(json), None);
    }

    #[test]
    fn test_extract_text_from_events_full_session() {
        // Simulate a full OpenCode session output
        let output = r#"{"type":"step_start","timestamp":1766840240795,"sessionID":"ses_abc","part":{"id":"prt_1","type":"step-start","snapshot":"abc"}}
{"type":"text","timestamp":1766840249580,"sessionID":"ses_abc","part":{"id":"prt_2","type":"text","text":"use std::fs;\n"}}
{"type":"text","timestamp":1766840249581,"sessionID":"ses_abc","part":{"id":"prt_3","type":"text","text":"use uuid::Uuid;\n"}}
{"type":"step_finish","timestamp":1766840249620,"sessionID":"ses_abc","part":{"id":"prt_4","type":"step-finish","reason":"stop"}}
"#;
        let result = extract_text_from_events(output).unwrap();
        assert_eq!(result, "use std::fs;\nuse uuid::Uuid;\n");
    }

    #[test]
    fn test_extract_text_from_events_no_text() {
        let output = r#"{"type":"step_start","timestamp":1,"sessionID":"s","part":{"type":"step-start"}}
{"type":"step_finish","timestamp":2,"sessionID":"s","part":{"type":"step-finish"}}
"#;
        let result = extract_text_from_events(output);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_prompt() {
        let prompt = build_prompt(9, 4, "rust", "fn main() {}", "/tmp/output.rs");
        assert!(prompt.contains("line 10"));
        assert!(prompt.contains("character 5"));
        // assert!(prompt.contains("rust"));
        assert!(prompt.contains("fn main() {}"));
        assert!(prompt.contains("/tmp/output.rs"));
    }
}
