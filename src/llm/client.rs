//! HTTP client for LM Studio API

use crate::llm::types::*;
use anyhow::{Context, Result};
use futures::StreamExt;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::mpsc;

/// Clean up OCR output by removing model artifacts
fn clean_ocr_output(text: &str) -> String {
    // Remove <think>...</think> blocks (closed)
    let re_think = Regex::new(r"(?s)<think>.*?</think>").unwrap();
    let text = re_think.replace_all(text, "");

    // Remove unclosed <think> blocks (when response is truncated)
    let re_think_unclosed = Regex::new(r"(?s)<think>.*$").unwrap();
    let text = re_think_unclosed.replace_all(&text, "");

    // Remove box markers
    let text = text.replace("<|begin_of_box|>", "");
    let text = text.replace("<|end_of_box|>", "");

    // Trim and clean up extra whitespace
    text.trim().to_string()
}

/// Events emitted during streaming
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Text content from the assistant
    Content(String),
    /// Tool call being accumulated
    ToolCallStart { id: String, name: String },
    /// Tool call arguments being streamed
    ToolCallArguments { index: usize, arguments: String },
    /// Tool call completed
    ToolCallComplete(ToolCall),
    /// Stream finished
    Done,
    /// Error occurred
    Error(String),
}

/// Client for LM Studio API
pub struct LmStudioClient {
    client: Client,
    base_url: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
}

