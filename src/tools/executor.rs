//! Tool executor with confirmation support

use super::registry::{ToolRegistry, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Callback type for asking user confirmation
pub type ConfirmationCallback = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Tool executor that handles confirmation and execution
pub struct ToolExecutor {
    registry: ToolRegistry,
    /// Tools that have been approved for the session (skip confirmation)
    approved_tools: Arc<Mutex<HashSet<String>>>,
    /// Whether to require confirmation for tools that need it
    require_confirmation: bool,
    /// Confirmation callback
    confirm_fn: Option<Arc<dyn Fn(&str, &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> + Send + Sync>>,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            approved_tools: Arc::new(Mutex::new(HashSet::new())),
            require_confirmation: true,
            confirm_fn: None,
        }
    }

    /// Disable confirmation (for testing or automated use)
    pub fn without_confirmation(mut self) -> Self {
        self.require_confirmation = false;
        self
    }

    /// Set a custom confirmation callback
    pub fn with_confirmation<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = bool> + Send + 'static,
    {
        self.confirm_fn = Some(Arc::new(move |tool_name: &str, summary: &str| {
            let fut = f(tool_name, summary);
            Box::pin(fut) as std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>
        }));
        self
    }

    /// Mark a tool as approved for the session
    pub async fn approve_tool(&self, tool_name: &str) {
        self.approved_tools.lock().await.insert(tool_name.to_string());
    }

    /// Check if a tool is approved
    pub async fn is_approved(&self, tool_name: &str) -> bool {
        self.approved_tools.lock().await.contains(tool_name)
    }

    /// Execute a tool call, handling confirmation if needed
    pub async fn execute(&self, call: &ParsedToolCall) -> Result<ExecutionResult> {
        let tool = self.registry.get(&call.name).ok_or_else(|| {
            anyhow::anyhow!("Unknown tool: {}", call.name)
        })?;

        let summary = tool.summarize_call(call);

        // Check if confirmation is needed
        if self.require_confirmation && tool.requires_confirmation() {
            if !self.is_approved(&call.name).await {
                // Need confirmation
                if let Some(ref confirm_fn) = self.confirm_fn {
                    let approved = confirm_fn(&call.name, &summary).await;
                    if !approved {
                        return Ok(ExecutionResult::Denied {
                            tool_name: call.name.clone(),
                            summary,
                        });
                    }
                } else {
                    // No confirmation callback, deny by default
                    return Ok(ExecutionResult::NeedsConfirmation {
                        tool_name: call.name.clone(),
                        summary,
                        call: call.clone(),
                    });
                }
            }
        }

        // Execute the tool
        match tool.execute(call).await {
            Ok(result) => Ok(ExecutionResult::Success {
                tool_name: call.name.clone(),
                result,
            }),
            Err(e) => Ok(ExecutionResult::Error {
                tool_name: call.name.clone(),
                error: e.to_string(),
            }),
        }
    }

    /// Execute multiple tool calls
    pub async fn execute_all(&self, calls: &[ParsedToolCall]) -> Vec<ExecutionResult> {
        let mut results = Vec::new();
        for call in calls {
            results.push(self.execute(call).await.unwrap_or_else(|e| ExecutionResult::Error {
                tool_name: call.name.clone(),
                error: e.to_string(),
            }));
        }
        results
    }

    /// Get the tool registry
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

/// Result of tool execution
#[derive(Debug)]
pub enum ExecutionResult {
    /// Tool executed successfully
    Success {
        tool_name: String,
        result: ToolResult,
    },
    /// Tool needs user confirmation before executing
    NeedsConfirmation {
        tool_name: String,
        summary: String,
        call: ParsedToolCall,
    },
    /// User denied the tool execution
    Denied {
        tool_name: String,
        summary: String,
    },
    /// Tool execution failed
    Error {
        tool_name: String,
        error: String,
    },
}

impl ExecutionResult {
    /// Convert to a string suitable for sending back to the LLM
    pub fn to_llm_response(&self) -> String {
        match self {
            ExecutionResult::Success { tool_name, result } => {
                if result.success {
                    result.content.clone()
                } else {
                    format!("Erreur lors de l'execution de {}: {}", tool_name, result.content)
                }
            }
            ExecutionResult::NeedsConfirmation { tool_name, summary, .. } => {
                format!("L'outil {} necessite une confirmation: {}", tool_name, summary)
            }
            ExecutionResult::Denied { tool_name, summary } => {
                format!("L'utilisateur a refuse l'execution de {}: {}", tool_name, summary)
            }
            ExecutionResult::Error { tool_name, error } => {
                format!("Erreur lors de l'execution de {}: {}", tool_name, error)
            }
        }
    }

    /// Check if this was a successful execution
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionResult::Success { result, .. } if result.success)
    }
}
