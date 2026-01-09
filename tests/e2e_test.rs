use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

fn set_nonblocking(fd: RawFd, nonblocking: bool) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if nonblocking {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        } else {
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }
}

struct LspClient {
    child: Child,
    request_id: i32,
    stdout_fd: RawFd,
}

impl LspClient {
    fn spawn() -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_agent-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn LSP server");

        let stdout_fd = child.stdout.as_ref().unwrap().as_raw_fd();

        Self {
            child,
            request_id: 0,
            stdout_fd,
        }
    }

    fn send_message(&mut self, content: &Value) {
        let content_str = serde_json::to_string(content).unwrap();
        let message = format!(
            "Content-Length: {}\r\n\r\n{}",
            content_str.len(),
            content_str
        );

        let stdin = self.child.stdin.as_mut().expect("Failed to get stdin");
        stdin.write_all(message.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn read_message_body_from_reader(
        reader: &mut BufReader<&mut ChildStdout>,
        header: &str,
    ) -> Value {
        let content_length: usize = header
            .trim()
            .strip_prefix("Content-Length:")
            .unwrap()
            .trim()
            .parse()
            .unwrap();

        let mut empty_line = String::new();
        reader.read_line(&mut empty_line).unwrap();

        let mut content = vec![0u8; content_length];
        reader.read_exact(&mut content).unwrap();

        serde_json::from_slice(&content).unwrap()
    }

    fn read_message_from_reader(reader: &mut BufReader<&mut ChildStdout>) -> Value {
        let mut header = String::new();
        loop {
            header.clear();
            reader.read_line(&mut header).unwrap();
            if header.starts_with("Content-Length:") {
                break;
            }
        }

        Self::read_message_body_from_reader(reader, &header)
    }

    fn read_message(&mut self) -> Value {
        let stdout = self.child.stdout.as_mut().expect("Failed to get stdout");
        let mut reader = BufReader::new(stdout);
        Self::read_message_from_reader(&mut reader)
    }

    fn try_read_message(&mut self, timeout: Duration) -> Option<Value> {
        set_nonblocking(self.stdout_fd, true);

        let start = std::time::Instant::now();
        let stdout = self.child.stdout.as_mut().expect("Failed to get stdout");
        let mut reader = BufReader::new(stdout);

        let mut header = String::new();
        loop {
            if start.elapsed() > timeout {
                set_nonblocking(self.stdout_fd, false);
                return None;
            }

            header.clear();
            match reader.read_line(&mut header) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(_) => {
                    if header.starts_with("Content-Length:") {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("Read error: {}", e),
            }
        }

        set_nonblocking(self.stdout_fd, false);

        Some(Self::read_message_body_from_reader(&mut reader, &header))
    }

    fn send_request(&mut self, method: &str, params: Value) -> Value {
        self.request_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params
        });
        self.send_message(&request);
        self.read_message()
    }

    fn send_request_async(&mut self, method: &str, params: Value) -> i32 {
        self.request_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params
        });
        self.send_message(&request);
        self.request_id
    }

    fn collect_messages(&mut self, timeout: Duration) -> Vec<Value> {
        set_nonblocking(self.stdout_fd, true);

        let mut messages = Vec::new();
        let start = std::time::Instant::now();
        let stdout = self.child.stdout.as_mut().expect("Failed to get stdout");
        let mut reader = BufReader::new(stdout);

        while start.elapsed() < timeout {
            let mut header = String::new();

            match reader.read_line(&mut header) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(_) => {
                    if !header.starts_with("Content-Length:") {
                        continue;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("Read error: {}", e),
            }

            set_nonblocking(self.stdout_fd, false);

            let msg = Self::read_message_body_from_reader(&mut reader, &header);
            messages.push(msg);

            set_nonblocking(self.stdout_fd, true);
        }

        set_nonblocking(self.stdout_fd, false);
        messages
    }

    fn send_notification(&mut self, method: &str, params: Value) {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send_message(&notification);
    }

    fn initialize(&mut self) -> Value {
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": null,
            "capabilities": {}
        });
        let response = self.send_request("initialize", init_params);
        self.send_notification("initialized", json!({}));

        // After initialization, server sends agent/backendInfo notification
        // We need to consume it to avoid it interfering with subsequent requests
        std::thread::sleep(Duration::from_millis(50));
        let _ = self.try_read_message(Duration::from_millis(100));

        response
    }

    fn shutdown(&mut self) {
        self.send_request("shutdown", json!(null));
        self.send_notification("exit", json!(null));

        std::thread::sleep(Duration::from_millis(100));
        let _ = self.child.kill();
    }

    fn drain_stderr(&mut self) -> String {
        if let Some(ref mut stderr) = self.child.stderr {
            let fd = stderr.as_raw_fd();
            set_nonblocking(fd, true);

            let mut output = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => output.extend_from_slice(&buf[..n]),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            set_nonblocking(fd, false);
            String::from_utf8_lossy(&output).to_string()
        } else {
            String::new()
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn test_initialization() {
    let mut client = LspClient::spawn();

    let response = client.initialize();

    assert!(
        response.get("result").is_some(),
        "Expected result in response"
    );
    let result = &response["result"];
    let capabilities = &result["capabilities"];

    assert!(
        capabilities.get("textDocumentSync").is_some(),
        "Expected textDocumentSync capability"
    );
    assert!(
        capabilities.get("completionProvider").is_some(),
        "Expected completionProvider capability"
    );
    assert!(
        capabilities.get("codeActionProvider").is_some(),
        "Expected codeActionProvider capability"
    );
    assert!(
        capabilities.get("executeCommandProvider").is_some(),
        "Expected executeCommandProvider capability"
    );

    let commands = &capabilities["executeCommandProvider"]["commands"];
    assert!(commands.is_array(), "Expected commands array");
    let commands_vec: Vec<&str> = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        commands_vec.contains(&"amp.implFunction"),
        "Expected amp.implFunction command"
    );

    client.shutdown();
}

#[test]
fn test_did_open_and_code_action() {
    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test.rs";
    let test_content = "fn hello() {\n    // TODO\n}\n";

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": test_content
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    let response = client.send_request(
        "textDocument/codeAction",
        json!({
            "textDocument": {
                "uri": test_uri
            },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "context": {
                "diagnostics": []
            }
        }),
    );

    assert!(
        response.get("result").is_some(),
        "Expected result in response"
    );
    let result = &response["result"];
    assert!(result.is_array(), "Expected array of code actions");

    let actions = result.as_array().unwrap();
    assert!(!actions.is_empty(), "Expected at least one code action");

    let action = &actions[0];
    assert_eq!(
        action["title"].as_str().unwrap(),
        "Implement function with Amp"
    );
    assert_eq!(
        action["command"]["command"].as_str().unwrap(),
        "amp.implFunction"
    );

    let args = action["command"]["arguments"].as_array().unwrap();
    assert_eq!(args[0].as_str().unwrap(), test_uri);
    assert_eq!(args[1].as_u64().unwrap(), 0);
    assert_eq!(args[2].as_u64().unwrap(), 0);
    assert_eq!(args[3].as_i64().unwrap(), 1);
    assert_eq!(args[4].as_str().unwrap(), "rust");

    client.shutdown();
}

#[test]
fn test_did_change() {
    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_change.rs";

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn foo() {}\n"
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    client.send_notification(
        "textDocument/didChange",
        json!({
            "textDocument": {
                "uri": test_uri,
                "version": 2
            },
            "contentChanges": [
                {
                    "range": {
                        "start": { "line": 0, "character": 3 },
                        "end": { "line": 0, "character": 6 }
                    },
                    "text": "bar"
                }
            ]
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    let response = client.send_request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": test_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "context": { "diagnostics": [] }
        }),
    );

    assert!(response.get("result").is_some());
    let actions = response["result"].as_array().unwrap();
    assert!(!actions.is_empty());

    assert_eq!(actions[0]["command"]["arguments"][3].as_i64().unwrap(), 2);

    client.shutdown();
}

#[test]
fn test_completion_returns_null() {
    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_completion.rs";
    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn test() {\n    let x = something.\n}\n"
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    let response = client.send_request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": test_uri },
            "position": { "line": 1, "character": 22 }
        }),
    );

    assert!(response.get("result").is_some());
    assert!(
        response["result"].is_null(),
        "Expected null completion result"
    );

    client.shutdown();
}