impl LmStudioClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            model: model.into(),
            max_tokens: 4096,
            temperature: 0.7,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Get the model name
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get max tokens setting
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    /// Send a chat completion request (non-streaming)
    pub async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatCompletionResponse> {
        let request = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            tools,
            tool_choice: None,
            max_tokens: Some(self.max_tokens),
            temperature: Some(self.temperature),
            stream: Some(false),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to LM Studio")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio API error ({}): {}", status, text);
        }

        response
            .json()
            .await
            .context("Failed to parse LM Studio response")
    }

    /// Send a streaming chat completion request
    pub async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(100);

        let request = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            tools,
            tool_choice: None,
            max_tokens: Some(self.max_tokens),
            temperature: Some(self.temperature),
            stream: Some(true),
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to LM Studio")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio API error ({}): {}", status, text);
        }

        // Spawn task to process the stream
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut tool_arguments: Vec<String> = Vec::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete SSE events
                        while let Some(pos) = buffer.find("\n\n") {
                            let event = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();

                            for line in event.lines() {
                                if let Some(data) = line.strip_prefix("data: ") {
                                    if data.trim() == "[DONE]" {
                                        // Complete any pending tool calls
                                        for (i, tc) in tool_calls.iter().enumerate() {
                                            let args = tool_arguments.get(i).cloned().unwrap_or_default();
                                            let complete_tc = ToolCall {
                                                id: tc.id.clone(),
                                                call_type: tc.call_type.clone(),
                                                function: FunctionCall {
                                                    name: tc.function.name.clone(),
                                                    arguments: args,
                                                },
                                            };
                                            let _ = tx.send(StreamEvent::ToolCallComplete(complete_tc)).await;
                                        }
                                        let _ = tx.send(StreamEvent::Done).await;
                                        return;
                                    }

                                    match serde_json::from_str::<ChatCompletionChunk>(data) {
                                        Ok(chunk) => {
                                            for choice in chunk.choices {
                                                // Handle content
                                                if let Some(content) = choice.delta.content {
                                                    if !content.is_empty() {
                                                        let _ = tx.send(StreamEvent::Content(content)).await;
                                                    }
                                                }

                                                // Handle tool calls
                                                if let Some(delta_tool_calls) = choice.delta.tool_calls {
                                                    for dtc in delta_tool_calls {
                                                        let idx = dtc.index as usize;

                                                        // Ensure we have space for this tool call
                                                        while tool_calls.len() <= idx {
                                                            tool_calls.push(ToolCall {
                                                                id: String::new(),
                                                                call_type: "function".to_string(),
                                                                function: FunctionCall {
                                                                    name: String::new(),
                                                                    arguments: String::new(),
                                                                },
                                                            });
                                                            tool_arguments.push(String::new());
                                                        }

                                                        // Update tool call info
                                                        if let Some(id) = dtc.id {
                                                            tool_calls[idx].id = id;
                                                        }
                                                        if let Some(ct) = dtc.call_type {
                                                            tool_calls[idx].call_type = ct;
                                                        }
                                                        if let Some(func) = dtc.function {
                                                            if let Some(name) = func.name {
                                                                tool_calls[idx].function.name = name.clone();
                                                                let _ = tx.send(StreamEvent::ToolCallStart {
                                                                    id: tool_calls[idx].id.clone(),
                                                                    name,
                                                                }).await;
                                                            }
                                                            if let Some(args) = func.arguments {
                                                                tool_arguments[idx].push_str(&args);
                                                                let _ = tx.send(StreamEvent::ToolCallArguments {
                                                                    index: idx,
                                                                    arguments: args,
                                                                }).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to parse chunk: {} - data: {}", e, data);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                        return;
                    }
                }
            }

            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }

    /// Check if LM Studio is available
    pub async fn health_check(&self) -> Result<bool> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .send()
            .await;

        match response {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    /// Get available models
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .send()
            .await
            .context("Failed to fetch models")?;

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelInfo>,
        }

        #[derive(Deserialize)]
        struct ModelInfo {
            id: String,
        }

        let models: ModelsResponse = response.json().await?;
        Ok(models.data.into_iter().map(|m| m.id).collect())
    }

    /// Perform OCR on an image using the vision model
    ///
    /// Takes a base64-encoded image and returns the extracted text.
    /// Mime type defaults to "image/jpeg" if not specified.
    pub async fn ocr_image(&self, image_base64: &str) -> Result<String> {
        self.ocr_image_with_mime(image_base64, "image/jpeg").await
    }

    /// Perform OCR on an image with explicit mime type
    pub async fn ocr_image_with_mime(&self, image_base64: &str, mime_type: &str) -> Result<String> {
        use crate::llm::types::{VisionChatRequest, VisionMessage};

        let request = VisionChatRequest {
            model: self.model.clone(),
            messages: vec![
                VisionMessage::system(
                    "You are an expert OCR assistant specialized in financial documents. \
                     Extract ALL text from the image exactly as it appears. Rules:\n\
                     - Preserve all numbers exactly (amounts, percentages, dates)\n\
                     - Reproduce tables using aligned columns with spaces\n\
                     - Keep headers and section titles\n\
                     - Output ONLY the extracted text, no commentary\n\
                     - For financial forms, extract every field and value"
                ),
                VisionMessage::user_with_image_mime(
                    "Extract all text from this document. Preserve structure and all numerical values.",
                    image_base64,
                    mime_type,
                ),
            ],
            max_tokens: Some(self.max_tokens),
            temperature: Some(0.1), // Low temperature for accurate OCR
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send OCR request to LM Studio")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio vision API error ({}): {}", status, text);
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .context("Failed to parse OCR response")?;

        let text = completion
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        // Clean up model artifacts (thinking tags, box markers)
        let text = clean_ocr_output(&text);

        Ok(text)
    }

    /// Perform OCR on an image with a custom prompt
    ///
    /// This allows different extraction strategies (TXT vs CSV) by specifying
    /// different prompts for the vision model.
    pub async fn ocr_image_with_prompt(&self, image_base64: &str, mime_type: &str, prompt: &str) -> Result<String> {
        use crate::llm::types::{VisionChatRequest, VisionMessage};

        let request = VisionChatRequest {
            model: self.model.clone(),
            messages: vec![
                VisionMessage::system(prompt),
                VisionMessage::user_with_image_mime(
                    "Process this document image according to the instructions.",
                    image_base64,
                    mime_type,
                ),
            ],
            max_tokens: Some(self.max_tokens),
            temperature: Some(0.1), // Low temperature for accurate extraction
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send OCR request to LM Studio")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio vision API error ({}): {}", status, text);
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .context("Failed to parse OCR response")?;

        let text = completion
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        // Clean up model artifacts (thinking tags, box markers)
        let text = clean_ocr_output(&text);

        Ok(text)
    }
}
