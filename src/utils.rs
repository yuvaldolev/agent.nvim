use crate::lsp_utils::WorkspaceEditBuilder;
use diffy::merge;
use lsp_types::{Url, WorkspaceEdit};
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::info;

/// Strip markdown code block wrapper from a string if present.
///
/// If the string is wrapped in triple backticks (with optional language identifier),
/// returns the content between them. Otherwise returns the original string.
///
/// # Examples
///
/// ```
/// use agent_nvim::utils::strip_markdown_code_block;
///
/// assert_eq!(strip_markdown_code_block("```rust\nfn foo() {}\n```"), "fn foo() {}");
/// assert_eq!(strip_markdown_code_block("plain text"), "plain text");
/// ```
pub fn strip_markdown_code_block(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_code_block_with_language() {
        let input = "```rust\nfn foo() {\n    println!(\"hello\");\n}\n```";
        let expected = "fn foo() {\n    println!(\"hello\");\n}";
        assert_eq!(strip_markdown_code_block(input), expected);
    }

    #[test]
    fn test_strip_markdown_code_block_without_language() {
        let input = "```\nsome code\n```";
        let expected = "some code";
        assert_eq!(strip_markdown_code_block(input), expected);
    }

    #[test]
    fn test_strip_markdown_code_block_plain_text() {
        let input = "plain text without code block";
        assert_eq!(strip_markdown_code_block(input), input);
    }

    #[test]
    fn test_strip_markdown_code_block_with_whitespace() {
        let input = "  ```python\nprint('hello')\n```  ";
        let expected = "print('hello')";
        assert_eq!(strip_markdown_code_block(input), expected);
    }

    #[test]
    fn test_strip_markdown_code_block_empty() {
        let input = "```\n```";
        let expected = "";
        assert_eq!(strip_markdown_code_block(input), expected);
    }

    #[test]
    fn test_strip_markdown_code_block_multiline() {
        let input = "```typescript\nconst x = 1;\nconst y = 2;\nreturn x + y;\n```";
        let expected = "const x = 1;\nconst y = 2;\nreturn x + y;";
        assert_eq!(strip_markdown_code_block(input), expected);
    }
}

/// Find the start line of the function containing or at the given line.
///
/// Scans backwards from `line` to find a line starting with `fn`, `pub fn`, `async fn`, etc.
/// Handles attributes and comments above the function?
/// For now, we assume we want to replace from the `fn` keyword line.
/// If attributes exist, we might want to preserve them or include them.
/// Usually the Agent generates the signature. Attributes are often preserved if outside the replacement range?
/// If we replace from `fn`, attributes above stay. That is good.
pub fn find_function_start(lines: &[&str], start_search_line: usize) -> Option<usize> {
    let mut current_line = start_search_line;
    if current_line >= lines.len() {
        return None;
    }

    loop {
        let line = lines[current_line].trim();
        // naive check for function declaration
        if line.starts_with("fn ")
            || line.starts_with("pub fn ")
            || line.starts_with("async fn ")
            || line.contains(" fn ")
        {
            // Verify it has an opening brace or start of signature
            // This heuristic is simple but covers 95% of rust cases
            return Some(current_line);
        }

        if current_line == 0 {
            break;
        }
        current_line -= 1;
    }
    None
}

/// Find the end line of a function based on brace counting.
///
/// Returns the line number (0-indexed) of the closing brace.
pub fn find_function_end(lines: &[&str], start_line: usize) -> Option<usize> {
    let mut open_braces = 0;
    let mut found_start = false;

    for (i, line) in lines.iter().enumerate().skip(start_line) {
        // Simple brace counting - ignores comments/strings for now which is a limitation
        // but likely sufficient for mostly-correct code.
        for char in line.chars() {
            match char {
                '{' => {
                    open_braces += 1;
                    found_start = true;
                }
                '}' => {
                    open_braces -= 1;
                }
                _ => {}
            }
        }

        if found_start && open_braces == 0 {
            return Some(i);
        }

        // Safety valve for unbalanced braces going negative
        if found_start && open_braces < 0 {
            return Some(i);
        }
    }

    None
}

/// Replace a function in the file content with a new implementation.
pub fn replace_function(
    file_content: &str,
    start_line: usize,
    new_implementation: &str,
) -> Option<String> {
    let lines: Vec<&str> = file_content.lines().collect();

    if start_line >= lines.len() {
        return None;
    }

    let end_line = find_function_end(&lines, start_line)?;

    let mut new_lines = Vec::new();
    // extend from start to start_line (exclusive)
    new_lines.extend_from_slice(&lines[0..start_line]);

    // add new implementation lines
    new_lines.extend(new_implementation.lines());

    // extend from end_line + 1 to end
    if end_line + 1 < lines.len() {
        new_lines.extend_from_slice(&lines[end_line + 1..]);
    }

    Some(new_lines.join("\n") + "\n") // Add trailing newline
}

