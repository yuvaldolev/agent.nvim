use std::error::Error;
use std::process::{Command, Stdio};

use serde::Deserialize;
use tracing::info;

#[derive(Debug, Deserialize)]
struct AmpResult {
    #[serde(rename = "type")]
    msg_type: String,
    result: Option<String>,
    is_error: Option<bool>,
}

fn strip_markdown_code_block(s: &str) -> String {
    let trimmed = s.trim();
    
    // Check if wrapped in markdown code block
    if trimmed.starts_with("```") && trimmed.ends_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            // Skip first line (```lang) and last line (```)
            return lines[1..lines.len() - 1].join("\n");
        }
    }
    
    s.to_string()
}

pub struct AmpClient;

impl AmpClient {
    pub fn new() -> Self {
        Self
    }

    pub fn implement_function(
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

        let prompt = format!(
            "Implement the function body at line {}, character {} in the following {} file. \
             Output ONLY the raw code for the function body (the code that goes between the curly braces). \
             Do NOT include the function signature/declaration. \
             Do NOT wrap the output in markdown code blocks. \
             Do NOT include any explanations. \
             Just output the raw implementation code:\n\n{}",
            line + 1,
            character + 1,
            language_id,
            file_contents
        );

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
            if let Ok(msg) = serde_json::from_str::<AmpResult>(line) {
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
}

impl Default for AmpClient {
    fn default() -> Self {
        Self::new()
    }
}
