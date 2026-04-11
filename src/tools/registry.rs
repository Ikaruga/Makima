//! Tool registry for managing available tools

use crate::llm::client::LmStudioClient;
use crate::llm::types::{ParsedToolCall, ToolDefinition};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Get the directory where the executable is located
fn get_exe_directory() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| p.to_string_lossy().to_string())
}

/// Trait that all tools must implement
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> &str;

    /// Get the tool description
    fn description(&self) -> &str;

    /// Get the JSON schema for parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with given arguments
    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult>;

    /// Whether this tool requires confirmation before execution
    fn requires_confirmation(&self) -> bool {
        false
    }

    /// Get a human-readable summary of what this call will do
    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        format!("Execute {} with {:?}", self.name(), args.arguments)
    }

    /// Convert to ToolDefinition for API
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(self.name(), self.description(), self.parameters_schema())
    }
}

/// Result from tool execution
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Whether the execution was successful
    pub success: bool,
    /// The output content
    pub content: String,
    /// Optional structured data
    pub data: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
            data: None,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            success: false,
            content: content.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    working_dir: String,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            working_dir: String::new(),
        }
    }

    /// Register a new tool
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Get all tool definitions for the API
    pub fn get_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    /// Get all tool names
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Get the working directory
    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }

    /// Create a registry with all default tools
    pub fn with_defaults(working_dir: Option<String>) -> Self {
        Self::with_defaults_and_client(working_dir, None)
    }

    /// Create a registry with all default tools and an optional LM Studio client for OCR
    pub fn with_defaults_and_client(
        working_dir: Option<String>,
        client: Option<Arc<LmStudioClient>>,
    ) -> Self {
        // Priorité: 1) paramètre explicite, 2) exe dir, 3) current dir
        let work_dir = working_dir
            .or_else(get_exe_directory)
            .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| ".".to_string());

        let mut registry = Self {
            tools: HashMap::new(),
            working_dir: work_dir.clone(),
        };

        // Register file operations
        registry.register(super::file_ops::ReadFileTool::new(work_dir.clone()));
        registry.register(super::file_ops::WriteFileTool::new(work_dir.clone()));
        registry.register(super::file_ops::EditFileTool::new(work_dir.clone()));
        registry.register(super::file_ops::DeleteTool::new(work_dir.clone()));
        registry.register(super::file_ops::ListDirectoryTool::new(work_dir.clone()));

        // Register search tools
        registry.register(super::glob::GlobTool::new(work_dir.clone()));
        registry.register(super::grep::GrepTool::new(work_dir.clone()));

        // Register bash
        registry.register(super::bash::BashTool::new(work_dir.clone()));

        // Register CSV to DOCX conversion
        registry.register(super::csv_to_docx::CsvToDocxTool::new(work_dir.clone()));

        // Register liasse fiscale formatter
        registry.register(super::format_liasse::FormatLiasseFiscaleTool::new(work_dir.clone()));

        // Register PDF to TXT extraction (with optional OCR support)
        if let Some(client) = client {
            registry.register(super::pdf_to_txt::PdfToTxtTool::with_client(
                work_dir,
                client,
            ));
        } else {
            registry.register(super::pdf_to_txt::PdfToTxtTool::new(work_dir));
        }

        registry
    }

    /// Create a registry with Akari tools (enhanced, optimized for GLM-4.6V)
    pub fn with_akari_tools(
        working_dir: Option<String>,
        client: Option<Arc<LmStudioClient>>,
    ) -> Self {
        let work_dir = working_dir
            .or_else(get_exe_directory)
            .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| ".".to_string());

        let mut registry = Self {
            tools: HashMap::new(),
            working_dir: work_dir.clone(),
        };

        // Outils Akari ameliores (remplacent les standards)
        registry.register(super::akari_tools::AkariReadFile::new(work_dir.clone()));
        registry.register(super::akari_tools::AkariWriteFile::new(work_dir.clone()));
        registry.register(super::akari_tools::AkariEditFile::new(work_dir.clone()));
        registry.register(super::akari_tools::AkariBash::new(work_dir.clone()));
        registry.register(super::akari_tools::AkariGlob::new(work_dir.clone()));
        registry.register(super::akari_tools::AkariGrep::new(work_dir.clone()));

        // Outils standards conserves tels quels
        registry.register(super::file_ops::DeleteTool::new(work_dir.clone()));
        registry.register(super::file_ops::ListDirectoryTool::new(work_dir.clone()));
        registry.register(super::csv_to_docx::CsvToDocxTool::new(work_dir.clone()));
        registry.register(super::format_liasse::FormatLiasseFiscaleTool::new(work_dir.clone()));

        // Nouveaux outils Akari
        registry.register(super::akari_tools::AkariWebFetch::new());
        registry.register(super::akari_tools::AkariWebSearch::new());

        // PDF avec OCR (inchange)
        if let Some(client) = client {
            registry.register(super::pdf_to_txt::PdfToTxtTool::with_client(work_dir, client));
        } else {
            registry.register(super::pdf_to_txt::PdfToTxtTool::new(work_dir));
        }

        registry
    }
}
