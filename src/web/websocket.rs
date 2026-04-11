//! WebSocket handler for real-time streaming

use crate::llm::{StreamEvent, ToolParser};
use crate::web::server::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use futures::SinkExt;

/// Execution mode
#[derive(Debug, Clone, Copy, PartialEq)]
enum ExecMode {
    Edit,
    Plan,
}

/// WebSocket message types
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// User sends a chat message
    #[serde(rename = "chat")]
    Chat {
        message: String,
        conversation_id: Option<String>,
    },
    /// Server streams content
    #[serde(rename = "content")]
    Content { text: String },
    /// Server indicates tool call start
    #[serde(rename = "tool_start")]
    ToolStart { name: String, id: String },
    /// Server sends tool result
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        result: String,
        success: bool,
    },
    /// Server requests tool confirmation
    #[serde(rename = "tool_confirm")]
    ToolConfirm {
        id: String,
        name: String,
        summary: String,
    },
    /// Client responds to tool confirmation
    #[serde(rename = "tool_response")]
    ToolResponse {
        id: String,
        approved: bool,
    },
    /// Server notifies tool was skipped
    #[serde(rename = "tool_skipped")]
    ToolSkipped {
        name: String,
        reason: String,
    },
    /// Client sets execution mode
    #[serde(rename = "set_mode")]
    SetMode { mode: String },
    /// Server confirms mode change
    #[serde(rename = "mode_changed")]
    ModeChanged { mode: String },
    /// Client requests clear conversation
    #[serde(rename = "clear")]
    Clear,
    /// Client requests new conversation
    #[serde(rename = "new_conversation")]
    NewConversation,
    /// Server indicates completion
    #[serde(rename = "done")]
    Done { conversation_id: String },
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
    /// Client requests stop streaming
    #[serde(rename = "stop")]
    Stop,
    /// Ping/pong for keepalive
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
}

/// WebSocket upgrade handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-connection state
struct ConnState {
    mode: ExecMode,
    pending_confirms: HashMap<String, oneshot::Sender<bool>>,
    cancel_token: CancellationToken,
}

/// Helper to send a WsMessage through a split sink
async fn ws_send(sender: &mut SplitSink<WebSocket, Message>, msg: &WsMessage) {
    let _ = sender
        .send(Message::Text(serde_json::to_string(msg).unwrap()))
        .await;
}

