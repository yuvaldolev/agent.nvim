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
use crate::config::{DELETE_TEMP_FILES, CURRENT_BACKEND};
use crate::document_store::DocumentStore;
use crate::job_tracker::JobTracker;
use crate::lsp_utils::{LspClient, WorkspaceEditBuilder};

pub const COMMAND_IMPL_FUNCTION: &str = "amp.implFunction";
pub const NOTIFICATION_IMPL_FUNCTION_PROGRESS: &str = "amp/implFunctionProgress";
pub const NOTIFICATION_JOB_COMPLETED: &str = "amp/jobCompleted";
pub const NOTIFICATION_BACKEND_INFO: &str = "agent/backendInfo";

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplFunctionProgressParams {
    pub job_id: String,
    pub uri: String,
    pub line: u32,
    pub preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobCompletedParams {
    pub job_id: String,
    pub uri: String,
    pub success: bool,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackendInfoParams {
    pub name: String,
}

/// Sends the backend info notification to inform the client which backend is being used.
/// This should be called immediately after LSP initialization completes.
pub fn send_backend_info_notification(
    connection: &Connection,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let lsp_client = LspClient::new(connection);
    let backend_name = CURRENT_BACKEND.display_name();
    lsp_client.send_notification(
        NOTIFICATION_BACKEND_INFO,
        BackendInfoParams {
            name: backend_name.to_string(),
        },
    )?;
    info!("Sent backend info notification: {}", backend_name);
    Ok(())
}

pub struct RequestHandler<'a> {
    connection: &'a Connection,
    document_store: Arc<DocumentStore>,
    job_tracker: Arc<JobTracker>,
}

impl<'a> RequestHandler<'a> {
    pub fn new(
        connection: &'a Connection,
        document_store: Arc<DocumentStore>,
        job_tracker: Arc<JobTracker>,
    ) -> Self {
        Self {
            connection,
            document_store,
            job_tracker,
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
        // Optional 6th argument: pending_id from client for correlation
        let pending_id: Option<String> = args
            .get(5)
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let uri = Url::parse(&uri_str)?;
        let doc = match self.document_store.get(&uri) {
            Some(d) => d,
            None => return lsp_client.send_invalid_params(req, "Document not found"),
        };

        // Check if we've reached the max concurrent jobs limit for this file
        if self.job_tracker.active_job_count(&uri)
            >= crate::job_tracker::MAX_CONCURRENT_JOBS_PER_FILE
        {
            return lsp_client.send_invalid_params(
                req,
                &format!(
                    "Maximum concurrent implementations ({}) reached for this file. Please wait.",
                    crate::job_tracker::MAX_CONCURRENT_JOBS_PER_FILE
                ),
            );
        }

        // Extract function signature for tracking
        let function_signature = crate::utils::extract_function_signature(&doc.text, line as usize)
            .unwrap_or_else(|| format!("line_{}", line));

        info!(
            "Extracted function signature for line {}: '{}'",
            line, function_signature
        );

        let file_path = uri
            .to_file_path()
            .map_err(|_| "Invalid file URI")?
            .to_string_lossy()
            .to_string();

        let job_id = Uuid::new_v4().to_string();
        let sender = self.connection.sender.clone();
        let uri_clone = uri.clone();
        let job_tracker = self.job_tracker.clone();
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
            function_signature,
            job_tracker,
            document_store,
            pending_id,
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
    function_signature: String,
    job_tracker: Arc<JobTracker>,
    document_store: Arc<DocumentStore>,
    pending_id: Option<String>,
) {
    thread::spawn(move || {
        let lsp_client = LspClient::new_from_sender(sender.clone());
        let backend = create_backend();

        // Register the job (non-blocking)
        if let Err(e) =
            job_tracker.register_job(&uri, &job_id, original_line, function_signature.clone())
        {
            error!("Failed to register job: {}", e);
            // Send job completed with error
            let _ = lsp_client.send_notification(
                NOTIFICATION_JOB_COMPLETED,
                JobCompletedParams {
                    job_id: job_id.clone(),
                    uri: uri.to_string(),
                    success: false,
                    error: Some(e),
                    pending_id: pending_id.clone(),
                },
            );
            return;
        }

        info!(
            "Registered job {} at line {} for {}",
            job_id, original_line, uri
        );

        // Get current document state
        let doc = match document_store.get(&uri) {
            Some(d) => d,
            None => {
                error!("Document not found");
                job_tracker.complete_job(&uri, &job_id);
                let _ = lsp_client.send_notification(
                    NOTIFICATION_JOB_COMPLETED,
                    JobCompletedParams {
                        job_id: job_id.clone(),
                        uri: uri.to_string(),
                        success: false,
                        error: Some("Document not found".to_string()),
                        pending_id: pending_id.clone(),
                    },
                );
                return;
            }
        };

        // Clone values for the progress callback closure
        let progress_job_id = job_id.clone();
        let progress_uri = uri.to_string();
        let progress_job_tracker = job_tracker.clone();
        let progress_sender = lsp_client.clone_sender();
        let progress_pending_id = pending_id.clone();

        // Generate a temporary file path for the agent to create and write the implementation
        // We DON'T create the file - let the agent create it to avoid unnecessary reads of empty files
        // Place it in the same directory as the file being edited to avoid permission errors.
        let parent_dir = std::path::Path::new(&file_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("tmp");

        let temp_filename = format!("agent_impl_{}", Uuid::new_v4());
        let output_path = parent_dir.join(&temp_filename);
        let output_path_str = output_path.to_string_lossy().to_string();
        info!(
            "Generated temp file path for agent output: {}",
            output_path_str
        );

        match backend.implement_function_streaming(
            &file_path,
            original_line,
            character,
            &language_id,
            &doc.text,
            &output_path_str,
            &function_signature,
            Box::new(move |preview| {
                // Get current line (may have been adjusted by other jobs)
                let current_line = progress_job_tracker
                    .get_current_line(&progress_job_id)
                    .unwrap_or(original_line);

                let params = ImplFunctionProgressParams {
                    job_id: progress_job_id.clone(),
                    uri: progress_uri.clone(),
                    line: current_line,
                    preview: preview.to_string(),
                    pending_id: progress_pending_id.clone(),
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
                        job_tracker.complete_job(&uri, &job_id);
                        let _ = lsp_client.send_notification(
                            NOTIFICATION_JOB_COMPLETED,
                            JobCompletedParams {
                                job_id: job_id.clone(),
                                uri: uri.to_string(),
                                success: false,
                                error: Some(format!("Failed to read output: {}", e)),
                                pending_id: pending_id.clone(),
                            },
                        );
                        return;
                    }
                };

                // Log the implementation we received for debugging
                info!(
                    "Job {} (original_line={}, signature='{}') received implementation:\n{}",
                    job_id,
                    original_line,
                    function_signature,
                    implementation.lines().take(5).collect::<Vec<_>>().join("\n")
                );

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
                    job_tracker.complete_job(&uri, &job_id);
                    let _ = lsp_client.send_notification(
                        NOTIFICATION_JOB_COMPLETED,
                        JobCompletedParams {
                            job_id: job_id.clone(),
                            uri: uri.to_string(),
                            success: false,
                            error: Some("Agent output is empty".to_string()),
                            pending_id: pending_id.clone(),
                        },
                    );
                    return;
                }

                // Get current document state
                let current_doc = match document_store.get(&uri) {
                    Some(d) => d,
                    None => {
                        error!("Document not found when applying edit");
                        job_tracker.complete_job(&uri, &job_id);
                        let _ = lsp_client.send_notification(
                            NOTIFICATION_JOB_COMPLETED,
                            JobCompletedParams {
                                job_id: job_id.clone(),
                                uri: uri.to_string(),
                                success: false,
                                error: Some("Document not found".to_string()),
                                pending_id: pending_id.clone(),
                            },
                        );
                        return;
                    }
                };
                let current_text = current_doc.text.clone();

                // Get current line (may have been adjusted by other jobs)
                let current_line = job_tracker
                    .get_current_line(&job_id)
                    .unwrap_or(original_line) as usize;

                // Get the expected function signature for verification
                // This ensures we replace the correct function even if line numbers have shifted
                let expected_signature = job_tracker.get_function_signature(&job_id);

                // Replace function in current document
                // This always uses latest agent output, overriding any user edits to this specific function
                let (new_text, start_line, end_line, lines_delta) =
                    match crate::utils::replace_function_in_document(
                        &current_text,
                        current_line,
                        &implementation,
                        expected_signature.as_deref(),
                    ) {
                        Ok(result) => result,
                        Err(e) => {
                            error!("Failed to replace function: {}", e);
                            job_tracker.complete_job(&uri, &job_id);
                            let _ = lsp_client.send_notification(
                                NOTIFICATION_JOB_COMPLETED,
                                JobCompletedParams {
                                    job_id: job_id.clone(),
                                    uri: uri.to_string(),
                                    success: false,
                                    error: Some(format!("Failed to replace function: {}", e)),
                                    pending_id: pending_id.clone(),
                                },
                            );
                            return;
                        }
                    };

                info!(
                    "Replaced function at lines {}-{}, delta: {}",
                    start_line, end_line, lines_delta
                );

                // Create workspace edit
                let edit =
                    WorkspaceEditBuilder::create_full_replace(&uri, &current_text, &new_text);

                // Send the edit
                if let Err(e) = lsp_client.send_apply_edit(edit) {
                    error!("Failed to send apply edit: {}", e);
                    job_tracker.complete_job(&uri, &job_id);
                    let _ = lsp_client.send_notification(
                        NOTIFICATION_JOB_COMPLETED,
                        JobCompletedParams {
                            job_id: job_id.clone(),
                            uri: uri.to_string(),
                            success: false,
                            error: Some(format!("Failed to apply edit: {}", e)),
                            pending_id: pending_id.clone(),
                        },
                    );
                    return;
                }

                // Adjust other jobs' lines
                job_tracker.adjust_lines_for_edit(&uri, start_line, end_line, lines_delta, &job_id);

                // Send line update notifications to other jobs
                let other_jobs = job_tracker.get_active_jobs(&uri);
                for (other_job_id, updated_line) in other_jobs {
                    if other_job_id != job_id {
                        let _ = lsp_client.send_notification(
                            NOTIFICATION_IMPL_FUNCTION_PROGRESS,
                            ImplFunctionProgressParams {
                                job_id: other_job_id,
                                uri: uri.to_string(),
                                line: updated_line,
                                preview: String::new(), // Empty preview indicates line update only
                                pending_id: None, // Other jobs already have their pending_id resolved
                            },
                        );
                    }
                }

                // Send job completed notification
                let _ = lsp_client.send_notification(
                    NOTIFICATION_JOB_COMPLETED,
                    JobCompletedParams {
                        job_id: job_id.clone(),
                        uri: uri.to_string(),
                        success: true,
                        error: None,
                        pending_id: pending_id.clone(),
                    },
                );

                // Complete the job
                job_tracker.complete_job(&uri, &job_id);
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

                // Send job completed notification with error
                let _ = lsp_client.send_notification(
                    NOTIFICATION_JOB_COMPLETED,
                    JobCompletedParams {
                        job_id: job_id.clone(),
                        uri: uri.to_string(),
                        success: false,
                        error: Some(format!("Backend error: {}", e)),
                        pending_id: pending_id.clone(),
                    },
                );

                // Complete the job
                job_tracker.complete_job(&uri, &job_id);
            }
        }
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
