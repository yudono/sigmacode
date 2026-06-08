use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ── Agent State ──

#[derive(Debug, Clone)]
pub struct AgentState {
    pub session_id: Uuid,
    pub task: String,
    pub messages: Vec<Message>,
    pub plan: Option<Plan>,
    pub results: Vec<TaskResult>,
    pub working_memory: WorkingMemory,
    pub workspace: PathBuf,
    pub config: AgentConfig,
    pub iteration: usize,
    pub event_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub max_iterations: usize,
    pub context_window: usize,
    pub temperature: f32,
    pub auto_compact: bool,
    pub sandbox_policy: SandboxPolicy,
    pub mcp_servers: Vec<McpServerConfig>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o".into(),
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            max_tokens: 4096,
            max_iterations: 50,
            context_window: 128_000,
            temperature: 0.0,
            auto_compact: true,
            sandbox_policy: SandboxPolicy::DiskRead,
            mcp_servers: Vec::new(),
        }
    }
}

// ── Messages ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant {
        content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl Message {
    pub fn token_estimate(&self) -> usize {
        let text = match self {
            Message::System { content } => content,
            Message::User { content } => content,
            Message::Assistant { content, .. } => content.as_deref().unwrap_or(""),
            Message::Tool { content, .. } => content,
        };
        text.len() / 4
    }
}

// ── Tool Calls ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

// ── Plan ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub task_type: String,
    pub instruction: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// ── Working Memory ──

#[derive(Debug, Clone)]
pub struct WorkingMemory {
    pub entries: Vec<MemoryEntry>,
    pub token_budget: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub label: String,
    pub content: String,
    pub token_estimate: usize,
}

impl WorkingMemory {
    pub fn new(token_budget: usize) -> Self {
        Self {
            entries: Vec::new(),
            token_budget,
        }
    }

    pub fn append(&mut self, label: impl Into<String>, content: impl Into<String>) {
        let label = label.into();
        let content = content.into();
        let token_estimate = content.len() / 4;

        self.entries.push(MemoryEntry {
            label,
            content,
            token_estimate,
        });

        self.trim_to_budget();
    }

    fn trim_to_budget(&mut self) {
        let mut total: usize = self.entries.iter().map(|e| e.token_estimate).sum();
        while total > self.token_budget && !self.entries.is_empty() {
            let removed = self.entries.remove(0);
            total -= removed.token_estimate;
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("[{}]\n{}", e.label, e.content))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// ── Task Result ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub task_type: String,
    pub output: String,
    pub success: bool,
}

// ── Config Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxPolicy {
    None,
    DiskRead,
    DiskWriteTemp,
    NetworkRestricted,
    FullIsolation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

// ── LLM Types ──

#[derive(Debug, Clone)]
pub struct CompletionOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tool_choice: Option<String>,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            max_tokens: None,
            tool_choice: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ── Streaming Events ──

#[derive(Debug, Clone)]
pub enum LlmEvent {
    ContentDelta(String),
    ToolUseStart { id: String, name: String },
    ToolUseDelta { id: String, arguments_delta: String },
    ToolUseEnd { id: String, name: String, arguments: serde_json::Value },
    Done { usage: TokenUsage },
    Error(String),
}

// ── Agent Events (for TUI) ──

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Planning { goal: String },
    PlanCreated { tasks: Vec<Task> },
    TaskStarted { task_id: String, task_type: String, instruction: String },
    TaskCompleted { task_id: String, success: bool, output: String },
    ToolCallStarted { tool_name: String, args_summary: String },
    ToolCallCompleted { tool_name: String, success: bool },
    ToolOutput { tool_call_id: String, line: String },
    Streaming { token: String },
    Verifying { command: String },
    Verified { success: bool },
    Error { message: String },
    Done { summary: String },
    PermissionRequest { tool_name: String, description: String, args_summary: String },
    PermissionResponse { allowed: bool, always: bool },
    DiffGenerated { file_path: String, old_content: String, new_content: String },
    Thinking { content: String },
    Compacting { message: String },
}

// ── Provider Config ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "openai")]
    OpenAi {
        api_key: String,
        base_url: Option<String>,
        model: String,
    },
    #[serde(rename = "anthropic")]
    Anthropic {
        api_key: String,
        model: String,
    },
    #[serde(rename = "gemini")]
    Gemini {
        api_key: String,
        model: String,
    },
    #[serde(rename = "ollama")]
    Ollama {
        base_url: Option<String>,
        model: String,
    },
}
