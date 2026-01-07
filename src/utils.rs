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

/// Extract a function signature for tracking purposes.
/// This is used to identify functions when line numbers may have shifted.
///
/// Supports multiple languages: Rust, C++, Python, Java, etc.
pub fn extract_function_signature(text: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if line >= lines.len() {
        return None;
    }

    // Find function start from the given line
    let start_line = find_function_start(&lines, line)?;

    // Return the line containing the function declaration
    // This is a simple identifier that should remain stable
    Some(lines[start_line].trim().to_string())
}

/// Find the start line of the function containing or at the given line.
///
/// Scans backwards from `line` to find a line with function keywords.
/// Supports: Rust (fn), C++ (void, int, etc.), Python (def), Java (public/private/void/etc.)
pub fn find_function_start(lines: &[&str], start_search_line: usize) -> Option<usize> {
    let mut current_line = start_search_line;
    if current_line >= lines.len() {
        return None;
    }

    loop {
        let line = lines[current_line].trim();

        // Check for function keywords in various languages
        // Rust: fn, pub fn, async fn, etc.
        if line.starts_with("fn ")
            || line.starts_with("pub fn ")
            || line.starts_with("async fn ")
            || line.starts_with("pub async fn ")
            || line.starts_with("pub(crate) fn ")
            || line.contains(" fn ")
        {
            return Some(current_line);
        }

        // Python: def
        if line.starts_with("def ") || line.starts_with("async def ") {
            return Some(current_line);
        }

        // C++/Java: return types and modifiers
        if line.starts_with("void ")
            || line.starts_with("int ")
            || line.starts_with("bool ")
            || line.starts_with("char ")
            || line.starts_with("float ")
            || line.starts_with("double ")
            || line.starts_with("public ")
            || line.starts_with("private ")
            || line.starts_with("protected ")
            || line.starts_with("static ")
            || line.contains("(") && (line.contains("void ") || line.contains("int "))
        {
            // Verify it looks like a function (has parentheses)
            if line.contains('(') {
                return Some(current_line);
            }
        }

        if current_line == 0 {
            break;
        }
        current_line -= 1;
    }
    None
}

/// Check if two function signatures match.
///
/// Compares trimmed versions and extracts function name for comparison.
/// Handles cases where signatures may have minor formatting differences.
fn signatures_match(found: &str, expected: &str) -> bool {
    let found = found.trim();
    let expected = expected.trim();

    // Exact match
    if found == expected {
        return true;
    }

    // Extract function names and compare
    let found_name = extract_function_name(found);
    let expected_name = extract_function_name(expected);

    if let (Some(f), Some(e)) = (found_name, expected_name) {
        return f == e;
    }

    false
}

/// Extract the function name from a signature line.
fn extract_function_name(sig: &str) -> Option<&str> {
    // Handle Rust: fn name, pub fn name, async fn name, etc.
    if let Some(pos) = sig.find(" fn ") {
        let after_fn = &sig[pos + 4..];
        return after_fn.split(&['(', '<', ' '][..]).next();
    }
    if sig.starts_with("fn ") {
        return sig[3..].split(&['(', '<', ' '][..]).next();
    }

    // Handle Python: def name, async def name
    if let Some(pos) = sig.find("def ") {
        let after_def = &sig[pos + 4..];
        return after_def.split(&['(', ' ', ':'][..]).next();
    }

    // Handle C/C++/Java: type name(
    // Look for identifier followed by (
    if let Some(paren_pos) = sig.find('(') {
        let before_paren = sig[..paren_pos].trim();
        // Get the last word before (
        return before_paren.split_whitespace().last();
    }

    None
}

/// Search forward from a line to find a function with the expected signature.
fn find_function_start_forward(
    lines: &[&str],
    start_search_line: usize,
    expected_signature: &str,
) -> Option<usize> {
    let expected_name = extract_function_name(expected_signature)?;

    for i in start_search_line..lines.len() {
        let line = lines[i].trim();

        // Check if this line looks like a function start
        if is_function_start(line) {
            if let Some(found_name) = extract_function_name(line) {
                if found_name == expected_name {
                    return Some(i);
                }
            }
        }
    }

    None
}

/// Search the entire document for a function matching the expected signature.
fn find_function_by_signature(lines: &[&str], expected_signature: &str) -> Option<usize> {
    let expected_name = extract_function_name(expected_signature)?;

    for (i, line) in lines.iter().enumerate() {
        let line = line.trim();

        if is_function_start(line) {
            if let Some(found_name) = extract_function_name(line) {
                if found_name == expected_name {
                    return Some(i);
                }
            }
        }
    }

    None
}

