//! File operation tools: read, write, edit, list

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Resolve a path relative to the working directory
fn resolve_path(working_dir: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}

/// Add line numbers to content
#[allow(dead_code)]
fn add_line_numbers(content: &str) -> String {
    content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:4} | {}", i + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// ReadFileTool
// ============================================================================

pub struct ReadFileTool {
    working_dir: String,
}

impl ReadFileTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Lire le contenu d'un fichier. Retourne le contenu avec numéros de ligne."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Chemin du fichier à lire (absolu ou relatif à l'espace de travail)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optionnel: Commencer à la ligne (index 1)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Optionnel: Arrêter à la ligne (inclus)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path").ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!("Fichier introuvable: {}", resolved.display())));
        }

        let content = fs::read_to_string(&resolved)
            .await
            .map_err(|e| anyhow::anyhow!("Échec de lecture: {}", e))?;

        let lines: Vec<&str> = content.lines().collect();
        let start = args
            .arguments
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).saturating_sub(1))
            .unwrap_or(0);
        let end = args
            .arguments
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(lines.len());

        let selected: Vec<String> = lines
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start)
            .map(|(i, line)| format!("{:4} | {}", i + 1, line))
            .collect();

        let output = format!(
            "File: {}\nLines: {}-{} of {}\n\n{}",
            resolved.display(),
            start + 1,
            end.min(lines.len()),
            lines.len(),
            selected.join("\n")
        );

        Ok(ToolResult::success(output))
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<unknown>".to_string());
        format!("Read file: {}", path)
    }
}

// ============================================================================
// WriteFileTool
// ============================================================================

pub struct WriteFileTool {
    working_dir: String,
}

impl WriteFileTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Écrire du contenu dans un fichier. Crée ou écrase le fichier."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Chemin du fichier à écrire"
                },
                "content": {
                    "type": "string",
                    "description": "Contenu à écrire dans le fichier"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path").ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let content = args.get_string("content").ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let resolved = resolve_path(&self.working_dir, &path);

        // Create parent directories if needed
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&resolved, &content).await?;

        let line_count = content.lines().count();
        Ok(ToolResult::success(format!(
            "Successfully wrote {} lines to {}",
            line_count,
            resolved.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<unknown>".to_string());
        let content = args.get_string("content").unwrap_or_default();
        let line_count = content.lines().count();
        format!("Write {} lines to file: {}", line_count, path)
    }
}

// ============================================================================
// EditFileTool
// ============================================================================

pub struct EditFileTool {
    working_dir: String,
}

impl EditFileTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Modifier un fichier en remplaçant une chaîne exacte par une autre."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false, only replace first)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path").ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let old_string = args.get_string("old_string").ok_or_else(|| anyhow::anyhow!("Missing 'old_string' parameter"))?;
        let new_string = args.get_string("new_string").ok_or_else(|| anyhow::anyhow!("Missing 'new_string' parameter"))?;
        let replace_all = args.get_bool("replace_all").unwrap_or(false);

        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!("File not found: {}", resolved.display())));
        }

        let content = fs::read_to_string(&resolved).await?;

        if !content.contains(&old_string) {
            return Ok(ToolResult::error(format!(
                "Chaîne non trouvée dans le fichier. old_string doit correspondre exactement.\nRecherche:\n{}\n\nDans: {}",
                old_string,
                resolved.display()
            )));
        }

        let occurrences = content.matches(&old_string).count();

        let new_content = if replace_all {
            content.replace(&old_string, &new_string)
        } else {
            content.replacen(&old_string, &new_string, 1)
        };

        fs::write(&resolved, &new_content).await?;

        let replaced_count = if replace_all { occurrences } else { 1 };
        Ok(ToolResult::success(format!(
            "Successfully replaced {} occurrence(s) in {}",
            replaced_count,
            resolved.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<unknown>".to_string());
        let old_str = args.get_string("old_string").unwrap_or_default();
        let preview = if old_str.len() > 50 {
            format!("{}...", &old_str[..50])
        } else {
            old_str
        };
        format!("Edit file {}: replace \"{}\"", path, preview)
    }
}

// ============================================================================
// DeleteTool
// ============================================================================

pub struct DeleteTool {
    working_dir: String,
}

impl DeleteTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for DeleteTool {
    fn name(&self) -> &str {
        "delete"
    }

    fn description(&self) -> &str {
        "Supprimer un fichier ou dossier. Utiliser recursive=true pour les dossiers non vides."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory to delete"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "For directories: delete recursively including contents (default: false)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path").ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let recursive = args.get_bool("recursive").unwrap_or(false);

        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!("Chemin introuvable: {}", resolved.display())));
        }

        if resolved.is_dir() {
            if recursive {
                fs::remove_dir_all(&resolved).await?;
                Ok(ToolResult::success(format!(
                    "Successfully deleted directory and all contents: {}",
                    resolved.display()
                )))
            } else {
                // Try to remove empty directory
                match fs::remove_dir(&resolved).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Successfully deleted empty directory: {}",
                        resolved.display()
                    ))),
                    Err(e) => {
                        // Check if directory is not empty using multiple methods:
                        // 1. Standard ErrorKind (works on most Unix systems)
                        // 2. Raw OS error code 145 on Windows (ERROR_DIR_NOT_EMPTY)
                        // 3. Raw OS error code 39 on Unix (ENOTEMPTY)
                        let is_not_empty = e.kind() == std::io::ErrorKind::DirectoryNotEmpty
                            || e.raw_os_error() == Some(145)  // Windows ERROR_DIR_NOT_EMPTY
                            || e.raw_os_error() == Some(39);  // Unix ENOTEMPTY

                        if is_not_empty {
                            Ok(ToolResult::error(format!(
                                "Répertoire non vide: {}. Utilisez recursive=true pour supprimer avec le contenu.",
                                resolved.display()
                            )))
                        } else {
                            Err(e.into())
                        }
                    }
                }
            }
        } else {
            fs::remove_file(&resolved).await?;
            Ok(ToolResult::success(format!(
                "Successfully deleted file: {}",
                resolved.display()
            )))
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<unknown>".to_string());
        let recursive = args.get_bool("recursive").unwrap_or(false);
        if recursive {
            format!("Delete recursively: {}", path)
        } else {
            format!("Delete: {}", path)
        }
    }
}

