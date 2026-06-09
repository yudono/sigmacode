use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ── Agent Mode ──

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Chat,
    Planner,
    Builder,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Chat
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Planner => write!(f, "planner"),
            Self::Builder => write!(f, "builder"),
        }
    }
}

impl std::str::FromStr for AgentMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "chat" => Ok(Self::Chat),
            "planner" => Ok(Self::Planner),
            "builder" => Ok(Self::Builder),
            _ => Err(format!("Unknown mode: {}", s)),
        }
    }
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
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
    // Architecture events
    AnalysisComplete { goals: Vec<String>, constraints: Vec<String>, success_criteria: Vec<String> },
    PlanValidated { issues: Vec<String> },
    VerificationStarted { step: String },
    VerificationPassed { step: String },
    VerificationFailed { step: String, errors: Vec<String> },
    Criticking { errors: Vec<String> },
    CriticResult { root_cause: String, fix: String },
    Replanning { reason: String, attempt: u32 },
    Reviewing,
    ReviewComplete { score: u32, issues_count: usize },
    Finalizing,
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

// ── Architecture Types ──

#[derive(Debug, Clone)]
pub struct TaskAnalysis {
    pub intent: String,
    pub goals: Vec<String>,
    pub constraints: Vec<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub id: usize,
    pub description: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub depends_on: Vec<usize>,
    pub risk_level: RiskLevel,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
    pub total_risk: RiskLevel,
    pub estimated_complexity: u32,
}

#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub passed: bool,
    pub step: String,
    pub errors: Vec<String>,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct CriticResult {
    pub root_cause: String,
    pub error_class: String,
    pub fix_recommendation: String,
    pub affected_steps: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub issues: Vec<ReviewIssue>,
    pub score: u32,
}

#[derive(Debug, Clone)]
pub struct ReviewIssue {
    pub severity: ReviewSeverity,
    pub category: String,
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct SessionMemory {
    pub actions: Vec<String>,
    pub files_modified: Vec<String>,
    pub errors_encountered: Vec<String>,
    pub patterns_learned: Vec<String>,
}

// ── Session Persistence ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub mode: AgentMode,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}
