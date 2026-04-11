//! Outils Akari (灯) — Optimises pour GLM-4.6V avec function calling natif
//! Inspires de l'approche Claude Code, avec des schemas ameliores
//! et des descriptions optimisees pour les modeles de vision.

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::fs;
use tokio::process::Command;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use walkdir::WalkDir;

/// Resolve a path relative to the working directory
fn resolve_path(working_dir: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}

// ============================================================================
// AkariReadFile — Lecture amelioree avec offset/limit
// ============================================================================

pub struct AkariReadFile {
    working_dir: String,
}

impl AkariReadFile {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Lire le contenu d'un fichier depuis le systeme de fichiers local. \
         Retourne le contenu avec numeros de ligne au format cat -n. \
         Supporte la lecture partielle avec offset et limit pour les gros fichiers. \
         Pour les fichiers tres longs (>2000 lignes), utiliser offset/limit."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Chemin du fichier a lire (absolu ou relatif). Ne PAS utiliser pour les repertoires."
                },
                "offset": {
                    "type": "integer",
                    "description": "Numero de ligne de depart (index 1). Utile pour les gros fichiers."
                },
                "limit": {
                    "type": "integer",
                    "description": "Nombre maximum de lignes a lire depuis offset."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'path' manquant"))?;
        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!(
                "Fichier introuvable: {}", resolved.display()
            )));
        }

        let content = fs::read_to_string(&resolved).await
            .map_err(|e| anyhow::anyhow!("Echec de lecture: {}", e))?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let offset = args.arguments.get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).saturating_sub(1))
            .unwrap_or(0);

        let limit = args.arguments.get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(total.saturating_sub(offset));

        let selected: Vec<String> = lines.iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(i, line)| format!("{:>6}\t{}", i + 1, line))
            .collect();

        let end_line = (offset + limit).min(total);
        let output = format!(
            "File: {}\nLines: {}-{} of {}\n\n{}",
            resolved.display(),
            offset + 1,
            end_line,
            total,
            selected.join("\n")
        );

        Ok(ToolResult::success(output))
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<inconnu>".to_string());
        format!("Lire: {}", path)
    }
}

// ============================================================================
// AkariWriteFile — Ecriture sans confirmation
// ============================================================================

pub struct AkariWriteFile {
    working_dir: String,
}

