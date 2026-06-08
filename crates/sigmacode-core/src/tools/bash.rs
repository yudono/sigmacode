use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return its output. Use for running builds, tests, git commands, etc."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 120000)",
                    "default": 120000
                }
            },
            "required": ["command"]
        })
    }

    fn permission_required(&self) -> PermissionLevel {
        PermissionLevel::Confirm
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'command' argument".into(),
            })?;

        let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(120_000);

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&context.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!("failed to spawn command: {}", e),
            })?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();

        let mut combined = String::new();
        let mut got_output = false;

        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref tx) = context.output_tx {
                                let _ = tx.send(line.clone()).await;
                            }
                            if got_output {
                                combined.push('\n');
                            }
                            combined.push_str(&line);
                            got_output = true;
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                line = stderr_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref tx) = context.output_tx {
                                let _ = tx.send(line.clone()).await;
                            }
                            if got_output {
                                combined.push('\n');
                            }
                            combined.push_str(&line);
                            got_output = true;
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)) => {
                    let _ = child.kill().await;
                    let msg = format!("Command timed out after {}ms", timeout_ms);
                    if let Some(ref tx) = context.output_tx {
                        let _ = tx.send(msg.clone()).await;
                    }
                    return Ok(ToolResult {
                        tool_call_id: String::new(),
                        content: msg,
                        is_error: true,
                    });
                }
                _ = context.signal.cancelled() => {
                    let _ = child.kill().await;
                    return Ok(ToolResult {
                        tool_call_id: String::new(),
                        content: "Command cancelled".into(),
                        is_error: true,
                    });
                }
            }
        }

        let status = child.wait().await.map_err(|e| SigmaError::Tool {
            tool: self.name().to_string(),
            message: format!("failed to wait for command: {}", e),
        })?;

        if combined.is_empty() {
            combined = "(no output)".to_string();
        }

        Ok(ToolResult {
            tool_call_id: String::new(),
            content: combined,
            is_error: !status.success(),
        })
    }
}
