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
