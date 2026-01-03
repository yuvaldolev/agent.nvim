use std::error::Error;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::Sender;
use lsp_server::{Connection, Message, Notification, Request};
use lsp_types::request::CodeActionRequest;
use lsp_types::{
    notification::DidChangeTextDocument, notification::DidOpenTextDocument,
    notification::Notification as _, request::Completion, request::ExecuteCommand,
    request::Request as _, CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams, ExecuteCommandParams,
    Url,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};
use uuid::Uuid;

use crate::backend::create_backend;
use crate::config::DELETE_TEMP_FILES;
use crate::document_store::DocumentStore;
use crate::job_queue::JobQueue;
use crate::lsp_utils::{LspClient, WorkspaceEditBuilder};

pub const COMMAND_IMPL_FUNCTION: &str = "amp.implFunction";
pub const NOTIFICATION_IMPL_FUNCTION_PROGRESS: &str = "amp/implFunctionProgress";

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplFunctionProgressParams {
    pub job_id: String,
    pub uri: String,
    pub line: u32,
    pub preview: String,
}

pub struct RequestHandler<'a> {
    connection: &'a Connection,
    document_store: Arc<DocumentStore>,
    job_queue: Arc<JobQueue>,
}

impl<'a> RequestHandler<'a> {
    pub fn new(
        connection: &'a Connection,
        document_store: Arc<DocumentStore>,
        job_queue: Arc<JobQueue>,
    ) -> Self {
        Self {
            connection,
            document_store,
            job_queue,
        }
    }

    pub fn handle(&self, req: &Request) -> Result<(), Box<dyn Error + Sync + Send>> {
        let lsp_client = LspClient::new(self.connection);

        match req.method.as_str() {
            Completion::METHOD => self.handle_completion(req, &lsp_client),
            CodeActionRequest::METHOD => self.handle_code_action(req, &lsp_client),
            ExecuteCommand::METHOD => self.handle_execute_command(req, &lsp_client),
            _ => {
                info!("Unhandled request: {}", req.method);
                lsp_client.send_method_not_found(req, &req.method)
            }
        }
    }

    fn handle_completion(
        &self,
        req: &Request,
        lsp_client: &LspClient,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params: CompletionParams = serde_json::from_value(req.params.clone())?;

        info!(
            "Completion request received - uri: {}, position: {:?}, context: {:?}",
            params.text_document_position.text_document.uri,
            params.text_document_position.position,
            params.context
        );

        lsp_client.send_success(req, serde_json::Value::Null)
    }

    fn handle_code_action(
        &self,
        req: &Request,
        lsp_client: &LspClient,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params: CodeActionParams = serde_json::from_value(req.params.clone())?;
        let uri = &params.text_document.uri;
        let position = params.range.start;

        info!(
            "Code action request - uri: {}, position: {:?}",
            uri, position
        );

        let doc = match self.document_store.get(uri) {
            Some(d) => d,
            None => return lsp_client.send_success(req, json!([])),
        };

        let action = CodeAction {
            title: "Implement function with Amp".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            command: Some(lsp_types::Command {
                title: "Implement function with Amp".to_string(),
                command: COMMAND_IMPL_FUNCTION.to_string(),
                arguments: Some(vec![
                    json!(uri.to_string()),
                    json!(position.line),
                    json!(position.character),
                    json!(doc.version),
                    json!(doc.language_id),
                ]),
            }),
            ..Default::default()
        };

        let actions: Vec<CodeActionOrCommand> = vec![CodeActionOrCommand::CodeAction(action)];
        lsp_client.send_success(req, serde_json::to_value(actions)?)
    }

    fn handle_execute_command(
        &self,
        req: &Request,
        lsp_client: &LspClient,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params: ExecuteCommandParams = serde_json::from_value(req.params.clone())?;
        info!("Execute command: {}", params.command);

        if params.command != COMMAND_IMPL_FUNCTION {
            return lsp_client
                .send_invalid_params(req, &format!("Unknown command: {}", params.command));
        }

        let args = &params.arguments;
        if args.len() < 5 {
            return lsp_client.send_invalid_params(req, "Missing arguments for amp.implFunction");
        }

        let uri_str: String = serde_json::from_value(args[0].clone())?;
        let line: u32 = serde_json::from_value(args[1].clone())?;
        let character: u32 = serde_json::from_value(args[2].clone())?;
        let _version: i32 = serde_json::from_value(args[3].clone())?;
        let language_id: String = serde_json::from_value(args[4].clone())?;

        let uri = Url::parse(&uri_str)?;
        if self.document_store.get(&uri).is_none() {
            return lsp_client.send_invalid_params(req, "Document not found");
        }

        let file_path = uri
            .to_file_path()
            .map_err(|_| "Invalid file URI")?
            .to_string_lossy()
            .to_string();

        let job_id = Uuid::new_v4().to_string();
        let sender = self.connection.sender.clone();
        let uri_clone = uri.clone();
        let job_queue = self.job_queue.clone();
        let document_store = self.document_store.clone();

        lsp_client.send_success(req, serde_json::Value::Null)?;

        spawn_implementation_worker(
            job_id,
            sender,
            uri_clone,
            file_path,
            line,
            character,
            language_id,
            job_queue,
            document_store,
        );

        Ok(())
    }
}