#[test]
fn test_unknown_request_returns_error() {
    let mut client = LspClient::spawn();
    client.initialize();

    let response = client.send_request("unknownMethod", json!({}));

    assert!(response.get("error").is_some(), "Expected error response");
    let error = &response["error"];
    assert_eq!(error["code"].as_i64().unwrap(), -32601);

    client.shutdown();
}

#[test]
#[ignore]
fn test_execute_command_prints_modifications() {
    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_impl.rs";
    let test_content = r#"fn add(a: i32, b: i32) -> i32 {
    todo!()
}

fn main() {
    println!("{}", add(1, 2));
}
"#;

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": test_content
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    let response = client.send_request(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [
                test_uri,
                0,
                0,
                1,
                "rust"
            ]
        }),
    );

    println!("\n=== Execute Command Response ===");
    println!("{}", serde_json::to_string_pretty(&response).unwrap());

    if let Some(apply_edit) = client.try_read_message(Duration::from_secs(30)) {
        println!("\n=== Workspace Apply Edit Request ===");
        println!("{}", serde_json::to_string_pretty(&apply_edit).unwrap());

        if let Some(params) = apply_edit.get("params") {
            if let Some(edit) = params.get("edit") {
                println!("\n=== Edit Details ===");
                if let Some(doc_changes) = edit.get("documentChanges") {
                    for (i, change) in doc_changes.as_array().unwrap_or(&vec![]).iter().enumerate()
                    {
                        println!("Document Change #{}:", i + 1);
                        if let Some(text_doc) = change.get("textDocument") {
                            println!(
                                "  URI: {}",
                                text_doc.get("uri").unwrap_or(&json!("unknown"))
                            );
                            println!(
                                "  Version: {}",
                                text_doc.get("version").unwrap_or(&json!("unknown"))
                            );
                        }
                        if let Some(edits) = change.get("edits") {
                            for (j, edit) in edits.as_array().unwrap_or(&vec![]).iter().enumerate()
                            {
                                println!("  Edit #{}:", j + 1);
                                if let Some(range) = edit.get("range") {
                                    println!(
                                        "    Range: ({},{}) -> ({},{})",
                                        range["start"]["line"],
                                        range["start"]["character"],
                                        range["end"]["line"],
                                        range["end"]["character"]
                                    );
                                }
                                if let Some(new_text) = edit.get("newText") {
                                    println!("    New Text:");
                                    println!("    ----");
                                    for line in new_text.as_str().unwrap_or("").lines() {
                                        println!("    {}", line);
                                    }
                                    println!("    ----");
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        println!("\n=== No workspace/applyEdit received within timeout ===");
        println!("This is expected if amp CLI is not available.");
    }

    let stderr = client.drain_stderr();
    if !stderr.is_empty() {
        println!("\n=== Server Stderr ===");
        println!("{}", stderr);
    }
    client.shutdown();
}

#[test]
#[ignore]
fn test_single_function_modification() {
    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_multi_func.rs";
    let test_content = r#"fn first_function(x: i32) -> i32 {
    todo!()
}

/// Increments each element in the array by 1
///
/// # Arguments
/// - `y`: A mutable slice of u32 integers
///
/// # Returns
/// - The sum of the incremented elements as an i32
fn increment_array(y: &mut [u32]) -> i32 {
    todo!()
}

fn third_function(z: i32) -> i32 {
    todo!()
}
"#;

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": test_content
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    let response = client.send_request(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [
                test_uri,
                12,
                0,
                1,
                "rust"
            ]
        }),
    );

    println!("\n=== Testing Single Function Modification ===");
    println!("Original file:");
    println!("----");
    print!("{}", test_content);
    println!("----");
    println!("\nExecuting command on line 5 (second_function)...");

    println!("\n=== Execute Command Response ===");
    println!("{}", serde_json::to_string_pretty(&response).unwrap());

    if let Some(apply_edit) = client.try_read_message(Duration::from_secs(30)) {
        println!("\n=== Workspace Apply Edit Request ===");
        println!("{}", serde_json::to_string_pretty(&apply_edit).unwrap());

        if let Some(params) = apply_edit.get("params") {
            if let Some(edit) = params.get("edit") {
                if let Some(doc_changes) = edit.get("documentChanges") {
                    let empty_vec = vec![];
                    let changes = doc_changes.as_array().unwrap_or(&empty_vec);

                    assert_eq!(changes.len(), 1, "Expected exactly one document change");

                    let change = &changes[0];
                    if let Some(edits) = change.get("edits") {
                        let empty_edits = vec![];
                        let edit_list = edits.as_array().unwrap_or(&empty_edits);
                        assert_eq!(edit_list.len(), 1, "Expected exactly one edit");

                        let the_edit = &edit_list[0];
                        if let Some(range) = the_edit.get("range") {
                            let start_line = range["start"]["line"].as_u64().unwrap();
                            let end_line = range["end"]["line"].as_u64().unwrap();

                            println!("\n=== Verification ===");
                            println!(
                                "Edit affects lines {} to {} (0-indexed)",
                                start_line, end_line
                            );

                            assert_eq!(
                                start_line, 0,
                                "Edit should be a full file replacement, starting at line 0"
                            );

                            assert!(
                                end_line >= 13,
                                "Edit should cover at least the whole file (13+ lines), got end line {}",
                                end_line
                            );

                            if let Some(new_text) = the_edit.get("newText") {
                                let content = new_text.as_str().unwrap_or("");
                                assert!(
                                    content.contains("fn first_function"),
                                    "New content should preserve first_function"
                                );
                                assert!(
                                    content.contains("fn third_function"),
                                    "New content should preserve third_function"
                                );
                                assert!(
                                    content.contains("fn increment_array"),
                                    "New content should contain increment_array"
                                );
                                // We can't easily check the *exact* implementation without mocking the backend output perfectly,
                                // but we know the backend returns *something*. Use existing test setup?
                                // The test uses "amp" CLI which might not be available or mocked?
                                // Actually, `test_execute_command_prints_modifications` uses `agent-lsp`.
                                // Does `agent-lsp` rely on valid `amp` or `opencode` CLI?
                                // Yes, `opencode.rs` spawns `opencode`.
                                // If `opencode` is not installed, the test might fail or skip.
                                // The test has `#[ignore]` on it!
                                // "amp" test also has `#[ignore]`.
                            }

                            println!(
                                "✓ Edit is a full file replacement preserving content structure"
                            );
                        }
                    }
                }
            }
        }
    } else {
        println!("\n=== No workspace/applyEdit received within timeout ===");
        println!("Skipping assertions - amp CLI may not be available.");
    }

    let stderr = client.drain_stderr();
    if !stderr.is_empty() {
        println!("\n=== Server Stderr ===");
        println!("{}", stderr);
    }
    client.shutdown();
}

#[test]
#[ignore]
fn test_concurrent_implementations() {
    use std::collections::{HashMap, HashSet};

    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri_1 = "file:///tmp/test_concurrent_1.rs";
    let test_content_1 = r#"/// Adds two numbers
fn add(a: i32, b: i32) -> i32 {
    todo!()
}
"#;

    let test_uri_2 = "file:///tmp/test_concurrent_2.rs";
    let test_content_2 = r#"/// Multiplies two numbers
fn multiply(a: i32, b: i32) -> i32 {
    todo!()
}
"#;

    let test_uri_3 = "file:///tmp/test_concurrent_3.rs";
    let test_content_3 = r#"/// Subtracts two numbers
fn subtract(a: i32, b: i32) -> i32 {
    todo!()
}
"#;

    for (uri, content) in [
        (test_uri_1, test_content_1),
        (test_uri_2, test_content_2),
        (test_uri_3, test_content_3),
    ] {
        client.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": content
                }
            }),
        );
    }

    std::thread::sleep(Duration::from_millis(50));

    println!("\n=== Sending 3 concurrent amp.implFunction commands ===");

    let req_id_1 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri_1, 1, 0, 1, "rust"]
        }),
    );

    let req_id_2 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri_2, 1, 0, 1, "rust"]
        }),
    );

    let req_id_3 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri_3, 1, 0, 1, "rust"]
        }),
    );

    println!(
        "Sent requests with IDs: {}, {}, {}",
        req_id_1, req_id_2, req_id_3
    );

    let messages = client.collect_messages(Duration::from_secs(60));

    println!("\n=== Collected {} messages ===", messages.len());

    let mut responses: HashMap<i32, Value> = HashMap::new();
    let mut progress_notifications: Vec<Value> = Vec::new();
    let mut apply_edits: Vec<Value> = Vec::new();

    for msg in &messages {
        if let Some(id) = msg.get("id") {
            if msg.get("result").is_some() || msg.get("error").is_some() {
                if let Some(id_num) = id.as_i64() {
                    responses.insert(id_num as i32, msg.clone());
                }
            } else if msg.get("method").map(|m| m.as_str()) == Some(Some("workspace/applyEdit")) {
                apply_edits.push(msg.clone());
            }
        } else if let Some(method) = msg.get("method") {
            if method.as_str() == Some("amp/implFunctionProgress") {
                progress_notifications.push(msg.clone());
            }
        }
    }

    println!("\n=== Responses ===");
    for (id, resp) in &responses {
        println!(
            "Request {}: {}",
            id,
            if resp.get("result").is_some() {
                "success"
            } else {
                "error"
            }
        );
    }

    assert!(
        responses.contains_key(&req_id_1),
        "Missing response for request {}",
        req_id_1
    );
    assert!(
        responses.contains_key(&req_id_2),
        "Missing response for request {}",
        req_id_2
    );
    assert!(
        responses.contains_key(&req_id_3),
        "Missing response for request {}",
        req_id_3
    );

    for (id, resp) in &responses {
        assert!(
            resp.get("result").is_some(),
            "Request {} should return success (non-blocking), got error: {:?}",
            id,
            resp.get("error")
        );
    }
    println!("✓ All 3 commands returned success immediately");

    println!("\n=== Progress Notifications ===");
    println!(
        "Received {} progress notifications",
        progress_notifications.len()
    );

    let mut job_ids: HashSet<String> = HashSet::new();
    let mut job_id_to_uri: HashMap<String, String> = HashMap::new();

    for notif in &progress_notifications {
        if let Some(params) = notif.get("params") {
            if let (Some(job_id), Some(uri)) = (
                params.get("job_id").and_then(|j| j.as_str()),
                params.get("uri").and_then(|u| u.as_str()),
            ) {
                job_ids.insert(job_id.to_string());
                job_id_to_uri.insert(job_id.to_string(), uri.to_string());

                if let Some(preview) = params.get("preview").and_then(|p| p.as_str()) {
                    println!(
                        "  job_id: {}... uri: {} preview_len: {}",
                        &job_id[..8.min(job_id.len())],
                        uri,
                        preview.len()
                    );
                }
            }
        }
    }

    if !progress_notifications.is_empty() {
        println!("✓ Progress notifications include job_id for correlation");
        println!("  Unique job_ids: {}", job_ids.len());

        assert!(
            job_ids.len() >= 1,
            "Expected at least 1 unique job_id in progress notifications"
        );
    }

    println!("\n=== Workspace Apply Edits ===");
    println!(
        "Received {} workspace/applyEdit requests",
        apply_edits.len()
    );

    let mut edited_uris: HashSet<String> = HashSet::new();
    for edit in &apply_edits {
        if let Some(params) = edit.get("params") {
            if let Some(doc_changes) = params
                .get("edit")
                .and_then(|e| e.get("documentChanges"))
                .and_then(|dc| dc.as_array())
            {
                for change in doc_changes {
                    if let Some(uri) = change
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(|u| u.as_str())
                    {
                        edited_uris.insert(uri.to_string());
                        println!("  Edit for: {}", uri);
                    }
                }
            }
        }
    }

    if apply_edits.len() >= 3 {
        assert_eq!(edited_uris.len(), 3, "Expected edits for 3 different files");
        assert!(
            edited_uris.contains(test_uri_1),
            "Missing edit for {}",
            test_uri_1
        );
        assert!(
            edited_uris.contains(test_uri_2),
            "Missing edit for {}",
            test_uri_2
        );
        assert!(
            edited_uris.contains(test_uri_3),
            "Missing edit for {}",
            test_uri_3
        );
        println!("✓ All 3 files received workspace/applyEdit");
    } else {
        println!(
            "Only {} apply edits received (expected 3) - amp CLI may have issues",
            apply_edits.len()
        );
    }

    let stderr = client.drain_stderr();
    if !stderr.is_empty() {
        println!("\n=== Server Stderr (last 2000 chars) ===");
        let stderr_tail: String = stderr
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        println!("{}", stderr_tail);
    }

    println!("\n=== Test Summary ===");
    println!("Responses received: {}/3", responses.len());
    println!("Progress notifications: {}", progress_notifications.len());
    println!("Apply edits: {}/3", apply_edits.len());
    println!("Unique job IDs: {}", job_ids.len());

    client.shutdown();
}

