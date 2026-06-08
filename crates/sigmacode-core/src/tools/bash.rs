use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
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

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&context.workspace)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                let mut combined = String::new();
                if !stdout.is_empty() {
                    combined.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push_str("\n--- stderr ---\n");
                    }
                    combined.push_str(&stderr);
                }

                if combined.is_empty() {
                    combined = "(no output)".to_string();
                }

                Ok(ToolResult {
                    tool_call_id: String::new(),
                    content: combined,
                    is_error: !output.status.success(),
                })
            }
            Ok(Err(e)) => Err(SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!("failed to execute command: {}", e),
            }),
            Err(_) => Ok(ToolResult {
                tool_call_id: String::new(),
                content: format!("Command timed out after {}ms", timeout_ms),
                is_error: true,
            }),
        }
    }
}