fn spawn_implementation_worker(
    job_id: String,
    sender: Sender<Message>,
    uri: Url,
    file_path: String,
    original_line: u32,
    character: u32,
    language_id: String,
    job_queue: Arc<JobQueue>,
    document_store: Arc<DocumentStore>,
) {
    thread::spawn(move || {
        let lsp_client = LspClient::new_from_sender(sender);
        let backend = create_backend();

        // Acquire the slot for this file (blocks if another job is active)
        // Returns the adjusted line number (may differ from original if previous edits shifted lines)
        let line = job_queue.acquire(&uri, &job_id, original_line);

        info!(
            "Acquired slot: original_line={}, adjusted_line={}",
            original_line, line
        );

        // Get fresh document state after acquiring the lock (BASE for merge)
        let doc = match document_store.get(&uri) {
            Some(d) => d,
            None => {
                error!("Document not found after acquiring lock");
                job_queue.release(&uri, &job_id);
                return;
            }
        };
        let base_text = doc.text.clone();

        // Clone values for the progress callback closure
        let progress_job_id = job_id.clone();
        let progress_uri = uri.to_string();
        let progress_line = line;
        let progress_sender = lsp_client.clone_sender();

        // Generate a temporary file path for the agent to create and write the implementation
        // We DON'T create the file - let the agent create it to avoid unnecessary reads of empty files
        // Place it in the same directory as the file being edited to avoid permission errors.
        let parent_dir = std::path::Path::new(&file_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));

        let temp_filename = format!("agent_impl_{}.rs", Uuid::new_v4());
        let output_path = parent_dir.join(&temp_filename);
        let output_path_str = output_path.to_string_lossy().to_string();
        info!(
            "Generated temp file path for agent output: {}",
            output_path_str
        );

        match backend.implement_function_streaming(
            &file_path,
            line,
            character,
            &language_id,
            &base_text,
            &output_path_str,
            Box::new(move |preview| {
                let params = ImplFunctionProgressParams {
                    job_id: progress_job_id.clone(),
                    uri: progress_uri.clone(),
                    line: progress_line,
                    preview: preview.to_string(),
                };
                let progress_client = LspClient::new_from_sender(progress_sender.clone());
                if let Err(e) =
                    progress_client.send_notification(NOTIFICATION_IMPL_FUNCTION_PROGRESS, params)
                {
                    error!("Failed to send progress notification: {}", e);
                }
            }),
        ) {
            Ok(_) => {
                // Read the implementation from the temp file that the agent created
                let implementation = match std::fs::read_to_string(&output_path) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to read agent output from temp file: {}", e);
                        job_queue.release(&uri, &job_id);
                        return;
                    }
                };

                // Clean up the temp file if configured to do so
                if DELETE_TEMP_FILES {
                    if let Err(e) = std::fs::remove_file(&output_path) {
                        error!("Failed to remove temp file: {}", e);
                    }
                } else {
                    info!("Preserving temp file for debugging: {}", output_path_str);
                }

                if implementation.trim().is_empty() {
                    error!("Agent output file is empty");
                    job_queue.release(&uri, &job_id);
                    return;
                }

                if implementation.trim().is_empty() {
                    error!("Agent output file is empty");
                    job_queue.release(&uri, &job_id);
                    return;
                }

                // Get "Yours" version (Current state with user edits)
                let current_doc = match document_store.get(&uri) {
                    Some(d) => d,
                    None => {
                        error!("Document not found when applying edit");
                        job_queue.release(&uri, &job_id);
                        return;
                    }
                };
                let current_text = current_doc.text.clone();

                // BUG FIX:
                // 1. Correct start line: The User might trigger CodeAction inside the function body.
                //    We need to find the actual start of the function signature to replace correctly.
                // 2. Full File Check: If the Agent ignored the prompt and wrote the whole file,
                //    `replace_function` would insert the WHOLE file into the function slot.

                let base_lines: Vec<&str> = base_text.lines().collect();

                // Heuristic: If implementation is large (> 80% of base) and contains base start/end lines?
                // Or just: If implementation has much more lines than the function we are replacing?
                // Let's assume for now we fix the prompt and rely on `replace_function`.
                // But we MUST fix the start line.

                let start_line = crate::utils::find_function_start(&base_lines, line as usize)
                    .unwrap_or(line as usize);
                info!("Adjusted start line from {} to {}", line, start_line);

                // Check if implementation looks like the whole file.
                // A single function usually isn't the whole file (unless the file is tiny).
                // If implementation line count is close to base text line count?
                // Better heuristic: If implementation contains "use " statements that match beginning of base_text?
                // For now, let's trust the refined prompt + start_line fix.
                // However, user said "re-adds other functions". This strongly implies full file output.
                // If it IS full file, we should treat `implementation` as the `theirs` text directly.

                let impl_lines_count = implementation.lines().count();
                let base_lines_count = base_lines.len();

                // Extremely rough heuristic: If implementation is > 50% of file (and file is not tiny), threat?
                // Or if it starts with the first line of base_text?
                let is_likely_full_file = if base_lines_count > 10 {
                    // Check if first non-empty line of base matches first non-empty line of implementation
                    let base_first = base_lines.iter().find(|l| !l.trim().is_empty());
                    let impl_first = implementation.lines().find(|l| !l.trim().is_empty());
                    // base_first is Option<&&str>, impl_first is Option<&str>
                    match (base_first, impl_first) {
                        (Some(b), Some(i)) => *b == i && impl_lines_count > base_lines_count / 2,
                        _ => false,
                    }
                } else {
                    false
                };

                let theirs_text = if is_likely_full_file {
                    info!("Detected likely full-file output from Agent. Using implementation as full text.");
                    implementation.clone()
                } else {
                    // Use 3-way merge helper logic (snippet replacement)
                    // We need to construct "Theirs" manually if we changed the start line logic,
                    // because `create_3way_merge_edit` takes `line`.
                    // We should update `create_3way_merge_edit` or call `replace_function` here.
                    // `create_3way_merge_edit` calls `replace_function`.
                    // So we can just pass the new `start_line`.
                    match crate::utils::replace_function(&base_text, start_line, &implementation) {
                        Some(text) => text,
                        None => {
                            error!("Failed to replace function in base text");
                            job_queue.release(&uri, &job_id);
                            return;
                        }
                    }
                };

                // Perform 3-way merge
                let merged_text = match diffy::merge(&base_text, &current_text, &theirs_text) {
                    Ok(text) => text,
                    Err(text) => text,
                };

                let edit =
                    WorkspaceEditBuilder::create_full_replace(&uri, &current_text, &merged_text);

                if let Err(e) = lsp_client.send_apply_edit(edit) {
                    error!("Failed to send apply edit: {}", e);
                }

                // Adjust pending job line numbers (using rough estimation)
                let new_lines_count = implementation.lines().count() as i32;
                let old_lines_count = crate::utils::find_function_end(&base_lines, start_line)
                    .map(|end| (end - start_line + 1) as i32)
                    .unwrap_or(0);

                let lines_added = if is_likely_full_file {
                    (impl_lines_count as i32) - (base_lines_count as i32)
                } else {
                    new_lines_count - old_lines_count
                };

                job_queue.adjust_pending_lines(&uri, line, lines_added);
                info!(
                    "Adjusted pending lines: edit_line={}, lines_added={}",
                    line, lines_added
                );
            }
            Err(e) => {
                error!("Backend error: {}", e);
                // Clean up the temp file on error (if it exists and cleanup is enabled)
                if DELETE_TEMP_FILES && output_path.exists() {
                    if let Err(cleanup_err) = std::fs::remove_file(&output_path) {
                        error!("Failed to remove temp file after error: {}", cleanup_err);
                    }
                } else if !DELETE_TEMP_FILES && output_path.exists() {
                    info!(
                        "Preserving temp file for debugging (error case): {}",
                        output_path_str
                    );
                }
            }
        }

        // Release the slot so the next job can proceed
        job_queue.release(&uri, &job_id);
    });
}

