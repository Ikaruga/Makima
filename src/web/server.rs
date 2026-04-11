//! Web server implementation

use crate::config::Config;
use crate::context::Conversation;
use crate::llm::{generate_tool_prompt, LmStudioClient};
use crate::tools::ToolRegistry;
use crate::web::routes;
use crate::web::websocket;
use anyhow::Result;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rust_embed::Embed;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

#[derive(Embed)]
#[folder = "static/"]
struct StaticFiles;

/// Shared application state
pub struct AppState {
    pub client: LmStudioClient,
    pub registry: ToolRegistry,
    pub conversations: RwLock<Vec<Conversation>>,
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let client = LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model)
            .with_max_tokens(config.lm_studio.max_tokens)
            .with_temperature(config.lm_studio.temperature);

        let working_dir = if config.tools.working_dir.is_empty() {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        } else {
            config.tools.working_dir.clone()
        };

        // Create a separate client for OCR (tools need their own Arc)
        let ocr_client = Arc::new(
            LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model)
                .with_max_tokens(config.lm_studio.max_tokens)
                .with_temperature(config.lm_studio.temperature)
        );

        let registry = ToolRegistry::with_defaults_and_client(Some(working_dir), Some(ocr_client));

        Self {
            client,
            registry,
            conversations: RwLock::new(Vec::new()),
            config,
        }
    }

    /// Get or create a conversation
    pub async fn get_or_create_conversation(&self, id: Option<&str>) -> Conversation {
        let mut convs = self.conversations.write().await;

        if let Some(id) = id {
            if let Some(conv) = convs.iter().find(|c| c.id == id) {
                return conv.clone();
            }
        }

        // Create new conversation
        let tool_prompt = generate_tool_prompt(&self.registry.get_definitions());
        let conv = Conversation::new().with_system_prompt(tool_prompt);
        convs.push(conv.clone());
        conv
    }

    /// Update a conversation
    pub async fn update_conversation(&self, conversation: Conversation) {
        let mut convs = self.conversations.write().await;
        if let Some(pos) = convs.iter().position(|c| c.id == conversation.id) {
            convs[pos] = conversation;
        } else {
            convs.push(conversation);
        }
    }
}

/// Handler pour servir les fichiers statiques embarqués
async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve_embedded_file(path)
}

/// Handler pour la racine "/"
async fn index_handler() -> Response {
    serve_embedded_file("index.html")
}

fn serve_embedded_file(path: &str) -> Response {
    match StaticFiles::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Run the web server
pub async fn run_server(config: Config) -> Result<()> {
    let state = Arc::new(AppState::new(config.clone()));

    // Build the router
    let app = Router::new()
        // API routes
        .route("/api/chat", post(routes::chat_handler))
        .route("/api/conversations", get(routes::list_conversations))
        .route("/api/conversations/{id}", get(routes::get_conversation).delete(routes::delete_conversation))
        .route("/api/config", get(routes::get_config))
        .route("/api/tools", get(routes::list_tools))
        .route("/api/health", get(routes::health_check))
        // WebSocket for streaming
        .route("/ws", get(websocket::ws_handler))
        // Fichiers statiques embarqués
        .route("/", get(index_handler))
        .fallback(get(static_handler))
        // Add CORS
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("🌐 Web server starting on http://{}", addr);
    println!("📡 WebSocket available at ws://{}/ws", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