impl AkariWriteFile {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariWriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Ecrire du contenu dans un fichier. Cree le fichier et les repertoires parents \
         si necessaire. Ecrase le fichier existant. TOUJOURS lire un fichier avant de \
         le modifier pour comprendre le contenu existant."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Chemin du fichier a ecrire (absolu ou relatif)"
                },
                "content": {
                    "type": "string",
                    "description": "Contenu complet a ecrire dans le fichier"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'path' manquant"))?;
        let content = args.get_string("content")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'content' manquant"))?;

        let resolved = resolve_path(&self.working_dir, &path);

        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&resolved, &content).await?;

        let line_count = content.lines().count();
        Ok(ToolResult::success(format!(
            "Ecrit {} lignes dans {}", line_count, resolved.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true // write_file: confirmation requise
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<inconnu>".to_string());
        let content = args.get_string("content").unwrap_or_default();
        let line_count = content.lines().count();
        format!("Ecrire {} lignes dans: {}", line_count, path)
    }
}

// ============================================================================
// AkariEditFile — Edition avec validation d'unicite
// ============================================================================

pub struct AkariEditFile {
    working_dir: String,
}

impl AkariEditFile {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariEditFile {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Modifier un fichier en remplacant une chaine exacte par une autre. \
         L'old_string DOIT etre suffisamment unique pour identifier une seule occurrence. \
         Si old_string apparait plusieurs fois et replace_all est false, l'operation echouera. \
         Inclure du contexte supplementaire (lignes autour) si necessaire pour garantir l'unicite."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Chemin du fichier a modifier"
                },
                "old_string": {
                    "type": "string",
                    "description": "Chaine exacte a trouver (doit etre UNIQUE dans le fichier sauf si replace_all=true)"
                },
                "new_string": {
                    "type": "string",
                    "description": "Chaine de remplacement (doit etre differente de old_string)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Remplacer toutes les occurrences (defaut: false — une seule)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let path = args.get_string("path")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'path' manquant"))?;
        let old_string = args.get_string("old_string")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'old_string' manquant"))?;
        let new_string = args.get_string("new_string")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'new_string' manquant"))?;
        let replace_all = args.get_bool("replace_all").unwrap_or(false);

        let resolved = resolve_path(&self.working_dir, &path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!(
                "Fichier introuvable: {}", resolved.display()
            )));
        }

        let content = fs::read_to_string(&resolved).await?;

        if !content.contains(&old_string) {
            return Ok(ToolResult::error(format!(
                "Chaine non trouvee dans le fichier. old_string doit correspondre EXACTEMENT.\n\
                 Recherche ({} chars):\n{}\n\nDans: {}",
                old_string.len(), old_string, resolved.display()
            )));
        }

        let occurrences = content.matches(&old_string).count();

        // Validation d'unicite — la difference Akari
        if !replace_all && occurrences > 1 {
            return Ok(ToolResult::error(format!(
                "old_string n'est PAS unique: {} occurrences trouvees dans {}.\n\
                 Fournissez plus de contexte (lignes autour) pour identifier une seule occurrence,\n\
                 ou utilisez replace_all=true pour toutes les remplacer.",
                occurrences, resolved.display()
            )));
        }

        let new_content = if replace_all {
            content.replace(&old_string, &new_string)
        } else {
            content.replacen(&old_string, &new_string, 1)
        };

        fs::write(&resolved, &new_content).await?;

        let replaced_count = if replace_all { occurrences } else { 1 };
        Ok(ToolResult::success(format!(
            "Remplace {} occurrence(s) dans {}",
            replaced_count, resolved.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true // edit_file: confirmation requise
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let path = args.get_string("path").unwrap_or_else(|| "<inconnu>".to_string());
        let old_str = args.get_string("old_string").unwrap_or_default();
        let preview = if old_str.len() > 50 {
            format!("{}...", safe_truncate(&old_str, 50))
        } else {
            old_str
        };
        format!("Editer {}: remplacer \"{}\"", path, preview)
    }
}

// ============================================================================
// AkariBash — Commandes shell avec timeout 120s, sans confirmation
// ============================================================================

pub struct AkariBash {
    working_dir: String,
}

impl AkariBash {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariBash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Executer une commande shell dans l'espace de travail. Pour git, build, test, etc. \
         Timeout par defaut: 120 secondes. Preferer les commandes simples et bien connues. \
         Ne PAS utiliser pour lire des fichiers (utiliser read_file), \
         chercher des fichiers (utiliser glob), ou chercher du contenu (utiliser grep)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "La commande shell a executer"
                },
                "description": {
                    "type": "string",
                    "description": "Description courte de ce que fait la commande (pour le log)"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Delai d'expiration en secondes (defaut: 120)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let command = args.get_string("command")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'command' manquant"))?;

        let timeout_secs = args.arguments
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);

        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut child = Command::new(shell)
            .arg(shell_arg)
            .arg(&command)
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("Echec capture stdout"))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("Echec capture stderr"))?;

        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let result = tokio::time::timeout(timeout, async {
            let stdout_task = async {
                let mut reader = BufReader::new(stdout).lines();
                let mut lines = Vec::new();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line);
                }
                lines
            };
            let stderr_task = async {
                let mut reader = BufReader::new(stderr).lines();
                let mut lines = Vec::new();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line);
                }
                lines
            };
            let (out, err) = tokio::join!(stdout_task, stderr_task);
            let status = child.wait().await;
            (out, err, status)
        }).await;

        match result {
            Ok((output_lines, error_lines, Ok(status))) => {
                let mut output = String::new();

                if !output_lines.is_empty() {
                    output.push_str("stdout:\n");
                    output.push_str(&output_lines.join("\n"));
                }

                if !error_lines.is_empty() {
                    if !output.is_empty() {
                        output.push_str("\n\n");
                    }
                    output.push_str("stderr:\n");
                    output.push_str(&error_lines.join("\n"));
                }

                if output.is_empty() {
                    output = "(pas de sortie)".to_string();
                }

                let exit_info = format!("\n\nCode de sortie: {}", status.code().unwrap_or(-1));
                output.push_str(&exit_info);

                if status.success() {
                    Ok(ToolResult::success(output))
                } else {
                    Ok(ToolResult::error(output))
                }
            }
            Ok((_, _, Err(e))) => Ok(ToolResult::error(format!("Echec d'execution: {}", e))),
            Err(_) => {
                let _ = child.kill().await;
                Ok(ToolResult::error(format!(
                    "Commande expiree apres {} secondes", timeout_secs
                )))
            }
        }
    }

    fn requires_confirmation(&self) -> bool {
        true // bash: confirmation requise
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let desc = args.get_string("description");
        let command = args.get_string("command").unwrap_or_else(|| "<inconnu>".to_string());
        if let Some(d) = desc {
            format!("Exec: {}", d)
        } else {
            let preview = if command.len() > 80 {
                format!("{}...", safe_truncate(&command, 80))
            } else {
                command
            };
            format!("Exec: {}", preview)
        }
    }
}

