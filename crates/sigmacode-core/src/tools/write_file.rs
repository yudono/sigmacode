use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn permission_required(&self) -> PermissionLevel {
        PermissionLevel::Confirm
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'path' argument".into(),
            })?;

        let content = args["content"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'content' argument".into(),
            })?;

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            context.workspace.join(path)
        };

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                SigmaError::Tool {
                    tool: self.name().to_string(),
                    message: format!("failed to create directories: {}", e),
                }
            })?;
        }

        tokio::fs::write(&full_path, content).await.map_err(|e| {
            SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!("failed to write {}: {}", full_path.display(), e),
            }
        })?;

        Ok(ToolResult {
            tool_call_id: String::new(),
            content: format!("Successfully wrote to {}", full_path.display()),
            is_error: false,
        })
    }
}
