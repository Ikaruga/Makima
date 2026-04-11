//! CSV to DOCX conversion tool

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use docx_rs::*;
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};

/// Resolve a path relative to the working directory
fn resolve_path(working_dir: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}

/// Tool for converting CSV files to Word documents with formatted tables
pub struct CsvToDocxTool {
    working_dir: String,
}

impl CsvToDocxTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }

    /// Get array of strings from arguments
    fn get_string_array(args: &ParsedToolCall, key: &str) -> Option<Vec<String>> {
        args.arguments.get(key).and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
    }
}

#[async_trait]
impl Tool for CsvToDocxTool {
    fn name(&self) -> &str {
        "csv_to_docx"
    }

    fn description(&self) -> &str {
        "Convertir un CSV en document Word (.docx) avec tableau formaté."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Chemin du fichier CSV à convertir"
                },
                "output": {
                    "type": "string",
                    "description": "Chemin du document Word de sortie (optionnel)"
                },
                "columns": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Liste des colonnes à inclure (optionnel, toutes par défaut)"
                },
                "title": {
                    "type": "string",
                    "description": "Titre en haut du document (optionnel)"
                }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        // Get parameters
        let input = args
            .get_string("input")
            .ok_or_else(|| anyhow::anyhow!("Missing 'input' parameter"))?;

        let input_path = resolve_path(&self.working_dir, &input);

        // Default output path: same as input with .docx extension
        let output = args.get_string("output").unwrap_or_else(|| {
            let stem = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output");
            format!("{}.docx", stem)
        });
        let output_path = resolve_path(&self.working_dir, &output);

        let columns_filter = Self::get_string_array(args, "columns");
        let title = args.get_string("title");

        // Check if input file exists
        if !input_path.exists() {
            return Ok(ToolResult::error(format!(
                "CSV file not found: {}",
                input_path.display()
            )));
        }

        // Read CSV file
        let file = File::open(&input_path)
            .map_err(|e| anyhow::anyhow!("Failed to open CSV file: {}", e))?;

        let mut reader = csv::Reader::from_reader(file);

        // Get headers
        let headers: Vec<String> = reader
            .headers()
            .map_err(|e| anyhow::anyhow!("Failed to read CSV headers: {}", e))?
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Determine which columns to include
        let (selected_indices, selected_headers): (Vec<usize>, Vec<String>) =
            if let Some(ref cols) = columns_filter {
                let mut indices = Vec::new();
                let mut hdrs = Vec::new();
                for col in cols {
                    if let Some(idx) = headers.iter().position(|h| h == col) {
                        indices.push(idx);
                        hdrs.push(col.clone());
                    } else {
                        return Ok(ToolResult::error(format!(
                            "Column '{}' not found in CSV. Available columns: {}",
                            col,
                            headers.join(", ")
                        )));
                    }
                }
                (indices, hdrs)
            } else {
                (
                    (0..headers.len()).collect(),
                    headers.clone(),
                )
            };

        // Read all records
        let mut records: Vec<Vec<String>> = Vec::new();
        for result in reader.records() {
            let record = result.map_err(|e| anyhow::anyhow!("Failed to read CSV record: {}", e))?;
            let row: Vec<String> = selected_indices
                .iter()
                .map(|&idx| record.get(idx).unwrap_or("").to_string())
                .collect();
            records.push(row);
        }

        let row_count = records.len();
        let col_count = selected_headers.len();

        // Create Word document
        let mut docx = Docx::new();

        // Add title if provided
        if let Some(ref title_text) = title {
            let title_para = Paragraph::new()
                .add_run(Run::new().add_text(title_text).bold().size(32));
            docx = docx.add_paragraph(title_para);

            // Add empty paragraph for spacing
            docx = docx.add_paragraph(Paragraph::new());
        }

        // Create table
        let mut table = Table::new(vec![]);

        // Create header row with bold text
        let header_cells: Vec<TableCell> = selected_headers
            .iter()
            .map(|h| {
                TableCell::new().add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text(h).bold()),
                )
            })
            .collect();
        let header_row = TableRow::new(header_cells);
        table = table.add_row(header_row);

        // Add data rows
        for record in &records {
            let cells: Vec<TableCell> = record
                .iter()
                .map(|cell| {
                    TableCell::new().add_paragraph(
                        Paragraph::new().add_run(Run::new().add_text(cell)),
                    )
                })
                .collect();
            table = table.add_row(TableRow::new(cells));
        }

        // Add table to document
        docx = docx.add_table(table);

        // Create parent directories if needed
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write document
        let file = File::create(&output_path)
            .map_err(|e| anyhow::anyhow!("Failed to create output file: {}", e))?;

        docx.build()
            .pack(file)
            .map_err(|e| anyhow::anyhow!("Failed to write DOCX file: {}", e))?;

        Ok(ToolResult::success(format!(
            "Created {} with {} rows and {} columns from {}",
            output_path.display(),
            row_count,
            col_count,
            input_path.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let input = args.get_string("input").unwrap_or_else(|| "<unknown>".to_string());
        let output = args.get_string("output");

        if let Some(out) = output {
            format!("Convert CSV {} to Word document {}", input, out)
        } else {
            format!("Convert CSV {} to Word document", input)
        }
    }
}