// ============================================================================
// ListDirectoryTool
// ============================================================================

pub struct ListDirectoryTool {
    working_dir: String,
}

impl ListDirectoryTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "Lister le contenu d'un répertoire."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list (default: working directory)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "List recursively (default: false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing (default: 3)"
                }
            }
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        let recursive = args.get_bool("recursive").unwrap_or(false);
        let max_depth = args
            .arguments
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!("Directory not found: {}", resolved.display())));
        }

        if !resolved.is_dir() {
            return Ok(ToolResult::error(format!("Not a directory: {}", resolved.display())));
        }

        let mut entries = Vec::new();

        if recursive {
            for entry in walkdir::WalkDir::new(&resolved)
                .max_depth(max_depth)
                .sort_by_file_name()
            {
                if let Ok(entry) = entry {
                    let rel_path = entry.path().strip_prefix(&resolved).unwrap_or(entry.path());
                    let prefix = if entry.file_type().is_dir() { "📁" } else { "📄" };
                    entries.push(format!("{} {}", prefix, rel_path.display()));
                }
            }
        } else {
            let mut dir = fs::read_dir(&resolved).await?;
            let mut items = Vec::new();
            while let Some(entry) = dir.next_entry().await? {
                items.push(entry);
            }
            items.sort_by_key(|e| e.file_name());

            for entry in items {
                let file_type = entry.file_type().await?;
                let prefix = if file_type.is_dir() { "📁" } else { "📄" };
                entries.push(format!("{} {}", prefix, entry.file_name().to_string_lossy()));
            }
        }

        let output = format!(
            "Directory: {}\n\n{}",
            resolved.display(),
            entries.join("\n")
        );

        Ok(ToolResult::success(output))
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        format!("List directory: {}", path)
    }
}