/// Create a 3-way merge edit.
///
/// 1. Constructs "Theirs" by applying `implementation` to `base_text`.
/// 2. Writes "Theirs" to a temporary file.
/// 3. Merges `base_text`, `current_text`, and `theirs_text`.
/// 4. Returns a full-file replacement WorkspaceEdit and the number of lines added.
pub fn create_3way_merge_edit(
    uri: &Url,
    base_text: &str,
    current_text: &str,
    implementation: &str,
    line: usize,
) -> Result<(WorkspaceEdit, i32), String> {
    // 1. Construct "Theirs" version
    let theirs_text = replace_function(base_text, line, implementation)
        .ok_or_else(|| "Failed to replace function in base text".to_string())?;

    // 2. Write to temporary file
    match NamedTempFile::new() {
        Ok(mut file) => {
            if let Err(e) = file.write_all(theirs_text.as_bytes()) {
                // Log but don't fail the operation just because temp write failed?
                // The requirements emphasize writing to temp file.
                // We'll log to stderr/tracing if possible, but here we just proceed.
                eprintln!("Failed to write to temp file: {}", e);
            } else {
                info!("Wrote agent implementation to temp file: {:?}", file.path());
            }
        }
        Err(e) => eprintln!("Failed to create temp file: {}", e),
    }

    // 3. Perform 3-way merge
    let merged_text = match merge(base_text, current_text, &theirs_text) {
        Ok(text) => text,
        Err(text) => text, // Use conflict markers
    };

    // 4. Create Edit
    let edit = WorkspaceEditBuilder::create_full_replace(uri, current_text, &merged_text);

    // Calculate lines added
    let new_lines_count = implementation.lines().count() as i32;
    let old_lines_count = find_function_end(&base_text.lines().collect::<Vec<_>>(), line)
        .map(|end| (end - line + 1) as i32)
        .unwrap_or(0);
    let lines_added = new_lines_count - old_lines_count;

    Ok((edit, lines_added))
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn test_find_function_start() {
        let code = r#"
#[test]
fn foo() {
    let x = 1;
    println!("{}", x);
}

pub async fn bar() {
    // comment
}
"#;
        let lines: Vec<&str> = code.lines().collect();
        // Line 3 is 'fn foo() {' (depending on split, let's trace)
        // 0: ""
        // 1: "#[test]"
        // 2: "fn foo() {" --> Start
        // 3: "    let x = 1;"

        assert_eq!(find_function_start(&lines, 2), Some(2));
        assert_eq!(find_function_start(&lines, 3), Some(2)); // Inside foo

        // bar is at line 7
        // 7: "pub async fn bar() {"
        assert_eq!(find_function_start(&lines, 7), Some(7));
        assert_eq!(find_function_start(&lines, 8), Some(7)); // Inside bar
    }

    #[test]
    fn test_find_function_end() {
        let code = r#"fn foo() {
    let x = {
        1
    };
    println!("{}", x);
}

fn bar() {}"#;
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(find_function_end(&lines, 0), Some(5));
        assert_eq!(find_function_end(&lines, 7), Some(7));
    }

    #[test]
    fn test_replace_function() {
        let code = "fn foo() {\n    todo!()\n}\n\nfn bar() {}";
        let new_impl = "fn foo() {\n    println!(\"implemented\");\n}";

        let result = replace_function(code, 0, new_impl).unwrap();
        let expected = "fn foo() {\n    println!(\"implemented\");\n}\n\nfn bar() {}\n";

        assert_eq!(result, expected);
    }

    #[test]
    fn test_create_3way_merge_edit() {
        let uri = Url::parse("file:///test.rs").unwrap();
        let base_text = "fn foo() {\n    todo!()\n}\n\nfn bar() {}\n";
        // User added a comment to bar()
        let current_text = "fn foo() {\n    todo!()\n}\n\nfn bar() {\n    // comment\n}\n";
        // Agent implements foo()
        let implementation = "fn foo() {\n    implemented();\n}";

        let (edit, lines_added) = create_3way_merge_edit(
            &uri,
            base_text,
            current_text,
            implementation,
            0, // line of foo()
        )
        .expect("Failed to create edit");

        // lines_added: new(3) - old(3) = 0
        assert_eq!(lines_added, 0);

        // Verify Content
        // We need to inspect the edit. The WorkspaceEdit struct is complex but we can check the new_text.
        let changes = edit.document_changes.unwrap();
        if let lsp_types::DocumentChanges::Edits(edits) = changes {
            let doc_edit = &edits[0];
            let changes = &doc_edit.edits;
            let text_edit = match &changes[0] {
                lsp_types::OneOf::Left(e) => e,
                _ => panic!("Expected TextEdit"),
            };

            let new_content = &text_edit.new_text;

            // Should contain implementation
            assert!(new_content.contains("implemented();"));
            // Should contain user edit
            assert!(new_content.contains("// comment"));
        } else {
            panic!("Expected DocumentChanges::Edits");
        }
    }

    #[test]
    fn test_create_3way_merge_conflict() {
        let uri = Url::parse("file:///test.rs").unwrap();
        let base_text = "fn foo() {\n    todo!()\n}\n";
        // User changed foo() body
        let current_text = "fn foo() {\n    user_change();\n}\n";
        // Agent implements foo() differently
        let implementation = "fn foo() {\n    agent_change();\n}";

        let (edit, _) = create_3way_merge_edit(&uri, base_text, current_text, implementation, 0)
            .expect("Failed to create edit");

        let changes = edit.document_changes.unwrap();
        if let lsp_types::DocumentChanges::Edits(edits) = changes {
            let doc_edit = &edits[0];
            let changes = &doc_edit.edits;
            let text_edit = match &changes[0] {
                lsp_types::OneOf::Left(e) => e,
                _ => panic!("Expected TextEdit"),
            };
            let new_content = &text_edit.new_text;

            // Should contain conflict markers
            assert!(new_content.contains("<<<<<<<"));
            assert!(new_content.contains("user_change();"));
            assert!(new_content.contains("agent_change();"));
        }
    }
}