// ============================================================================
// AkariGlob — Recherche de fichiers amelioree
// ============================================================================

pub struct AkariGlob {
    working_dir: String,
}

impl AkariGlob {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariGlob {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Trouver des fichiers par motif glob. Exemples de patterns: \
         '**/*.rs' (tous les .rs recursif), 'src/**/*.py' (Python dans src/), \
         '*.{js,ts}' (JS et TS dans le dossier courant), '**/test_*.rs' (fichiers de test). \
         Retourne les chemins relatifs a l'espace de travail."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Motif glob (ex: '**/*.rs', 'src/**/*.py', '*.{js,ts}')"
                },
                "path": {
                    "type": "string",
                    "description": "Repertoire de base (defaut: espace de travail)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de resultats (defaut: 100)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let pattern = args.get_string("pattern")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'pattern' manquant"))?;

        let base_path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        let max_results = args.arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        let search_base = if Path::new(&base_path).is_absolute() {
            base_path.clone()
        } else {
            format!("{}/{}", self.working_dir, base_path)
        };

        let full_pattern = if pattern.starts_with('/') || pattern.contains(':') {
            pattern.clone()
        } else {
            format!("{}/{}", search_base, pattern)
        };

        let full_pattern = full_pattern.replace('\\', "/");

        let mut matches = Vec::new();

        match glob::glob(&full_pattern) {
            Ok(paths) => {
                for entry in paths.take(max_results) {
                    match entry {
                        Ok(path) => {
                            let display_path = path
                                .strip_prefix(&self.working_dir)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| path.to_string_lossy().to_string());

                            let prefix = if path.is_dir() { "D" } else { "F" };
                            matches.push(format!("[{}] {}", prefix, display_path));
                        }
                        Err(e) => {
                            tracing::warn!("Glob error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                return Ok(ToolResult::error(format!("Pattern glob invalide: {}", e)));
            }
        }

        if matches.is_empty() {
            Ok(ToolResult::success(format!(
                "Aucun fichier correspondant au pattern: {}", pattern
            )))
        } else {
            let output = format!(
                "Trouve {} fichier(s) pour '{}':\n\n{}",
                matches.len(), pattern, matches.join("\n")
            );
            Ok(ToolResult::success(output))
        }
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let pattern = args.get_string("pattern").unwrap_or_else(|| "<inconnu>".to_string());
        format!("Glob: {}", pattern)
    }
}

// ============================================================================
// AkariGrep — Recherche avec output_mode et head_limit
// ============================================================================

pub struct AkariGrep {
    working_dir: String,
}

impl AkariGrep {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for AkariGrep {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Rechercher un motif regex dans les fichiers. Supporte les expressions regulieres. \
         Trois modes de sortie: 'content' (lignes correspondantes avec contexte), \
         'files_with_matches' (chemins des fichiers uniquement), \
         'count' (nombre de correspondances par fichier)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Expression reguliere a rechercher"
                },
                "path": {
                    "type": "string",
                    "description": "Fichier ou repertoire (defaut: espace de travail)"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Filtre glob sur les noms de fichiers (ex: '*.rs', '*.py')"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Recherche insensible a la casse (defaut: false)"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Mode de sortie (defaut: 'content')"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lignes de contexte avant/apres chaque correspondance (defaut: 0)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Limiter la sortie aux N premiers resultats (defaut: 50)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let pattern = args.get_string("pattern")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'pattern' manquant"))?;

        let search_path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        let file_pattern = args.get_string("file_pattern");
        let case_insensitive = args.get_bool("case_insensitive").unwrap_or(false);
        let output_mode = args.get_string("output_mode")
            .unwrap_or_else(|| "content".to_string());
        let context_lines = args.arguments
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let head_limit = args.arguments
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.clone()
        };

        let regex = match Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!("Pattern regex invalide: {}", e)));
            }
        };

        let file_glob = file_pattern.as_ref().and_then(|p| glob::Pattern::new(p).ok());

        let resolved = if Path::new(&search_path).is_absolute() {
            Path::new(&search_path).to_path_buf()
        } else {
            Path::new(&self.working_dir).join(&search_path)
        };

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

        let mut results = Vec::new();
        let mut file_matches = Vec::new();
        let mut count_results = Vec::new();
        let mut files_searched = 0;
        let mut total_matches = 0;

        for file_path in &files {
            files_searched += 1;

            let content = match fs::read_to_string(file_path).await {
                Ok(c) => c,
                Err(_) => continue,
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut file_match_count = 0;

            for (line_num, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    file_match_count += 1;
                    total_matches += 1;

                    if output_mode == "content" && results.len() < head_limit {
                        let rel_path = file_path
                            .strip_prefix(&self.working_dir)
                            .unwrap_or(file_path)
                            .to_string_lossy();

                        let mut match_output = String::new();
                        match_output.push_str(&format!("{}:{}:\n", rel_path, line_num + 1));

                        let start = line_num.saturating_sub(context_lines);
                        for i in start..line_num {
                            match_output.push_str(&format!("  {:4} | {}\n", i + 1, lines[i]));
                        }
                        match_output.push_str(&format!("→ {:4} | {}\n", line_num + 1, line));

                        let end = (line_num + 1 + context_lines).min(lines.len());
                        for i in (line_num + 1)..end {
                            match_output.push_str(&format!("  {:4} | {}\n", i + 1, lines[i]));
                        }

                        results.push(match_output);
                    }
                }
            }

            if file_match_count > 0 {
                let rel_path = file_path
                    .strip_prefix(&self.working_dir)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();

                if file_matches.len() < head_limit {
                    file_matches.push(rel_path.clone());
                }
                if count_results.len() < head_limit {
                    count_results.push(format!("{}: {} match(es)", rel_path, file_match_count));
                }
            }
        }

        let output = match output_mode.as_str() {
            "files_with_matches" => {
                if file_matches.is_empty() {
                    format!("Aucune correspondance pour '{}' dans {} fichier(s)", pattern, files_searched)
                } else {
                    format!(
                        "{} fichier(s) contenant '{}' (sur {} parcourus):\n\n{}",
                        file_matches.len(), pattern, files_searched,
                        file_matches.join("\n")
                    )
                }
            }
            "count" => {
                if count_results.is_empty() {
                    format!("Aucune correspondance pour '{}' dans {} fichier(s)", pattern, files_searched)
                } else {
                    format!(
                        "{} correspondance(s) totale(s) pour '{}' dans {} fichier(s):\n\n{}",
                        total_matches, pattern, files_searched,
                        count_results.join("\n")
                    )
                }
            }
            _ => {
                // "content" mode (default)
                if results.is_empty() {
                    format!("Aucune correspondance pour '{}' dans {} fichier(s)", pattern, files_searched)
                } else {
                    format!(
                        "{} correspondance(s) pour '{}' dans {} fichier(s):\n\n{}",
                        total_matches, pattern, files_searched,
                        results.join("\n")
                    )
                }
            }
        };

        Ok(ToolResult::success(output))
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let pattern = args.get_string("pattern").unwrap_or_else(|| "<inconnu>".to_string());
        let path = args.get_string("path").unwrap_or_else(|| ".".to_string());
        format!("Grep '{}' dans {}", pattern, path)
    }
}