pub struct NotificationHandler<'a> {
    document_store: &'a DocumentStore,
}

impl<'a> NotificationHandler<'a> {
    pub fn new(document_store: &'a DocumentStore) -> Self {
        Self { document_store }
    }

    pub fn handle(&self, notification: &Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => self.handle_did_open(notification),
            DidChangeTextDocument::METHOD => self.handle_did_change(notification),
            _ => {
                info!("Unhandled notification: {}", notification.method);
                Ok(())
            }
        }
    }

    fn handle_did_open(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params: DidOpenTextDocumentParams =
            serde_json::from_value(notification.params.clone())?;
        info!(
            "Document opened - uri: {}, language: {}, version: {}",
            params.text_document.uri,
            params.text_document.language_id,
            params.text_document.version
        );
        self.document_store.open(
            params.text_document.uri,
            params.text_document.text,
            params.text_document.version,
            params.text_document.language_id,
        );
        Ok(())
    }

    fn handle_did_change(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params: DidChangeTextDocumentParams =
            serde_json::from_value(notification.params.clone())?;
        info!(
            "Document changed - uri: {}, version: {}, changes: {}",
            params.text_document.uri,
            params.text_document.version,
            params.content_changes.len()
        );
        self.document_store.change(
            &params.text_document.uri,
            params.text_document.version,
            &params.content_changes,
        );
        Ok(())
    }
}
