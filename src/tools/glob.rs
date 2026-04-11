//! Glob pattern file search tool

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use glob::glob as glob_match;
use serde_json::json;
use std::path::Path;

pub struct GlobTool {
    working_dir: String,
}

impl GlobTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Trouver des fichiers par motif glob. Ex: '**/*.rs', 'src/*.py'"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Motif glob pour filtrer (ex: '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Répertoire de base (défaut: espace de travail)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de résultats (défaut: 100)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let pattern = args
            .get_string("pattern")
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

        let base_path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        let max_results = args
            .arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        let search_base = if Path::new(&base_path).is_absolute() {
            base_path.clone()
        } else {
            format!("{}/{}", self.working_dir, base_path)
        };

        // Build the full pattern
        let full_pattern = if pattern.starts_with('/') || pattern.contains(':') {
            pattern.clone()
        } else {
            format!("{}/{}", search_base, pattern)
        };

        // Normalize path separators for Windows
        let full_pattern = full_pattern.replace('\\', "/");

        let mut matches = Vec::new();

        match glob_match(&full_pattern) {
            Ok(paths) => {
                for entry in paths.take(max_results) {
                    match entry {
                        Ok(path) => {
                            // Try to make path relative to working dir
                            let display_path = path
                                .strip_prefix(&self.working_dir)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| path.to_string_lossy().to_string());

                            let prefix = if path.is_dir() { "📁" } else { "📄" };
                            matches.push(format!("{} {}", prefix, display_path));
                        }
                        Err(e) => {
                            tracing::warn!("Glob error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                return Ok(ToolResult::error(format!("Invalid glob pattern: {}", e)));
            }
        }

        if matches.is_empty() {
            Ok(ToolResult::success(format!(
                "No files found matching pattern: {}",
                pattern
            )))
        } else {
            let output = format!(
                "Found {} file(s) matching '{}':\n\n{}",
                matches.len(),
                pattern,
                matches.join("\n")
            );
            Ok(ToolResult::success(output))
        }
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let pattern = args.get_string("pattern").unwrap_or_else(|| "<unknown>".to_string());
        format!("Find files matching: {}", pattern)
    }
}
