mod amp;
mod backend;
mod claude_code;
mod config;
mod document_store;
mod handlers;
mod job_tracker;
mod lsp_utils;
mod opencode;
mod utils;

use std::error::Error;
use std::sync::Arc;

use lsp_server::{Connection, Message};
use lsp_types::{
    CodeActionKind, CodeActionOptions, CodeActionProviderCapability, CompletionOptions,
    ExecuteCommandOptions, InitializeParams, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind,
};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::config::CURRENT_BACKEND;
use crate::document_store::DocumentStore;
use crate::handlers::{
    BackendInfoParams, NotificationHandler, RequestHandler, COMMAND_IMPL_FUNCTION,
    NOTIFICATION_BACKEND_INFO,
};
use crate::job_tracker::JobTracker;
use crate::lsp_utils::LspClient;

struct Server {
    connection: Connection,
    document_store: Arc<DocumentStore>,
    job_tracker: Arc<JobTracker>,
}

impl Server {
    fn new(connection: Connection) -> Self {
        Self {
            connection,
            document_store: Arc::new(DocumentStore::new()),
            job_tracker: Arc::new(JobTracker::new()),
        }
    }

    fn initialize(&self) -> Result<serde_json::Value, Box<dyn Error + Sync + Send>> {
        let capabilities = ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(
                TextDocumentSyncKind::INCREMENTAL,
            )),
            completion_provider: Some(CompletionOptions {
                resolve_provider: Some(false),
                trigger_characters: Some(vec![".".to_string()]),
                ..Default::default()
            }),
            code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
                code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                ..Default::default()
            })),
            execute_command_provider: Some(ExecuteCommandOptions {
                commands: vec![COMMAND_IMPL_FUNCTION.to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };

        let server_capabilities = serde_json::to_value(capabilities)?;
        let initialization_params = self.connection.initialize(server_capabilities)?;

        info!(
            "Server initialized with params: {:?}",
            initialization_params
        );

        Ok(initialization_params)
    }

    fn run(&self, params: serde_json::Value) -> Result<(), Box<dyn Error + Sync + Send>> {
        let _init_params: InitializeParams = serde_json::from_value(params)?;

        // Send backend info notification to inform client which backend is being used
        let lsp_client = LspClient::new(&self.connection);
        let backend_name = CURRENT_BACKEND.display_name();
        lsp_client.send_notification(
            NOTIFICATION_BACKEND_INFO,
            BackendInfoParams {
                name: backend_name.to_string(),
            },
        )?;
        info!("Sent backend info notification: {}", backend_name);

        for msg in &self.connection.receiver {
            match msg {
                Message::Request(req) => {
                    if self.connection.handle_shutdown(&req)? {
                        break;
                    }
                    let handler = RequestHandler::new(
                        &self.connection,
                        self.document_store.clone(),
                        self.job_tracker.clone(),
                    );
                    handler.handle(&req)?;
                }
                Message::Notification(notification) => {
                    let handler = NotificationHandler::new(&self.document_store);
                    handler.handle(&notification)?;
                }
                Message::Response(resp) => {
                    info!("Received response: {:?}", resp);
                }
            }
        }

        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    info!("Starting agent-lsp server");

    let (connection, io_threads) = Connection::stdio();

    let server = Server::new(connection);
    let params = server.initialize()?;
    server.run(params)?;

    io_threads.join()?;

    info!("Server shutting down");

    Ok(())
}
