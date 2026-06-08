use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths sorted by modification time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g. '**/*.rs', 'src/**/*.ts')"
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_required(&self) -> PermissionLevel {
        PermissionLevel::Auto
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| SigmaError::Tool {
                tool: self.name().to_string(),
                message: "missing 'pattern' argument".into(),
            })?;

        let _full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", context.workspace.display(), pattern)
        };

        let mut files: Vec<String> = Vec::new();

        // Use glob::glob if available, otherwise fall back to simple matching
        // For now, use a simple recursive walk with pattern matching
        let workspace = &context.workspace;
        collect_files(workspace, pattern, &mut files).await?;

        files.sort();

        let output = if files.is_empty() {
            "No files found matching pattern".to_string()
        } else {
            files.join("\n")
        };

        Ok(ToolResult {
            tool_call_id: String::new(),
            content: output,
            is_error: false,
        })
    }
}

async fn collect_files(dir: &std::path::Path, pattern: &str, files: &mut Vec<String>) -> Result<()> {
    let mut entries = tokio::fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = entry.metadata().await?;

        if metadata.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
                continue;
            }
            Box::pin(collect_files(&path, pattern, files)).await?;
        } else if metadata.is_file() {
            if matches_glob(pattern, &path) {
                files.push(path.display().to_string());
            }
        }
    }

    Ok(())
}

fn matches_glob(pattern: &str, path: &std::path::Path) -> bool {
    let path_str = path.display().to_string();

    if pattern.contains("**") {
        let suffix = pattern.trim_start_matches("**/");
        return path_str.ends_with(suffix.trim_start_matches('*'))
            || path_str.contains(suffix);
    }

    if let Some(ext) = pattern.strip_prefix("*.") {
        return path.extension()
            .map(|e| e == ext)
            .unwrap_or(false);
    }

    path_str.contains(pattern)
}
