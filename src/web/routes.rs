//! HTTP route handlers

use crate::context::Conversation;
use crate::web::server::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health check response
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub lm_studio_connected: bool,
}

/// Health check endpoint
pub async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let connected = state.client.health_check().await.unwrap_or(false);

    Json(HealthResponse {
        status: if connected { "ok".to_string() } else { "degraded".to_string() },
        lm_studio_connected: connected,
    })
}

/// Chat request
#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub conversation_id: Option<String>,
}

/// Chat response
#[derive(Serialize)]
pub struct ChatResponse {
    pub conversation_id: String,
    pub response: String,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Serialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub result: String,
    pub success: bool,
}

/// Chat endpoint (non-streaming)
pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    // Get or create conversation
    let mut conversation = state
        .get_or_create_conversation(request.conversation_id.as_deref())
        .await;

    let conversation_id = conversation.id.clone();

    // Add user message
    conversation.add_user_message(&request.message);

    // Get response from LLM
    let messages = conversation.get_messages();
    let tools = Some(state.registry.get_definitions());

    let mut rx = state
        .client
        .chat_stream(messages, tools)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut consumer = crate::llm::StreamConsumer::new();

    while let Some(event) = rx.recv().await {
        consumer.accumulator.process_event(event);
        if consumer.accumulator.done {
            break;
        }
    }

    let response_content = consumer.get_clean_content();
    let tool_calls = consumer.get_tool_calls();

    // Execute tool calls
    let mut tool_call_infos = Vec::new();

    for call in &tool_calls {
        if let Some(tool) = state.registry.get(&call.name) {
            // For API, auto-approve non-destructive tools
            // In a real app, you'd want more sophisticated authorization
            let result = tool.execute(call).await;

            match result {
                Ok(result) => {
                    conversation.add_tool_result(&call.id, &result.content);
                    tool_call_infos.push(ToolCallInfo {
                        name: call.name.clone(),
                        result: result.content.clone(),
                        success: result.success,
                    });
                }
                Err(e) => {
                    let error = format!("Error: {}", e);
                    conversation.add_tool_result(&call.id, &error);
                    tool_call_infos.push(ToolCallInfo {
                        name: call.name.clone(),
                        result: error,
                        success: false,
                    });
                }
            }
        }
    }

    // Add assistant response
    conversation.add_assistant_message(&response_content);

    // Save conversation
    state.update_conversation(conversation).await;

    Ok(Json(ChatResponse {
        conversation_id,
        response: response_content,
        tool_calls: tool_call_infos,
    }))
}

/// Conversation summary
#[derive(Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

/// List all conversations
pub async fn list_conversations(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<ConversationSummary>> {
    let convs = state.conversations.read().await;

    let summaries: Vec<ConversationSummary> = convs
        .iter()
        .map(|c| ConversationSummary {
            id: c.id.clone(),
            title: c.title.clone(),
            message_count: c.message_count(),
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        })
        .collect();

    Json(summaries)
}

/// Get a specific conversation
pub async fn get_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Conversation>, StatusCode> {
    let convs = state.conversations.read().await;

    convs
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Tool info
#[derive(Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub requires_confirmation: bool,
}

/// Delete a conversation
pub async fn delete_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> StatusCode {
    let mut convs = state.conversations.write().await;
    if let Some(pos) = convs.iter().position(|c| c.id == id) {
        convs.remove(pos);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Config info response
#[derive(Serialize)]
pub struct ConfigInfo {
    pub working_dir: String,
    pub model: String,
    pub max_tokens: u32,
}

/// Get configuration info
pub async fn get_config(State(state): State<Arc<AppState>>) -> Json<ConfigInfo> {
    let working_dir = if state.config.tools.working_dir.is_empty() {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| ".".to_string())
    } else {
        state.config.tools.working_dir.clone()
    };

    Json(ConfigInfo {
        working_dir,
        model: state.config.lm_studio.model.clone(),
        max_tokens: state.config.lm_studio.max_tokens,
    })
}

/// List available tools
pub async fn list_tools(State(state): State<Arc<AppState>>) -> Json<Vec<ToolInfo>> {
    let tools: Vec<ToolInfo> = state
        .registry
        .names()
        .into_iter()
        .filter_map(|name| {
            state.registry.get(&name).map(|tool| ToolInfo {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                requires_confirmation: tool.requires_confirmation(),
            })
        })
        .collect();

    Json(tools)
}
