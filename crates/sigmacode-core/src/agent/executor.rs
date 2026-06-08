use crate::error::Result;
use crate::tools::{ToolContext, ToolRouter};
use crate::types::*;

pub async fn execute_task(
    task: &Task,
    tools: &ToolRouter,
    workspace: &std::path::Path,
    state: &AgentState,
    signal: tokio_util::sync::CancellationToken,
) -> Result<TaskResult> {
    let context = ToolContext {
        workspace: workspace.to_path_buf(),
        state: state.clone(),
        signal,
        output_tx: None,
    };

    // Parse instruction as tool call arguments
    let args = parse_task_args(task);

    let result = tools.execute(&task.task_type, args, &context).await;

    match result {
        Ok(tool_result) => Ok(TaskResult {
            task_id: task.id.clone(),
            task_type: task.task_type.clone(),
            output: tool_result.content,
            success: !tool_result.is_error,
        }),
        Err(e) => Ok(TaskResult {
            task_id: task.id.clone(),
            task_type: task.task_type.clone(),
            output: format!("Error: {}", e),
            success: false,
        }),
    }
}

fn parse_task_args(task: &Task) -> serde_json::Value {
    // Try to extract arguments from instruction based on tool type
    match task.task_type.as_str() {
        "read_file" => {
            let path = extract_path(&task.instruction);
            serde_json::json!({ "path": path })
        }
        "write_file" => {
            let path = extract_path(&task.instruction);
            serde_json::json!({ "path": path, "content": task.instruction })
        }
        "edit_file" => {
            let path = extract_path(&task.instruction);
            serde_json::json!({
                "path": path,
                "old_string": "",
                "new_string": task.instruction
            })
        }
        "bash" => {
            serde_json::json!({ "command": task.instruction })
        }
        "glob" => {
            serde_json::json!({ "pattern": &task.instruction })
        }
        "grep" => {
            serde_json::json!({ "pattern": &task.instruction })
        }
        _ => serde_json::json!({ "instruction": &task.instruction }),
    }
}

fn extract_path(instruction: &str) -> String {
    // Simple heuristic to extract file path from instruction
    let words: Vec<&str> = instruction.split_whitespace().collect();
    for word in &words {
        if word.contains('/') || word.contains('.') {
            return word.trim_matches(|c| c == '`' || c == '"').to_string();
        }
    }
    "unknown".to_string()
}
