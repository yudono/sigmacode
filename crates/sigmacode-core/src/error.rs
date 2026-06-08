use thiserror::Error;

#[derive(Error, Debug)]
pub enum SigmaError {
    #[error("LLM error: {0}")]
    Llm(String),

    #[error("LLM rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("LLM auth failed: {0}")]
    LlmAuth(String),

    #[error("Tool error: {tool} - {message}")]
    Tool { tool: String, message: String },

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),

    #[error("Retrieval error: {0}")]
    Retrieval(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Request error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Task cancelled")]
    Cancelled,

    #[error("Max iterations ({0}) exceeded")]
    MaxIterations(usize),

    #[error("Context window exceeded")]
    ContextWindowExceeded,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SigmaError>;
