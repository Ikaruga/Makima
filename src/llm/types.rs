//! Types for OpenAI-compatible API interactions with LM Studio

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Multimodal Vision Types (for OCR)
// ============================================================================

/// Message content that can be either text or multimodal (text + images)
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Multimodal(Vec<ContentPart>),
}

/// A part of multimodal content
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlData },
}

/// Image URL data (supports base64 data URLs)
#[derive(Debug, Clone, Serialize)]
pub struct ImageUrlData {
    pub url: String, // "data:image/png;base64,..."
}

impl ImageUrlData {
    /// Create from base64-encoded PNG data
    pub fn from_base64_png(base64_data: &str) -> Self {
        Self {
            url: format!("data:image/png;base64,{}", base64_data),
        }
    }

    /// Create from base64-encoded JPEG data
    pub fn from_base64_jpeg(base64_data: &str) -> Self {
        Self {
            url: format!("data:image/jpeg;base64,{}", base64_data),
        }
    }

    /// Create from base64 data with explicit mime type
    pub fn from_base64(base64_data: &str, mime_type: &str) -> Self {
        Self {
            url: format!("data:{};base64,{}", mime_type, base64_data),
        }
    }
}

/// A vision message with multimodal content
#[derive(Debug, Clone, Serialize)]
pub struct VisionMessage {
    pub role: Role,
    pub content: MessageContent,
}

impl VisionMessage {
    /// Create a user message with text and an image (PNG format)
    pub fn user_with_image(text: &str, image_base64: &str) -> Self {
        Self::user_with_image_mime(text, image_base64, "image/png")
    }

    /// Create a user message with text and an image with explicit mime type
    pub fn user_with_image_mime(text: &str, image_base64: &str, mime_type: &str) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Multimodal(vec![
                ContentPart::ImageUrl {
                    image_url: ImageUrlData::from_base64(image_base64, mime_type),
                },
                ContentPart::Text {
                    text: text.to_string(),
                },
            ]),
        }
    }

    /// Create a system message
    pub fn system(content: &str) -> Self {
        Self {
            role: Role::System,
            content: MessageContent::Text(content.to_string()),
        }
    }
}

/// Vision chat completion request
#[derive(Debug, Serialize)]
pub struct VisionChatRequest {
    pub model: String,
    pub messages: Vec<VisionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

// ============================================================================
// Standard Chat Types
// ============================================================================

/// A chat message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Some(content.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Some(content.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(content.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content,
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Message role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call from the assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// Function call details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// Function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Chat completion request
#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// Tool choice for the API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String), // "auto", "none", "required"
    Specific {
        #[serde(rename = "type")]
        choice_type: String,
        function: ToolChoiceFunction,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Chat completion response
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

/// A choice in the response
#[derive(Debug, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<String>,
}

/// Token usage statistics
#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Streaming chunk response
#[derive(Debug, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

/// A choice in a streaming chunk
#[derive(Debug, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

/// Delta content in streaming
#[derive(Debug, Default, Deserialize)]
pub struct Delta {
    pub role: Option<Role>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

/// Tool call delta in streaming
#[derive(Debug, Deserialize)]
pub struct DeltaToolCall {
    pub index: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<DeltaFunction>,
}

/// Function delta in streaming
#[derive(Debug, Deserialize)]
pub struct DeltaFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// Parsed tool call (either from API or fallback parsing)
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: HashMap<String, serde_json::Value>,
}

impl ParsedToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: HashMap<String, serde_json::Value>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.arguments.get(key).and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.arguments.get(key).and_then(|v| v.as_bool())
    }
}
