use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::Sender;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    request::ApplyWorkspaceEdit, request::Request as _, ApplyWorkspaceEditParams,
    OptionalVersionedTextDocumentIdentifier, Position, Range, TextDocumentEdit, TextEdit, Url,
    WorkspaceEdit,
};
use tracing::info;

static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct LspClient {
    sender: Sender<Message>,
}

impl LspClient {
    pub fn new(connection: &Connection) -> Self {
        Self {
            sender: connection.sender.clone(),
        }
    }

    pub fn new_from_sender(sender: Sender<Message>) -> Self {
        Self { sender }
    }

    pub fn send_response(&self, response: Response) -> Result<(), Box<dyn Error + Sync + Send>> {
        self.sender.send(Message::Response(response))?;
        Ok(())
    }

    pub fn send_success(
        &self,
        req: &Request,
        result: serde_json::Value,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let response = Response {
            id: req.id.clone(),
            result: Some(result),
            error: None,
        };
        self.send_response(response)
    }

    pub fn send_error(
        &self,
        req: &Request,
        code: i32,
        message: &str,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let response = Response {
            id: req.id.clone(),
            result: None,
            error: Some(lsp_server::ResponseError {
                code,
                message: message.to_string(),
                data: None,
            }),
        };
        self.send_response(response)
    }

    pub fn send_method_not_found(
        &self,
        req: &Request,
        method: &str,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        self.send_error(
            req,
            lsp_server::ErrorCode::MethodNotFound as i32,
            &format!("Method not found: {}", method),
        )
    }

    pub fn send_invalid_params(
        &self,
        req: &Request,
        message: &str,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        self.send_error(req, lsp_server::ErrorCode::InvalidParams as i32, message)
    }

    pub fn send_apply_edit(
        &self,
        edit: WorkspaceEdit,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let params = ApplyWorkspaceEditParams {
            label: Some("Implement function".to_string()),
            edit,
        };

        let request_id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let request_id = lsp_server::RequestId::from(format!("apply_edit_{}", request_id));

        let request = Request {
            id: request_id,
            method: ApplyWorkspaceEdit::METHOD.to_string(),
            params: serde_json::to_value(params)?,
        };

        info!("Sending workspace/applyEdit request");
        self.sender.send(Message::Request(request))?;
        Ok(())
    }

    pub fn send_notification<T: serde::Serialize>(
        &self,
        method: &str,
        params: T,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let notification = Notification {
            method: method.to_string(),
            params: serde_json::to_value(params)?,
        };
        self.sender.send(Message::Notification(notification))?;
        Ok(())
    }
}

pub struct WorkspaceEditBuilder;

impl WorkspaceEditBuilder {
    pub fn create_line_insert(
        uri: &Url,
        current_text: &str,
        line: u32,
        implementation: &str,
        version: i32,
    ) -> WorkspaceEdit {
        let line_start = Position { line, character: 0 };
        let line_end = Position {
            line: line + 1,
            character: 0,
        };

        let current_line = current_text.lines().nth(line as usize).unwrap_or("");

        let new_text = format!("{}\n{}\n", current_line, implementation);

        let edit = TextEdit {
            range: Range {
                start: line_start,
                end: line_end,
            },
            new_text,
        };

        WorkspaceEdit {
            document_changes: Some(lsp_types::DocumentChanges::Edits(vec![TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: Some(version),
                },
                edits: vec![lsp_types::OneOf::Left(edit)],
            }])),
            ..Default::default()
        }
    }
}
