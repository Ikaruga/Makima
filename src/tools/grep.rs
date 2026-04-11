//! Grep-like content search tool

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::path::Path;
use tokio::fs;
use walkdir::WalkDir;

pub struct GrepTool {
    working_dir: String,
}

impl GrepTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Rechercher un motif dans les fichiers. Supporte les regex."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Expression régulière à rechercher"
                },
                "path": {
                    "type": "string",
                    "description": "Fichier ou répertoire à parcourir (défaut: espace de travail)"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Motif glob pour filtrer les fichiers (ex: '*.rs')"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Recherche insensible à la casse (défaut: false)"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lignes de contexte avant/après (défaut: 0)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de correspondances (défaut: 50)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let pattern = args
            .get_string("pattern")
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

        let search_path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        let file_pattern = args.get_string("file_pattern");
        let case_insensitive = args.get_bool("case_insensitive").unwrap_or(false);
        let context_lines = args
            .arguments
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let max_results = args
            .arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // Build regex
        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.clone()
        };

        let regex = match Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!("Invalid regex pattern: {}", e)));
            }
        };

        // Build file pattern matcher
        let file_glob = file_pattern.as_ref().and_then(|p| glob::Pattern::new(p).ok());

        // Resolve search path
        let resolved = if Path::new(&search_path).is_absolute() {
            Path::new(&search_path).to_path_buf()
        } else {
            Path::new(&self.working_dir).join(&search_path)
        };

        let mut results = Vec::new();
        let mut files_searched = 0;

        // Collect files to search
        let files: Vec<_> = if resolved.is_file() {
            vec![resolved.clone()]
        } else {
            WalkDir::new(&resolved)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    if let Some(ref glob) = file_glob {
                        glob.matches(e.file_name().to_str().unwrap_or(""))
                    } else {
                        true
                    }
                })
                .map(|e| e.path().to_path_buf())
                .collect()
        };

        'outer: for file_path in files {
            files_searched += 1;

            // Skip binary files
            let content = match fs::read_to_string(&file_path).await {
                Ok(c) => c,
                Err(_) => continue, // Skip files we can't read
            };

            let lines: Vec<&str> = content.lines().collect();

            for (line_num, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    let rel_path = file_path
                        .strip_prefix(&self.working_dir)
                        .unwrap_or(&file_path)
                        .to_string_lossy();

                    let mut match_output = String::new();
                    match_output.push_str(&format!("{}:{}:\n", rel_path, line_num + 1));

                    // Add context before
                    let start = line_num.saturating_sub(context_lines);
                    for i in start..line_num {
                        match_output.push_str(&format!("  {:4} | {}\n", i + 1, lines[i]));
                    }

                    // Add matching line with highlight marker
                    match_output.push_str(&format!("→ {:4} | {}\n", line_num + 1, line));

                    // Add context after
                    let end = (line_num + 1 + context_lines).min(lines.len());
                    for i in (line_num + 1)..end {
                        match_output.push_str(&format!("  {:4} | {}\n", i + 1, lines[i]));
                    }

                    results.push(match_output);

                    if results.len() >= max_results {
                        break 'outer;
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(ToolResult::success(format!(
                "No matches found for '{}' in {} file(s)",
                pattern, files_searched
            )))
        } else {
            let output = format!(
                "Found {} match(es) for '{}' in {} file(s):\n\n{}",
                results.len(),
                pattern,
                files_searched,
                results.join("\n")
            );
            Ok(ToolResult::success(output))
        }
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let pattern = args.get_string("pattern").unwrap_or_else(|| "<unknown>".to_string());
        let path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        format!("Search for '{}' in {}", pattern, path)
    }
}