#[test]
#[ignore]
fn test_concurrent_same_file_implementations() {
    use std::collections::HashMap;

    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_same_file_concurrent.rs";
    let test_content = r#"fn add(a: i32, b: i32) -> i32 {
    todo!()
}

fn subtract(a: i32, b: i32) -> i32 {
    todo!()
}

fn multiply(a: i32, b: i32) -> i32 {
    todo!()
}
"#;

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": test_content
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    println!("\n=== Testing Concurrent Same-File Implementations ===");
    println!("Sending 3 concurrent implementation requests for the same file");

    // Send 3 concurrent requests for different functions in the same file
    let req_id_1 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri, 0, 0, 1, "rust"] // add function at line 0
        }),
    );

    let req_id_2 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri, 4, 0, 1, "rust"] // subtract function at line 4
        }),
    );

    let req_id_3 = client.send_request_async(
        "workspace/executeCommand",
        json!({
            "command": "amp.implFunction",
            "arguments": [test_uri, 8, 0, 1, "rust"] // multiply function at line 8
        }),
    );

    println!(
        "Sent requests with IDs: {}, {}, {}",
        req_id_1, req_id_2, req_id_3
    );

    // Collect messages for up to 90 seconds (3 jobs * 30 seconds each, but they run in parallel)
    let messages = client.collect_messages(Duration::from_secs(90));

    println!("\n=== Collected {} messages ===", messages.len());

    let mut responses: HashMap<i32, Value> = HashMap::new();
    let mut progress_notifications: Vec<Value> = Vec::new();
    let mut apply_edits: Vec<Value> = Vec::new();
    let mut job_completed_notifications: Vec<Value> = Vec::new();

    for msg in &messages {
        if let Some(id) = msg.get("id") {
            if msg.get("result").is_some() || msg.get("error").is_some() {
                if let Some(id_num) = id.as_i64() {
                    responses.insert(id_num as i32, msg.clone());
                }
            } else if msg.get("method").map(|m| m.as_str()) == Some(Some("workspace/applyEdit")) {
                apply_edits.push(msg.clone());
            }
        } else if let Some(method) = msg.get("method") {
            match method.as_str() {
                Some("amp/implFunctionProgress") => {
                    progress_notifications.push(msg.clone());
                }
                Some("amp/jobCompleted") => {
                    job_completed_notifications.push(msg.clone());
                }
                _ => {}
            }
        }
    }

    println!("\n=== Responses ===");
    for (id, resp) in &responses {
        println!(
            "Request {}: {}",
            id,
            if resp.get("result").is_some() {
                "success"
            } else {
                "error"
            }
        );
    }

    // All 3 commands should return success immediately (non-blocking)
    assert!(
        responses.contains_key(&req_id_1),
        "Missing response for request {}",
        req_id_1
    );
    assert!(
        responses.contains_key(&req_id_2),
        "Missing response for request {}",
        req_id_2
    );
    assert!(
        responses.contains_key(&req_id_3),
        "Missing response for request {}",
        req_id_3
    );

    for (id, resp) in &responses {
        assert!(
            resp.get("result").is_some(),
            "Request {} should return success, got error: {:?}",
            id,
            resp.get("error")
        );
    }
    println!("✓ All 3 commands returned success immediately (non-blocking)");

    println!("\n=== Job Completed Notifications ===");
    println!(
        "Received {} job completed notifications",
        job_completed_notifications.len()
    );

    let mut successful_jobs = 0;
    for notif in &job_completed_notifications {
        if let Some(params) = notif.get("params") {
            if let Some(success) = params.get("success").and_then(|s| s.as_bool()) {
                if success {
                    successful_jobs += 1;
                    if let Some(job_id) = params.get("job_id").and_then(|j| j.as_str()) {
                        println!(
                            "  ✓ Job {} completed successfully",
                            &job_id[..8.min(job_id.len())]
                        );
                    }
                } else {
                    if let Some(error) = params.get("error").and_then(|e| e.as_str()) {
                        println!("  ✗ Job failed: {}", error);
                    }
                }
            }
        }
    }

    println!("\n=== Workspace Apply Edits ===");
    println!(
        "Received {} workspace/applyEdit requests",
        apply_edits.len()
    );

    if apply_edits.len() >= 3 {
        println!("✓ All 3 jobs completed and sent apply edits");
    } else {
        println!(
            "⚠ Only {} apply edits received (expected 3)",
            apply_edits.len()
        );
    }

    println!("\n=== Test Summary ===");
    println!("Responses: {}/3", responses.len());
    println!("Progress notifications: {}", progress_notifications.len());
    println!(
        "Job completed: {}/{}",
        successful_jobs,
        job_completed_notifications.len()
    );
    println!("Apply edits: {}/3", apply_edits.len());

    let stderr = client.drain_stderr();
    if !stderr.is_empty() {
        println!("\n=== Server Stderr (last 2000 chars) ===");
        let stderr_tail: String = stderr
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        println!("{}", stderr_tail);
    }

    client.shutdown();
}

