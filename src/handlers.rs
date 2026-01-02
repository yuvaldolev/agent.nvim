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

        // Get fresh document state after acquiring the lock
        let doc = match document_store.get(&uri) {
            Some(d) => d,
            None => {
                error!("Document not found after acquiring lock");
                job_queue.release(&uri, &job_id);
                return;
            }
        };
        let doc_text = doc.text.clone();

        // Clone values for the progress callback closure
        let progress_job_id = job_id.clone();
        let progress_uri = uri.to_string();
        let progress_line = line;
        let progress_sender = lsp_client.clone_sender();

        match backend.implement_function_streaming(
            &file_path,
            line,
            character,
            &language_id,
            &doc_text,
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
            Ok(implementation) => {
                // Calculate how many lines the implementation adds
                let lines_added = implementation.lines().count() as i32;

                let edit = WorkspaceEditBuilder::create_line_insert(
                    &uri,
                    &doc_text,
                    line,
                    &implementation,
                );
                if let Err(e) = lsp_client.send_apply_edit(edit) {
                    error!("Failed to send apply edit: {}", e);
                }

                // Adjust pending job line numbers to account for the inserted lines
                job_queue.adjust_pending_lines(&uri, line, lines_added);
                info!(
                    "Adjusted pending lines: edit_line={}, lines_added={}",
                    line, lines_added
                );
            }
            Err(e) => {
                error!("Backend error: {}", e);
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
