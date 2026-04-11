//! Streaming response handling

use crate::llm::client::StreamEvent;
use crate::llm::types::{ParsedToolCall, ToolCall};
use crate::llm::tool_parser::ToolParser;
use tokio::sync::mpsc;

/// Accumulated state from a streaming response
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    /// Accumulated text content
    pub content: String,
    /// Accumulated tool calls (from native API)
    pub tool_calls: Vec<ToolCall>,
    /// Whether the stream is complete
    pub done: bool,
    /// Any error that occurred
    pub error: Option<String>,
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event and update state
    pub fn process_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Content(text) => {
                self.content.push_str(&text);
            }
            StreamEvent::ToolCallComplete(tc) => {
                self.tool_calls.push(tc);
            }
            StreamEvent::Done => {
                self.done = true;
            }
            StreamEvent::Error(e) => {
                self.error = Some(e);
                self.done = true;
            }
            _ => {}
        }
    }

    /// Get all parsed tool calls (native + fallback)
    pub fn get_tool_calls(&self, parser: &ToolParser) -> Vec<ParsedToolCall> {
        let mut calls = Vec::new();

        // First, try native tool calls
        if !self.tool_calls.is_empty() {
            calls.extend(parser.parse_native(&self.tool_calls));
        }

        // Then, try fallback parsing from content
        if calls.is_empty() && parser.contains_tool_calls(&self.content) {
            calls.extend(parser.parse_from_text(&self.content));
        }

        calls
    }

    /// Get the text content without tool call tags
    pub fn get_clean_content(&self, parser: &ToolParser) -> String {
        if parser.contains_tool_calls(&self.content) {
            parser.extract_text_before_tools(&self.content)
        } else {
            self.content.clone()
        }
    }
}

/// A helper to consume a stream and accumulate results
pub struct StreamConsumer {
    pub accumulator: StreamAccumulator,
    parser: ToolParser,
}

impl StreamConsumer {
    pub fn new() -> Self {
        Self {
            accumulator: StreamAccumulator::new(),
            parser: ToolParser::new(),
        }
    }

    /// Consume the entire stream, calling the callback for each content chunk
    pub async fn consume<F>(
        &mut self,
        mut rx: mpsc::Receiver<StreamEvent>,
        mut on_content: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(&str),
    {
        while let Some(event) = rx.recv().await {
            match &event {
                StreamEvent::Content(text) => {
                    on_content(text);
                }
                StreamEvent::Error(e) => {
                    return Err(anyhow::anyhow!("{}", e));
                }
                _ => {}
            }
            self.accumulator.process_event(event);

            if self.accumulator.done {
                break;
            }
        }

        Ok(())
    }

    /// Get parsed tool calls
    pub fn get_tool_calls(&self) -> Vec<ParsedToolCall> {
        self.accumulator.get_tool_calls(&self.parser)
    }

    /// Get clean content
    pub fn get_clean_content(&self) -> String {
        self.accumulator.get_clean_content(&self.parser)
    }
}

impl Default for StreamConsumer {
    fn default() -> Self {
        Self::new()
    }
}