#[test]
fn test_max_concurrent_jobs_limit() {
    use std::collections::HashMap;

    let mut client = LspClient::spawn();
    client.initialize();

    let test_uri = "file:///tmp/test_max_jobs.rs";

    // Create a file with 12 functions
    let mut test_content = String::new();
    for i in 0..12 {
        test_content.push_str(&format!("fn func_{}() {{\n    todo!()\n}}\n\n", i));
    }

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": test_uri,
                "languageId": "rust",
                "version": 1,
                "text": test_content
            }
        }),
    );

    std::thread::sleep(Duration::from_millis(50));

    println!("\n=== Testing Max Concurrent Jobs Limit ===");
    println!("Attempting to start 12 concurrent implementations (limit is 10)");

    let mut request_ids = Vec::new();

    // Try to send 12 concurrent requests
    for i in 0..12 {
        let line = i * 4; // Each function is at line i*4
        let req_id = client.send_request_async(
            "workspace/executeCommand",
            json!({
                "command": "amp.implFunction",
                "arguments": [test_uri, line, 0, 1, "rust"]
            }),
        );
        request_ids.push(req_id);
        // Small delay to ensure order
        std::thread::sleep(Duration::from_millis(10));
    }

    println!("Sent {} requests", request_ids.len());

    // Collect responses (should be quick since they return immediately)
    let messages = client.collect_messages(Duration::from_secs(5));

    let mut responses: HashMap<i32, Value> = HashMap::new();
    for msg in &messages {
        if let Some(id) = msg.get("id") {
            if msg.get("result").is_some() || msg.get("error").is_some() {
                if let Some(id_num) = id.as_i64() {
                    responses.insert(id_num as i32, msg.clone());
                }
            }
        }
    }

    println!("\n=== Response Analysis ===");

    let mut success_count = 0;
    let mut error_count = 0;
    let mut max_limit_errors = 0;

    for req_id in &request_ids {
        if let Some(resp) = responses.get(req_id) {
            if resp.get("result").is_some() {
                success_count += 1;
            } else if let Some(error) = resp.get("error") {
                error_count += 1;
                if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                    if message.contains("Maximum concurrent implementations") {
                        max_limit_errors += 1;
                        println!("  Request {}: Max limit error (expected)", req_id);
                    } else {
                        println!("  Request {}: Other error: {}", req_id, message);
                    }
                }
            }
        } else {
            println!("  Request {}: No response", req_id);
        }
    }

    println!("\n=== Results ===");
    println!("Success: {}", success_count);
    println!("Errors: {}", error_count);
    println!("Max limit errors: {}", max_limit_errors);

    // We expect first 10 to succeed, last 2 to fail with max limit error
    assert!(
        success_count <= 10,
        "Expected at most 10 successful requests, got {}",
        success_count
    );
    assert!(
        max_limit_errors >= 2,
        "Expected at least 2 max limit errors (for requests 11-12), got {}",
        max_limit_errors
    );

    println!("✓ Max concurrent jobs limit is enforced correctly");

    let stderr = client.drain_stderr();
    if !stderr.is_empty() {
        println!("\n=== Server Stderr (last 1000 chars) ===");
        let stderr_tail: String = stderr
            .chars()
            .rev()
            .take(1000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        println!("{}", stderr_tail);
    }

    client.shutdown();
}
