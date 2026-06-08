use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match with new content. The old_string must match exactly (including whitespace/indentation)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace (must match exactly)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with"
                },
                "replaceAll": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)",
                    "default": false
                }
            },
            "required": ["path", "old_string", "new_string"]
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

        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'old_string' argument".into(),
            })?;

        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'new_string' argument".into(),
            })?;

        let replace_all = args["replaceAll"].as_bool().unwrap_or(false);

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

        let count = content.matches(old_string).count();
        if count == 0 {
            return Err(SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!(
                    "old_string not found in {}. Make sure the string matches exactly.",
                    full_path.display()
                ),
            });
        }

        if count > 1 && !replace_all {
            return Err(SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!(
                    "Found {} matches for old_string in {}. Provide more surrounding lines or use replaceAll.",
                    count,
                    full_path.display()
                ),
            });
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        tokio::fs::write(&full_path, &new_content).await.map_err(|e| {
            SigmaError::Tool {
                tool: self.name().to_string(),
                message: format!("failed to write {}: {}", full_path.display(), e),
            }
        })?;

        let msg = if replace_all {
            format!("Replaced {} occurrences in {}", count, full_path.display())
        } else {
            format!("Successfully edited {}", full_path.display())
        };

        Ok(ToolResult {
            tool_call_id: String::new(),
            content: msg,
            is_error: false,
        })
    }
}
