mod bash;
mod edit_file;
mod glob;
mod grep;
mod read_file;
mod write_file;

pub use bash::BashTool;
pub use edit_file::EditFileTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read_file::ReadFileTool;
pub use write_file::WriteFileTool;

use async_trait::async_trait;

use crate::error::Result;
use crate::types::{AgentState, ToolDefinition, ToolResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionLevel {
    Auto,
    Confirm,
    Approve,
}

pub struct ToolContext {
    pub workspace: std::path::PathBuf,
    pub state: AgentState,
    pub signal: tokio_util::sync::CancellationToken,
    pub output_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;

    async fn execute(
        &self,
        args: serde_json::Value,
        context: &ToolContext,
    ) -> Result<ToolResult>;

    fn permission_required(&self) -> PermissionLevel {
        PermissionLevel::Auto
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

pub struct ToolRouter {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRouter {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn register_defaults(&mut self) {
        self.register(Box::new(ReadFileTool));
        self.register(Box::new(WriteFileTool));
        self.register(Box::new(EditFileTool));
        self.register(Box::new(BashTool));
        self.register(Box::new(GlobTool));
        self.register(Box::new(GrepTool));
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.definition()).collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        context: &ToolContext,
    ) -> Result<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| crate::error::SigmaError::ToolNotFound(name.to_string()))?;

        if let Some(err) = sandbox_check(name, &args, &context.workspace) {
            return Ok(ToolResult {
                tool_call_id: String::new(),
                content: err,
                is_error: true,
            });
        }

        tool.execute(args, context).await
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }
}

impl Default for ToolRouter {
    fn default() -> Self {
        let mut router = Self::new();
        router.register_defaults();
        router
    }
}

fn sandbox_check(tool_name: &str, args: &serde_json::Value, workspace: &std::path::Path) -> Option<String> {
    let path_arg = match tool_name {
        "read_file" | "write_file" | "edit_file" => args["path"].as_str()?,
        "bash" => {
            let cmd = args["command"].as_str()?;
            if cmd.contains("..") || cmd.contains("~") {
                return Some(format!(
                    "Sandbox violation: command '{}' contains path traversal. Workspace: {}",
                    &cmd[..cmd.len().min(80)],
                    workspace.display()
                ));
            }
            return None;
        }
        _ => return None,
    };

    let full_path = if std::path::Path::new(path_arg).is_absolute() {
        std::path::PathBuf::from(path_arg)
    } else {
        workspace.join(path_arg)
    };

    let canonical = full_path.canonicalize().ok();
    let ws_canonical = workspace.canonicalize().ok();

    match (canonical, ws_canonical) {
        (Some(c), Some(ws)) => {
            if !c.starts_with(&ws) {
                return Some(format!(
                    "Sandbox violation: {} is outside workspace {}\nOnly files within the workspace can be accessed.",
                    c.display(),
                    ws.display()
                ));
            }
        }
        (Some(c), None) => {
            // Workspace doesn't exist yet, allow operations (e.g., first file creation)
            let _ = c;
        }
        _ => {}
    }

    None
}
