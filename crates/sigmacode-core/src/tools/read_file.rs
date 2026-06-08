use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path. Returns the full file content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-indexed)",
                    "default": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read",
                    "default": 2000
                }
            },
            "required": ["path"]
        })
    }

    fn permission_required(&self) -> PermissionLevel {
        PermissionLevel::Auto
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'path' argument".into(),
            })?;

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            context.workspace.join(path)
        };

        let content = tokio::fs::read_to_string(&full_path).await.map_err(|e| {
            SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!("failed to read {}: {}", full_path.display(), e),
            }
        })?;

        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let limit = args["limit"].as_u64().unwrap_or(2000) as usize;

        let lines: Vec<&str> = content.lines().collect();
        let start = offset.min(lines.len());
        let end = (start + limit).min(lines.len());

        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {}", start + i + 1, line))
            .collect();

        Ok(ToolResult {
            tool_call_id: String::new(),
            content: selected.join("\n"),
            is_error: false,
        })
    }
}