// ============================================================================
// AkariWebFetch — Recuperer le contenu d'une URL
// ============================================================================

pub struct AkariWebFetch;

impl AkariWebFetch {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AkariWebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Recuperer le contenu textuel d'une URL. Convertit le HTML en texte lisible. \
         Les URLs HTTP sont mises a niveau en HTTPS automatiquement. \
         Utile pour consulter de la documentation, des API, ou des pages web."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "L'URL complete a recuperer (ex: https://docs.rs/tokio)"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Longueur max du texte retourne en caracteres (defaut: 15000)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let url = args.get_string("url")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'url' manquant"))?;
        let max_length = args.arguments.get("max_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(15000) as usize;

        // Upgrade HTTP to HTTPS
        let url = if url.starts_with("http://") {
            url.replacen("http://", "https://", 1)
        } else {
            url
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Makima/0.2 (local coding assistant)")
            .build()?;

        let response = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!("Echec de la requete: {}", e)));
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!("HTTP {}: {}", status, url)));
        }

        let content_type = response.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = response.text().await?;

        let text = if content_type.contains("text/html") {
            strip_html_tags(&body)
        } else {
            body
        };

        let text = if text.len() > max_length {
            format!("{}...\n\n[Tronque a {} caracteres sur {} total]",
                safe_truncate(&text, max_length), max_length, text.len())
        } else {
            text
        };

        Ok(ToolResult::success(format!("URL: {}\nType: {}\n\n{}", url, content_type, text)))
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let url = args.get_string("url").unwrap_or_else(|| "<inconnu>".to_string());
        format!("Fetch: {}", url)
    }
}

