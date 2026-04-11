//! Bash/shell command execution tool

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct BashTool {
    working_dir: String,
}

impl BashTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Exécuter une commande shell. Pour git, build, etc. S'exécute dans l'espace de travail."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "La commande shell à exécuter"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Délai d'expiration en secondes (défaut: 60)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let command = args
            .get_string("command")
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        let timeout_secs = args
            .arguments
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        // Determine shell based on platform
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
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stderr"))?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut output_lines = Vec::new();
        let mut error_lines = Vec::new();

        // Collect output with timeout
        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let result = tokio::time::timeout(timeout, async {
            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line {
                            Ok(Some(line)) => output_lines.push(line),
                            Ok(None) => break,
                            Err(e) => {
                                error_lines.push(format!("Error reading stdout: {}", e));
                                break;
                            }
                        }
                    }
                    line = stderr_reader.next_line() => {
                        match line {
                            Ok(Some(line)) => error_lines.push(line),
                            Ok(None) => {},
                            Err(e) => {
                                error_lines.push(format!("Error reading stderr: {}", e));
                            }
                        }
                    }
                }
            }
            child.wait().await
        })
        .await;

        match result {
            Ok(Ok(status)) => {
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
                    output = "(no output)".to_string();
                }

                let exit_info = format!("\n\nExit code: {}", status.code().unwrap_or(-1));
                output.push_str(&exit_info);

                if status.success() {
                    Ok(ToolResult::success(output))
                } else {
                    Ok(ToolResult::error(output))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute command: {}", e))),
            Err(_) => {
                // Timeout - try to kill the process
                let _ = child.kill().await;
                Ok(ToolResult::error(format!(
                    "Command timed out after {} seconds",
                    timeout_secs
                )))
            }
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let command = args.get_string("command").unwrap_or_else(|| "<unknown>".to_string());
        let preview = if command.len() > 80 {
            format!("{}...", &command[..80])
        } else {
            command
        };
        format!("Execute command: {}", preview)
    }
}