/// Check if a line looks like a function start.
fn is_function_start(line: &str) -> bool {
    // Rust
    if line.starts_with("fn ")
        || line.starts_with("pub fn ")
        || line.starts_with("async fn ")
        || line.starts_with("pub async fn ")
        || line.starts_with("pub(crate) fn ")
        || line.contains(" fn ")
    {
        return true;
    }

    // Python
    if line.starts_with("def ") || line.starts_with("async def ") {
        return true;
    }

    // C++/Java: return types and modifiers with parentheses
    if (line.starts_with("void ")
        || line.starts_with("int ")
        || line.starts_with("bool ")
        || line.starts_with("char ")
        || line.starts_with("float ")
        || line.starts_with("double ")
        || line.starts_with("public ")
        || line.starts_with("private ")
        || line.starts_with("protected ")
        || line.starts_with("static "))
        && line.contains('(')
    {
        return true;
    }

    false
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

/// Replace a function in the current document, handling concurrent edits.
/// Returns (new_text, start_line, end_line, lines_delta)
///
/// The `expected_signature` parameter is used to verify we found the correct function.
/// This is critical for concurrent implementations where line numbers may have shifted.
pub fn replace_function_in_document(
    current_text: &str,
    current_line: usize,
    new_implementation: &str,
    expected_signature: Option<&str>,
) -> Result<(String, u32, u32, i32), String> {
    use tracing::info;

    let lines: Vec<&str> = current_text.lines().collect();

    info!(
        "replace_function_in_document: current_line={}, expected_signature={:?}, total_lines={}",
        current_line,
        expected_signature,
        lines.len()
    );

    if current_line >= lines.len() {
        return Err("Line out of bounds".to_string());
    }

    // Find the actual function start (in case cursor is inside function)
    // First try backwards search from current_line
    let mut start_line = find_function_start(&lines, current_line);

    info!(
        "Backward search from line {} found function at line {:?}",
        current_line, start_line
    );

    // Verify we found the correct function using signature matching
    if let (Some(start), Some(expected_sig)) = (start_line, expected_signature) {
        let found_sig = lines[start].trim();
        info!(
            "Comparing found_sig='{}' with expected_sig='{}'",
            found_sig, expected_sig
        );
        // Check if the found signature matches the expected one
        // We compare trimmed versions and check for containment to handle minor differences
        if !signatures_match(found_sig, expected_sig) {
            info!("Signatures don't match! Searching forward and globally...");
            // Wrong function found! Search forward from current_line instead
            start_line = find_function_start_forward(&lines, current_line, expected_sig);
            info!("Forward search result: {:?}", start_line);
            if start_line.is_none() {
                // Try searching the entire document for the matching signature
                start_line = find_function_by_signature(&lines, expected_sig);
                info!("Global search result: {:?}", start_line);
            }
        } else {
            info!("Signatures match!");
        }
    }

    let start_line = start_line.ok_or_else(|| "Could not find function start".to_string())?;
    info!("Final start_line: {}", start_line);

    // Find the function end
    let end_line = find_function_end(&lines, start_line)
        .ok_or_else(|| "Could not find function end".to_string())?;

    // Calculate lines delta
    let old_function_lines = (end_line - start_line + 1) as i32;
    let new_function_lines = new_implementation.lines().count() as i32;
    let lines_delta = new_function_lines - old_function_lines;

    // Build new document
    let mut new_lines = Vec::new();

    // Lines before function
    new_lines.extend_from_slice(&lines[0..start_line]);

    // New implementation
    new_lines.extend(new_implementation.lines());

    // Lines after function
    if end_line + 1 < lines.len() {
        new_lines.extend_from_slice(&lines[end_line + 1..]);
    }

    let new_text = new_lines.join("\n") + "\n";

    Ok((new_text, start_line as u32, end_line as u32, lines_delta))
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
    fn test_extract_function_signature_rust() {
        let code = "fn foo(x: i32) -> i32 {\n    todo!()\n}";
        let sig = extract_function_signature(code, 0);
        assert_eq!(sig, Some("fn foo(x: i32) -> i32 {".to_string()));
    }

    #[test]
    fn test_extract_function_signature_python() {
        let code = "def calculate(a, b):\n    return a + b";
        let sig = extract_function_signature(code, 0);
        assert_eq!(sig, Some("def calculate(a, b):".to_string()));
    }

    #[test]
    fn test_extract_function_signature_cpp() {
        let code = "int add(int a, int b) {\n    return a + b;\n}";
        let sig = extract_function_signature(code, 0);
        assert_eq!(sig, Some("int add(int a, int b) {".to_string()));
    }

    #[test]
    fn test_find_function_start_rust() {
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
    fn test_find_function_start_python() {
        let code = r#"
def foo():
    x = 1
    print(x)

async def bar():
    pass
"#;
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(find_function_start(&lines, 1), Some(1)); // def foo
        assert_eq!(find_function_start(&lines, 2), Some(1)); // Inside foo
        assert_eq!(find_function_start(&lines, 5), Some(5)); // async def bar
    }

    #[test]
    fn test_find_function_start_cpp() {
        let code = r#"
int add(int a, int b) {
    return a + b;
}

void process() {
    // code
}
"#;
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(find_function_start(&lines, 1), Some(1)); // int add
        assert_eq!(find_function_start(&lines, 2), Some(1)); // Inside add
        assert_eq!(find_function_start(&lines, 5), Some(5)); // void process
    }

    #[test]
    fn test_find_function_start_java() {
        let code = r#"
public int calculate(int x) {
    return x * 2;
}

private void helper() {
    // code
}
"#;
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(find_function_start(&lines, 1), Some(1)); // public int
        assert_eq!(find_function_start(&lines, 5), Some(5)); // private void
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
    fn test_replace_function_in_document() {
        let code = "fn foo() {\n    todo!()\n}\n\nfn bar() {\n    todo!()\n}";
        let new_impl = "fn foo() {\n    println!(\"implemented\");\n}";

        let (new_text, start_line, end_line, lines_delta) =
            replace_function_in_document(code, 0, new_impl, None).unwrap();

        assert_eq!(start_line, 0);
        assert_eq!(end_line, 2);
        assert_eq!(lines_delta, 0); // 3 lines -> 3 lines
        assert!(new_text.contains("println!(\"implemented\")"));
        assert!(new_text.contains("fn bar()"));
    }

    #[test]
    fn test_replace_function_in_document_from_inside() {
        let code = "fn foo() {\n    todo!()\n}\n\nfn bar() {}";
        let new_impl = "fn foo() {\n    implemented();\n}";

        // Start from inside the function (line 1)
        let (new_text, start_line, end_line, lines_delta) =
            replace_function_in_document(code, 1, new_impl, None).unwrap();

        assert_eq!(start_line, 0); // Should find start at line 0
        assert_eq!(end_line, 2);
        assert_eq!(lines_delta, 0);
        assert!(new_text.contains("implemented()"));
    }

    #[test]
    fn test_replace_function_in_document_lines_delta() {
        let code = "fn foo() {\n    todo!()\n}";
        // New implementation has more lines
        let new_impl = "fn foo() {\n    let x = 1;\n    let y = 2;\n    x + y\n}";

        let (_, _, _, lines_delta) = replace_function_in_document(code, 0, new_impl, None).unwrap();

        // Old: 3 lines, New: 5 lines, Delta: +2
        assert_eq!(lines_delta, 2);
    }

    #[test]
    fn test_replace_function_in_document_cpp() {
        let code = "int add(int a, int b) {\n    return a + b;\n}\n\nint multiply(int a, int b) {\n    return a * b;\n}";
        let new_impl = "int add(int a, int b) {\n    int result = a + b;\n    return result;\n}";

        let (new_text, start_line, end_line, lines_delta) =
            replace_function_in_document(code, 0, new_impl, None).unwrap();

        assert_eq!(start_line, 0);
        assert_eq!(end_line, 2);
        assert_eq!(lines_delta, 1); // 3 lines -> 4 lines
        assert!(new_text.contains("int result = a + b"));
        assert!(new_text.contains("int multiply"));
    }

    #[test]
    fn test_signatures_match() {
        // Exact match
        assert!(signatures_match("fn foo() {", "fn foo() {"));

        // Same function name, different formatting
        assert!(signatures_match("fn foo() {", "fn foo(x: i32) {"));

        // Pub vs non-pub (same function name)
        assert!(signatures_match("pub fn bar() {", "fn bar() {"));

        // Different function names
        assert!(!signatures_match("fn foo() {", "fn bar() {"));

        // Python
        assert!(signatures_match("def calculate(a, b):", "def calculate():"));
        assert!(!signatures_match("def foo():", "def bar():"));

        // C++
        assert!(signatures_match("int add(int a, int b) {", "int add() {"));
        assert!(!signatures_match("int add() {", "int multiply() {"));
    }

    #[test]
    fn test_extract_function_name() {
        // Rust
        assert_eq!(extract_function_name("fn foo() {"), Some("foo"));
        assert_eq!(extract_function_name("pub fn bar() {"), Some("bar"));
        assert_eq!(extract_function_name("async fn baz() {"), Some("baz"));
        assert_eq!(extract_function_name("pub async fn qux() {"), Some("qux"));
        assert_eq!(extract_function_name("pub(crate) fn internal() {"), Some("internal"));

        // Python
        assert_eq!(extract_function_name("def calculate(a, b):"), Some("calculate"));
        assert_eq!(extract_function_name("async def fetch():"), Some("fetch"));

        // C++/Java
        assert_eq!(extract_function_name("int add(int a, int b) {"), Some("add"));
        assert_eq!(extract_function_name("void process() {"), Some("process"));
    }

    #[test]
    fn test_concurrent_replacement_with_shifted_lines() {
        // This simulates the bug scenario:
        // 1. Original document has foo() at line 0 and bar() at line 4
        // 2. foo() gets implemented, adding 10 lines (now 13 lines total)
        // 3. bar()'s adjusted line is now 14 (was 4, +10 delta)
        // 4. BUT when searching backwards from line 14, we might find foo() instead!

        // After foo() was implemented (now spans lines 0-12)
        let code_after_foo_impl = r#"fn foo() {
    // This is a much longer implementation
    let x = 1;
    let y = 2;
    let z = 3;
    let a = 4;
    let b = 5;
    let c = 6;
    let d = 7;
    let e = 8;
    let result = x + y + z + a + b + c + d + e;
    result
}

fn bar() {
    todo!()
}"#;

        let bar_impl = "fn bar() {\n    println!(\"bar implemented\");\n}";

        // Without signature matching, searching backwards from line 14 might find foo()
        // because line 14 is inside or near foo()'s implementation
        // With signature matching, we should correctly find bar()

        // The adjusted line for bar() after foo() expanded is 14
        // (originally at line 4, foo added 10 lines)
        let adjusted_line = 14;

        let (new_text, start_line, _end_line, _lines_delta) =
            replace_function_in_document(code_after_foo_impl, adjusted_line, bar_impl, Some("fn bar() {")).unwrap();

        // Key assertion: bar() should be replaced, not foo()
        // foo()'s implementation should still be intact
        assert!(new_text.contains("let result = x + y + z"));
        assert!(new_text.contains("fn bar() {\n    println!(\"bar implemented\");"));
        assert_eq!(start_line, 14); // bar() starts at line 14
    }

    #[test]
    fn test_find_function_by_signature_when_backward_fails() {
        // Scenario: The backward search from adjusted line finds the WRONG function
        // The signature matching should search forward/entire document to find the right one

        // Lines (0-indexed):
        // 0: fn first() {
        // 1:     // many
        // 2:     // lines
        // 3:     // of
        // 4:     // code
        // 5:     // here
        // 6: }
        // 7: (empty)
        // 8: fn second() {
        // 9:     todo!()
        // 10: }
        // 11: (empty)
        // 12: fn third() {
        // 13:     todo!()
        // 14: }
        let code = r#"fn first() {
    // many
    // lines
    // of
    // code
    // here
}

fn second() {
    todo!()
}

fn third() {
    todo!()
}"#;

        // If we're looking for "third" but start searching backward from line 10
        // (which is the closing brace of "second"), we'd find "second" first.
        // With signature matching, we should find "third" instead.

        let third_impl = "fn third() {\n    println!(\"third\");\n}";

        // Search from line 10, but with signature "fn third()"
        // Should find third() at line 12, not second() at line 8
        let (new_text, start_line, _, _) =
            replace_function_in_document(code, 10, third_impl, Some("fn third() {")).unwrap();

        assert_eq!(start_line, 12);
        assert!(new_text.contains("fn second() {\n    todo!()\n}"));
        assert!(new_text.contains("fn third() {\n    println!(\"third\");\n}"));
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
