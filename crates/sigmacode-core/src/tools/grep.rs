use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::process::Command;

use crate::error::{Result, SigmaError};
use crate::tools::{PermissionLevel, Tool, ToolContext};
use crate::types::ToolResult;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Returns matching lines with file paths and line numbers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "include": {
                    "type": "string",
                    "description": "File pattern to include (e.g. '*.rs', '*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: workspace root)"
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

        let include = args["include"].as_str().unwrap_or("*");
        let search_path = args["path"]
            .as_str()
            .map(|p| {
                if std::path::Path::new(p).is_absolute() {
                    std::path::PathBuf::from(p)
                } else {
                    context.workspace.join(p)
                }
            })
            .unwrap_or_else(|| context.workspace.clone());

        // Try using ripgrep if available
        let result = Command::new("rg")
            .arg("--line-number")
            .arg("--color=never")
            .arg("--include")
            .arg(include)
            .arg(pattern)
            .arg(&search_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() || !output.stdout.is_empty() => {
                let content = String::from_utf8_lossy(&output.stdout).to_string();
                let truncated = if content.len() > 50_000 {
                    format!("{}\n\n--- Output truncated ({} bytes) ---", &content[..50_000], content.len())
                } else {
                    content
                };

                Ok(ToolResult {
                    tool_call_id: String::new(),
                    content: if truncated.is_empty() { "No matches found".into() } else { truncated },
                    is_error: false,
                })
            }
            _ => {
                // Fallback: simple grep-like search
                simple_grep(pattern, &search_path, &context).await
            }
        }
    }
}

async fn simple_grep(
    pattern: &str,
    dir: &std::path::Path,
    _context: &ToolContext,
) -> Result<ToolResult> {
    let regex = regex::Regex::new(pattern).map_err(|e| SigmaError::Tool {
        tool: "grep".into(),
        message: format!("invalid regex: {}", e),
    })?;

    let mut results = Vec::new();
    collect_grep_matches(dir, &regex, &mut results, 1000).await?;

    let output = if results.is_empty() {
        "No matches found".to_string()
    } else {
        results.join("\n")
    };

    Ok(ToolResult {
        tool_call_id: String::new(),
        content: output,
        is_error: false,
    })
}

async fn collect_grep_matches(
    dir: &std::path::Path,
    regex: &regex::Regex,
    results: &mut Vec<String>,
    max_results: usize,
) -> Result<()> {
    if results.len() >= max_results {
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        if results.len() >= max_results {
            break;
        }

        let path = entry.path();
        let metadata = entry.metadata().await?;

        if metadata.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
                continue;
            }
            Box::pin(collect_grep_matches(&path, regex, results, max_results)).await?;
        } else if metadata.is_file() {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    if results.len() >= max_results {
                        break;
                    }
                    if regex.is_match(line) {
                        results.push(format!("{}:{}: {}", path.display(), i + 1, line));
                    }
                }
            }
        }
    }

    Ok(())
}