// ============================================================================
// AkariWebSearch — Recherche web via DuckDuckGo
// ============================================================================

pub struct AkariWebSearch;

impl AkariWebSearch {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AkariWebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Rechercher sur le web via DuckDuckGo. Retourne les resultats avec titres, \
         URLs et extraits. Utile pour obtenir des informations actuelles \
         ou de la documentation recente."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "La requete de recherche"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de resultats (defaut: 5, max: 10)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let query = args.get_string("query")
            .ok_or_else(|| anyhow::anyhow!("Parametre 'query' manquant"))?;
        let max_results = args.arguments.get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(10) as usize;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Makima/0.2 (local coding assistant)")
            .build()?;

        // DuckDuckGo Instant Answer API
        let api_url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            urlencoding_simple(&query)
        );

        let mut results = Vec::new();

        // Try Instant Answer API first
        if let Ok(response) = client.get(&api_url).send().await {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                // Abstract (direct answer)
                if let Some(abstract_text) = body["AbstractText"].as_str() {
                    if !abstract_text.is_empty() {
                        let source = body["AbstractSource"].as_str().unwrap_or("");
                        let url = body["AbstractURL"].as_str().unwrap_or("");
                        results.push(format!("## Reponse directe ({})\n{}\nSource: {}", source, abstract_text, url));
                    }
                }

                // Related topics
                if let Some(topics) = body["RelatedTopics"].as_array() {
                    for topic in topics.iter().take(max_results) {
                        if let (Some(text), Some(url)) = (topic["Text"].as_str(), topic["FirstURL"].as_str()) {
                            if !text.is_empty() {
                                results.push(format!("- {}\n  {}", text, url));
                            }
                        }
                    }
                }
            }
        }

        // Fallback: HTML search
        if results.is_empty() {
            let html_url = format!(
                "https://html.duckduckgo.com/html/?q={}",
                urlencoding_simple(&query)
            );

            if let Ok(response) = client.get(&html_url).send().await {
                if let Ok(html) = response.text().await {
                    static RE_LINK: OnceLock<Regex> = OnceLock::new();
                    static RE_SNIPPET: OnceLock<Regex> = OnceLock::new();
                    let re_l = RE_LINK.get_or_init(|| Regex::new(r#"<a rel="nofollow" class="result__a" href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap());
                    let re_s = RE_SNIPPET.get_or_init(|| Regex::new(r#"<a class="result__snippet"[^>]*>(.*?)</a>"#).unwrap());

                    let urls: Vec<_> = re_l.captures_iter(&html).collect();
                    let snippets: Vec<_> = re_s.captures_iter(&html).collect();

                    for (i, cap) in urls.iter().take(max_results).enumerate() {
                        let url = &cap[1];
                        let title = strip_html_tags_simple(&cap[2]);
                        let snippet = snippets.get(i)
                            .map(|s| strip_html_tags_simple(&s[1]))
                            .unwrap_or_default();
                        results.push(format!("{}. {}\n   {}\n   {}", i + 1, title, url, snippet));
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(ToolResult::success(format!("Aucun resultat pour: {}", query)))
        } else {
            Ok(ToolResult::success(format!(
                "Resultats pour '{}':\n\n{}",
                query,
                results.join("\n\n")
            )))
        }
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let query = args.get_string("query").unwrap_or_else(|| "<inconnu>".to_string());
        format!("Recherche: {}", query)
    }
}

// ============================================================================
// Fonctions utilitaires
// ============================================================================

/// Tronquer une chaine de maniere safe pour UTF-8
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Supprime les tags HTML et retourne du texte brut
fn strip_html_tags(html: &str) -> String {
    static RE_SCRIPT: OnceLock<Regex> = OnceLock::new();
    static RE_TAGS: OnceLock<Regex> = OnceLock::new();
    static RE_SPACES: OnceLock<Regex> = OnceLock::new();
    static RE_NEWLINES: OnceLock<Regex> = OnceLock::new();

    let re_script = RE_SCRIPT.get_or_init(|| Regex::new(r"(?si)<(script|style)[^>]*>.*?</\1>").unwrap());
    let text = re_script.replace_all(html, "");

    let re_tags = RE_TAGS.get_or_init(|| Regex::new(r"<[^>]+>").unwrap());
    let text = re_tags.replace_all(&text, " ");

    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    let re_spaces = RE_SPACES.get_or_init(|| Regex::new(r"[ \t]+").unwrap());
    let text = re_spaces.replace_all(&text, " ");
    let re_newlines = RE_NEWLINES.get_or_init(|| Regex::new(r"\n{3,}").unwrap());
    let text = re_newlines.replace_all(&text, "\n\n");

    text.trim().to_string()
}

/// Version simple pour les petits fragments HTML
fn strip_html_tags_simple(html: &str) -> String {
    static RE_TAGS: OnceLock<Regex> = OnceLock::new();
    let re = RE_TAGS.get_or_init(|| Regex::new(r"<[^>]+>").unwrap());
    re.replace_all(html, "").trim().to_string()
}

/// Encodage URL simple
fn urlencoding_simple(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
            String::from(b as char)
        }
        b' ' => "+".to_string(),
        _ => format!("%{:02X}", b),
    }).collect()
}