/// Read next text message from split stream, handling pings
async fn ws_recv_text(
    receiver: &mut SplitStream<WebSocket>,
    sender: &mut SplitSink<WebSocket, Message>,
) -> Option<String> {
    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => return Some(text.to_string()),
            Some(Ok(Message::Close(_))) | None => return None,
            Some(Ok(Message::Ping(data))) => {
                let _ = sender.send(Message::Pong(data)).await;
            }
            _ => continue,
        }
    }
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let parser = ToolParser::new();
    let mut conn = ConnState {
        mode: ExecMode::Edit,
        pending_confirms: HashMap::new(),
        cancel_token: CancellationToken::new(),
    };

    let (mut sender, mut receiver) = socket.split();

    loop {
        let text = match ws_recv_text(&mut receiver, &mut sender).await {
            Some(t) => t,
            None => break,
        };

        let ws_msg: WsMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                ws_send(&mut sender, &WsMessage::Error {
                    message: format!("Invalid message format: {}", e),
                }).await;
                continue;
            }
        };

        match ws_msg {
            WsMessage::SetMode { mode } => {
                conn.mode = match mode.as_str() {
                    "plan" => ExecMode::Plan,
                    _ => ExecMode::Edit,
                };
                ws_send(&mut sender, &WsMessage::ModeChanged {
                    mode: if conn.mode == ExecMode::Plan { "plan".into() } else { "edit".into() },
                }).await;
            }
            WsMessage::Clear | WsMessage::NewConversation => {}
            WsMessage::ToolResponse { id, approved } => {
                if let Some(tx) = conn.pending_confirms.remove(&id) {
                    let _ = tx.send(approved);
                }
            }
            WsMessage::Stop => {
                conn.cancel_token.cancel();
                conn.cancel_token = CancellationToken::new();
            }
            WsMessage::Chat {
                message,
                conversation_id,
            } => {
                // Reset cancel token for this request
                conn.cancel_token = CancellationToken::new();
                let cancel = conn.cancel_token.clone();

                let mut conversation = state
                    .get_or_create_conversation(conversation_id.as_deref())
                    .await;

                let conv_id = conversation.id.clone();
                conversation.add_user_message(&message);

                let mut cancelled = false;

                // Stream response loop (tool call loop)
                loop {
                    let messages = conversation.get_messages();
                    let tools = Some(state.registry.get_definitions());

                    let rx = match state.client.chat_stream(messages, tools).await {
                        Ok(rx) => rx,
                        Err(e) => {
                            ws_send(&mut sender, &WsMessage::Error {
                                message: e.to_string(),
                            }).await;
                            break;
                        }
                    };

                    let mut rx = rx;
                    let mut content = String::new();
                    let mut tool_calls = Vec::new();

                    // Streaming loop with cancellation support
                    // We spawn a task to forward WS messages (Stop, ToolResponse, Ping)
                    // while we consume the LLM stream
                    // Streaming loop: select between LLM events, WS messages, and cancel
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => {
                                // Stop requested
                                drop(rx);
                                cancelled = true;
                                break;
                            }
                            ws_text = receiver.next() => {
                                match ws_text {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(inner) = serde_json::from_str::<WsMessage>(&text.to_string()) {
                                            match inner {
                                                WsMessage::Stop => {
                                                    cancel.cancel();
                                                    drop(rx);
                                                    cancelled = true;
                                                    break;
                                                }
                                                WsMessage::Ping => {
                                                    ws_send(&mut sender, &WsMessage::Pong).await;
                                                }
                                                WsMessage::ToolResponse { id, approved } => {
                                                    if let Some(tx) = conn.pending_confirms.remove(&id) {
                                                        let _ = tx.send(approved);
                                                    }
                                                }
                                                WsMessage::SetMode { mode } => {
                                                    conn.mode = match mode.as_str() {
                                                        "plan" => ExecMode::Plan,
                                                        _ => ExecMode::Edit,
                                                    };
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        cancelled = true;
                                        break;
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = sender.send(Message::Pong(data)).await;
                                    }
                                    _ => {}
                                }
                            }
                            event = rx.recv() => {
                                match event {
                                    Some(StreamEvent::Content(text)) => {
                                        content.push_str(&text);
                                        ws_send(&mut sender, &WsMessage::Content { text }).await;
                                    }
                                    Some(StreamEvent::ToolCallStart { id, name }) => {
                                        ws_send(&mut sender, &WsMessage::ToolStart {
                                            name: name.clone(),
                                            id: id.clone(),
                                        }).await;
                                    }
                                    Some(StreamEvent::ToolCallComplete(tc)) => {
                                        tool_calls.push(tc);
                                    }
                                    Some(StreamEvent::Done) => break,
                                    Some(StreamEvent::Error(e)) => {
                                        ws_send(&mut sender, &WsMessage::Error { message: e }).await;
                                    }
                                    None => break,
                                    _ => {}
                                }
                            }
                        }
                    }

                    if cancelled {
                        // Save partial content
                        if !content.is_empty() {
                            let clean = if parser.contains_tool_calls(&content) {
                                parser.extract_text_before_tools(&content)
                            } else {
                                content.clone()
                            };
                            conversation.add_assistant_message(&clean);
                        }
                        break;
                    }

                    // Parse tool calls from content if no native ones
                    let parsed_calls = if tool_calls.is_empty() && parser.contains_tool_calls(&content) {
                        parser.parse_from_text(&content)
                    } else {
                        parser.parse_native(&tool_calls)
                    };

                    let clean_content = if parser.contains_tool_calls(&content) {
                        parser.extract_text_before_tools(&content)
                    } else {
                        content.clone()
                    };

                    if !clean_content.is_empty() || parsed_calls.is_empty() {
                        conversation.add_assistant_message(&clean_content);
                    }

                    if parsed_calls.is_empty() {
                        break;
                    }

                    let current_mode = conn.mode;

                    for call in &parsed_calls {
                        if cancel.is_cancelled() {
                            cancelled = true;
                            break;
                        }

                        if let Some(tool) = state.registry.get(&call.name) {
                            if current_mode == ExecMode::Plan {
                                ws_send(&mut sender, &WsMessage::ToolSkipped {
                                    name: call.name.clone(),
                                    reason: "Non exécuté (mode Plan)".into(),
                                }).await;
                                conversation.add_tool_result(&call.id, "[Non exécuté — mode Plan]");
                                continue;
                            }

                            if tool.requires_confirmation() {
                                let summary = tool.summarize_call(call);
                                ws_send(&mut sender, &WsMessage::ToolConfirm {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    summary,
                                }).await;

                                let (tx, rx_confirm) = oneshot::channel();
                                conn.pending_confirms.insert(call.id.clone(), tx);

                                let approved = loop {
                                    let text = match ws_recv_text(&mut receiver, &mut sender).await {
                                        Some(t) => t,
                                        None => break false,
                                    };
                                    if let Ok(inner) = serde_json::from_str::<WsMessage>(&text) {
                                        match inner {
                                            WsMessage::ToolResponse { id: rid, approved } if rid == call.id => {
                                                conn.pending_confirms.remove(&rid);
                                                break approved;
                                            }
                                            WsMessage::ToolResponse { id: rid, approved } => {
                                                if let Some(tx) = conn.pending_confirms.remove(&rid) {
                                                    let _ = tx.send(approved);
                                                }
                                            }
                                            WsMessage::Stop => {
                                                cancel.cancel();
                                                break false;
                                            }
                                            WsMessage::Ping => {
                                                ws_send(&mut sender, &WsMessage::Pong).await;
                                            }
                                            _ => {}
                                        }
                                    }
                                };

                                drop(rx_confirm);

                                if cancel.is_cancelled() {
                                    cancelled = true;
                                    break;
                                }

                                if !approved {
                                    ws_send(&mut sender, &WsMessage::ToolSkipped {
                                        name: call.name.clone(),
                                        reason: "Refusé par l'utilisateur".into(),
                                    }).await;
                                    conversation.add_tool_result(&call.id, "[Refusé par l'utilisateur]");
                                    continue;
                                }
                            }

                            let result = tool.execute(call).await;

                            match result {
                                Ok(result) => {
                                    conversation.add_tool_result(&call.id, &result.content);
                                    ws_send(&mut sender, &WsMessage::ToolResult {
                                        name: call.name.clone(),
                                        result: result.content,
                                        success: result.success,
                                    }).await;
                                }
                                Err(e) => {
                                    let error_msg = format!("Erreur : {}", e);
                                    conversation.add_tool_result(&call.id, &error_msg);
                                    ws_send(&mut sender, &WsMessage::ToolResult {
                                        name: call.name.clone(),
                                        result: error_msg,
                                        success: false,
                                    }).await;
                                }
                            }
                        }
                    }

                    if cancelled || current_mode == ExecMode::Plan {
                        break;
                    }
                }

                state.update_conversation(conversation).await;

                ws_send(&mut sender, &WsMessage::Done {
                    conversation_id: conv_id,
                }).await;
            }
            WsMessage::Ping => {
                ws_send(&mut sender, &WsMessage::Pong).await;
            }
            _ => {}
        }
    }
}
