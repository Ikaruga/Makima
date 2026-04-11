//! Conversation history management

use crate::llm::types::{Message, Role, ToolCall};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A conversation session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Unique conversation ID
    pub id: String,
    /// Conversation title (auto-generated from first message)
    pub title: String,
    /// When the conversation was created
    pub created_at: DateTime<Utc>,
    /// When the conversation was last updated
    pub updated_at: DateTime<Utc>,
    /// System prompt
    pub system_prompt: Option<String>,
    /// Message history
    pub messages: Vec<Message>,
    /// Maximum messages to keep in context
    pub max_messages: usize,
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: "New Conversation".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            system_prompt: None,
            messages: Vec::new(),
            max_messages: 50,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_max_messages(mut self, max: usize) -> Self {
        self.max_messages = max;
        self
    }

    /// Add a user message
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        let content = content.into();

        // Update title from first user message
        if self.messages.is_empty() || self.title == "New Conversation" {
            self.title = Self::generate_title(&content);
        }

        self.messages.push(Message::user(content));
        self.updated_at = Utc::now();
        self.trim_messages();
    }

    /// Add an assistant message
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message::assistant(content));
        self.updated_at = Utc::now();
        self.trim_messages();
    }

    /// Add an assistant message with tool calls
    pub fn add_assistant_tool_calls(&mut self, content: Option<String>, tool_calls: Vec<ToolCall>) {
        self.messages.push(Message::assistant_with_tool_calls(content, tool_calls));
        self.updated_at = Utc::now();
    }

    /// Add a tool result
    pub fn add_tool_result(&mut self, tool_call_id: impl Into<String>, result: impl Into<String>) {
        self.messages.push(Message::tool_result(tool_call_id, result));
        self.updated_at = Utc::now();
    }

    /// Get all messages for the API (including system prompt)
    pub fn get_messages(&self) -> Vec<Message> {
        let mut messages = Vec::new();

        if let Some(ref prompt) = self.system_prompt {
            messages.push(Message::system(prompt.clone()));
        }

        messages.extend(self.messages.clone());
        messages
    }

    /// Get the last message
    pub fn last_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    /// Get the last assistant message content
    pub fn last_assistant_content(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .and_then(|m| m.content.as_deref())
    }

    /// Clear the conversation
    pub fn clear(&mut self) {
        self.messages.clear();
        self.title = "New Conversation".to_string();
        self.updated_at = Utc::now();
    }

    /// Trim messages if over limit
    fn trim_messages(&mut self) {
        if self.messages.len() > self.max_messages {
            let excess = self.messages.len() - self.max_messages;
            self.messages.drain(0..excess);
        }
    }

    /// Generate a title from content
    fn generate_title(content: &str) -> String {
        let words: Vec<&str> = content.split_whitespace().take(6).collect();
        let title = words.join(" ");
        if title.len() > 50 {
            format!("{}...", &title[..47])
        } else if title.is_empty() {
            "New Conversation".to_string()
        } else {
            title
        }
    }

    /// Get message count
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Estimate token count (rough approximation)
    pub fn estimate_tokens(&self) -> usize {
        let mut chars = 0;
        if let Some(ref prompt) = self.system_prompt {
            chars += prompt.len();
        }
        for msg in &self.messages {
            if let Some(ref content) = msg.content {
                chars += content.len();
            }
        }
        // Rough approximation: ~4 chars per token
        chars / 4
    }
}

/// Manager for multiple conversations
#[derive(Debug, Default)]
pub struct ConversationManager {
    conversations: Vec<Conversation>,
    current_index: Option<usize>,
}

impl ConversationManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new conversation and make it current
    pub fn new_conversation(&mut self) -> &mut Conversation {
        let conv = Conversation::new();
        self.conversations.push(conv);
        let idx = self.conversations.len() - 1;
        self.current_index = Some(idx);
        // Safe: we just pushed and idx is the last valid index
        &mut self.conversations[idx]
    }

    /// Get the current conversation, creating one if needed
    pub fn current(&self) -> Option<&Conversation> {
        self.current_index.and_then(|i| self.conversations.get(i))
    }

    /// Get the current conversation mutably
    pub fn current_mut(&mut self) -> Option<&mut Conversation> {
        self.current_index.and_then(|i| self.conversations.get_mut(i))
    }

    /// Get or create current conversation
    pub fn current_or_new(&mut self) -> &mut Conversation {
        if self.current_index.is_none() {
            return self.new_conversation();
        }
        // Safe: current_index is Some, verified by the check above
        let idx = self.current_index.expect("current_index checked above");
        &mut self.conversations[idx]
    }

    /// Switch to a conversation by index
    pub fn switch_to(&mut self, index: usize) -> bool {
        if index < self.conversations.len() {
            self.current_index = Some(index);
            true
        } else {
            false
        }
    }

    /// List all conversations
    pub fn list(&self) -> Vec<(usize, &str, &str)> {
        self.conversations
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.id.as_str(), c.title.as_str()))
            .collect()
    }

    /// Delete a conversation by index
    pub fn delete(&mut self, index: usize) -> bool {
        if index < self.conversations.len() {
            self.conversations.remove(index);
            if let Some(current) = self.current_index {
                if current == index {
                    self.current_index = None;
                } else if current > index {
                    self.current_index = Some(current - 1);
                }
            }
            true
        } else {
            false
        }
    }
}
