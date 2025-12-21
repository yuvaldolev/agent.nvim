use std::error::Error;
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

use crate::amp::AmpClient;
use crate::document_store::DocumentStore;
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
    document_store: &'a DocumentStore,
}

impl<'a> RequestHandler<'a> {
    pub fn new(connection: &'a Connection, document_store: &'a DocumentStore) -> Self {
        Self {
            connection,
            document_store,
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
        let version: i32 = serde_json::from_value(args[3].clone())?;
        let language_id: String = serde_json::from_value(args[4].clone())?;

        let uri = Url::parse(&uri_str)?;
        let doc = match self.document_store.get(&uri) {
            Some(d) => d,
            None => return lsp_client.send_invalid_params(req, "Document not found"),
        };

        if doc.version != version {
            info!(
                "Version mismatch: expected {}, got {}. Using current version.",
                version, doc.version
            );
        }

        let file_path = uri
            .to_file_path()
            .map_err(|_| "Invalid file URI")?
            .to_string_lossy()
            .to_string();

        let job_id = Uuid::new_v4().to_string();
        let sender = self.connection.sender.clone();
        let doc_text = doc.text.clone();
        let doc_version = doc.version;
        let uri_clone = uri.clone();

        lsp_client.send_success(req, serde_json::Value::Null)?;

        spawn_implementation_worker(
            job_id,
            sender,
            uri_clone,
            file_path,
            line,
            character,
            language_id,
            doc_text,
            doc_version,
        );

        Ok(())
    }
}

fn spawn_implementation_worker(
    job_id: String,
    sender: Sender<Message>,
    uri: Url,
    file_path: String,
    line: u32,
    character: u32,
    language_id: String,
    doc_text: String,
    doc_version: i32,
) {
    thread::spawn(move || {
        let lsp_client = LspClient::new_from_sender(sender);
        let amp_client = AmpClient::new();
        let uri_str = uri.to_string();
        let job_id_clone = job_id.clone();

        match amp_client.implement_function_streaming(
            &file_path,
            line,
            character,
            &language_id,
            &doc_text,
            |preview| {
                let params = ImplFunctionProgressParams {
                    job_id: job_id_clone.clone(),
                    uri: uri_str.clone(),
                    line,
                    preview: preview.to_string(),
                };
                if let Err(e) =
                    lsp_client.send_notification(NOTIFICATION_IMPL_FUNCTION_PROGRESS, params)
                {
                    error!("Failed to send progress notification: {}", e);
                }
            },
        ) {
            Ok(implementation) => {
                let edit = WorkspaceEditBuilder::create_line_insert(
                    &uri,
                    &doc_text,
                    line,
                    &implementation,
                    doc_version,
                );
                if let Err(e) = lsp_client.send_apply_edit(edit) {
                    error!("Failed to send apply edit: {}", e);
                }
            }
            Err(e) => {
                error!("Amp CLI error: {}", e);
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
