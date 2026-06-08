use sigmacode_core::llm::create_provider;
use sigmacode_core::tools::ToolRouter;
use sigmacode_core::Agent;
use sigmacode_core::AgentConfig;
use sigmacode_core::AgentEvent;
use sigmacode_core::AgentState;
use sigmacode_core::ContextBuilder;
use sigmacode_core::ProviderConfig;
use sigmacode_core::WorkingMemory;
use tokio::sync::mpsc;

pub struct App {
    pub state: AppState,
    pub input: String,
    pub messages: Vec<ChatMessage>,
    pub agent_handle: Option<tokio::task::JoinHandle<()>>,
    pub event_rx: Option<mpsc::UnboundedReceiver<AgentEvent>>,
    pub event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    pub should_quit: bool,
    pub config: AppConfig,
    pub current_tab: Tab,
    pub logs: Vec<String>,
    pub scroll_offset: usize,
    pub tick_count: usize,
    pub token_display: String,
    pub context_usage_pct: u32,
    pub cost: f64,
    pub total_tokens: usize,
    pub permission_pending: Option<PermissionRequest>,
    pub setup: SetupState,
    pub last_task: Option<String>,
}

#[derive(PartialEq)]
pub enum AppState {
    Idle,
    Input,
    Running,
    Done,
    Permission,
    Setup,
}

#[derive(PartialEq)]
pub enum Tab {
    Chat,
    Logs,
}

pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub diff: Option<DiffView>,
}

#[derive(PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
    Thought,
    Diff,
}

pub struct DiffView {
    pub file_path: String,
    pub old_lines: Vec<DiffLine>,
    pub new_lines: Vec<DiffLine>,
}

#[allow(dead_code)]
pub struct DiffLine {
    pub line_num: usize,
    pub content: String,
    pub is_added: bool,
    pub is_removed: bool,
}

#[allow(dead_code)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub description: String,
    pub args_summary: String,
    pub allow_always: bool,
}

pub struct AppConfig {
    pub provider: ProviderConfig,
    pub model: String,
}

// ── Setup Wizard ──

pub struct SetupState {
    pub step: SetupStep,
    pub provider_choice: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[derive(PartialEq)]
pub enum SetupStep {
    Welcome,
    Provider,
    ApiKey,
    BaseUrl,
    #[allow(dead_code)]
    Model,
    Done,
}

impl Default for SetupState {
    fn default() -> Self {
        Self {
            step: SetupStep::Welcome,
            provider_choice: String::new(),
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
        }
    }
}

impl App {
    pub async fn new() -> anyhow::Result<Self> {
        let needs_setup = !std::path::Path::new(".env").exists()
            && std::env::var("SIGMACODE_API_KEY").unwrap_or_default().is_empty()
            && std::env::var("OPENAI_API_KEY").unwrap_or_default().is_empty();

        let config = load_config()?;

        Ok(Self {
            state: if needs_setup {
                AppState::Setup
            } else {
                AppState::Idle
            },
            input: String::new(),
            messages: Vec::new(),
            agent_handle: None,
            event_rx: None,
            event_tx: None,
            should_quit: false,
            config,
            current_tab: Tab::Chat,
            logs: Vec::new(),
            scroll_offset: 0,
            tick_count: 0,
            token_display: "0 tokens".into(),
            context_usage_pct: 0,
            cost: 0.0,
            total_tokens: 0,
            permission_pending: None,
            setup: SetupState::default(),
            last_task: None,
        })
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.state {
            AppState::Setup => self.handle_setup_key(key),
            AppState::Idle | AppState::Done => match key.code {
                crossterm::event::KeyCode::Char('i') | crossterm::event::KeyCode::Char('I') => {
                    self.state = AppState::Input;
                }
                crossterm::event::KeyCode::Char('l') | crossterm::event::KeyCode::Char('L') => {
                    self.current_tab = Tab::Logs;
                }
                crossterm::event::KeyCode::Char('c') | crossterm::event::KeyCode::Esc => {
                    self.current_tab = Tab::Chat;
                }
                crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                }
                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
                _ => {}
            },
            AppState::Input => match key.code {
                crossterm::event::KeyCode::Enter => {
                    if !self.input.trim().is_empty() {
                        let task = self.input.clone();
                        self.input.clear();
                        if task.starts_with('/') {
                            self.handle_slash_command(&task);
                        } else {
                            self.last_task = Some(task.clone());
                            self.spawn_agent(task);
                        }
                    }
                }
                crossterm::event::KeyCode::Esc => {
                    self.state = if self.messages.is_empty() {
                        AppState::Idle
                    } else {
                        AppState::Done
                    };
                    self.input.clear();
                }
                crossterm::event::KeyCode::Backspace => {
                    self.input.pop();
                }
                crossterm::event::KeyCode::Char(c) => {
                    self.input.push(c);
                }
                _ => {}
            },
            AppState::Permission => match key.code {
                crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                    self.respond_permission(true, false);
                }
                crossterm::event::KeyCode::Char('a') | crossterm::event::KeyCode::Char('A') => {
                    self.respond_permission(true, true);
                }
                crossterm::event::KeyCode::Char('n')
                | crossterm::event::KeyCode::Char('N')
                | crossterm::event::KeyCode::Esc => {
                    self.respond_permission(false, false);
                }
                _ => {}
            },
            AppState::Running => {}
        }
    }

    // ── Setup Wizard ──

    fn handle_setup_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            crossterm::event::KeyCode::Enter => match self.setup.step {
                SetupStep::Welcome => {
                    self.setup.step = SetupStep::Provider;
                }
                SetupStep::Provider => {
                    let choice = self.input.trim().to_string();
                    self.input.clear();
                    match choice.as_str() {
                        "1" | "openai" => {
                            self.setup.provider_choice = "openai".into();
                            self.setup.model = "gpt-4o".into();
                            self.setup.step = SetupStep::ApiKey;
                        }
                        "2" | "anthropic" => {
                            self.setup.provider_choice = "anthropic".into();
                            self.setup.model = "claude-sonnet-4-20250514".into();
                            self.setup.step = SetupStep::ApiKey;
                        }
                        "3" | "ollama" => {
                            self.setup.provider_choice = "ollama".into();
                            self.setup.model = "llama3".into();
                            self.setup.step = SetupStep::BaseUrl;
                        }
                        "4" | "gemini" => {
                            self.setup.provider_choice = "gemini".into();
                            self.setup.model = "gemini-2.0-flash".into();
                            self.setup.step = SetupStep::ApiKey;
                        }
                        "5" | "mimo" => {
                            self.setup.provider_choice = "openai".into();
                            self.setup.base_url = "https://api.xiaomimimo.com/v1".into();
                            self.setup.model = "mimo-v2.5".into();
                            self.setup.step = SetupStep::ApiKey;
                        }
                        _ => {}
                    }
                }
                SetupStep::ApiKey => {
                    self.setup.api_key = self.input.trim().to_string();
                    self.input.clear();
                    self.setup.step = SetupStep::Done;
                    self.finish_setup();
                }
                SetupStep::BaseUrl => {
                    self.setup.base_url = self.input.trim().to_string();
                    self.input.clear();
                    self.setup.step = SetupStep::Done;
                    self.finish_setup();
                }
                SetupStep::Model => {
                    let m = self.input.trim().to_string();
                    if !m.is_empty() {
                        self.setup.model = m;
                    }
                    self.input.clear();
                    self.setup.step = SetupStep::Done;
                    self.finish_setup();
                }
                SetupStep::Done => {}
            },
            crossterm::event::KeyCode::Esc => {
                if self.setup.step == SetupStep::Provider {
                    self.setup.step = SetupStep::Welcome;
                } else if self.setup.step == SetupStep::ApiKey || self.setup.step == SetupStep::BaseUrl {
                    self.setup.step = SetupStep::Provider;
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.input.pop();
            }
            crossterm::event::KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn finish_setup(&mut self) {
        let provider = &self.setup.provider_choice;
        let model = &self.setup.model;
        let api_key = &self.setup.api_key;
        let base_url = &self.setup.base_url;

        let mut env_content = format!(
            "SIGMACODE_PROVIDER={}\nSIGMACODE_MODEL={}\n",
            provider, model
        );

        match provider.as_str() {
            "openai" => {
                env_content.push_str(&format!("SIGMACODE_API_KEY={}\n", api_key));
                if !base_url.is_empty() {
                    env_content.push_str(&format!("SIGMACODE_BASE_URL={}\n", base_url));
                }
            }
            "anthropic" => {
                env_content.push_str(&format!("ANTHROPIC_API_KEY={}\n", api_key));
            }
            "ollama" => {
                env_content.push_str(&format!(
                    "SIGMACODE_BASE_URL={}\n",
                    if base_url.is_empty() {
                        "http://localhost:11434"
                    } else {
                        base_url
                    }
                ));
            }
            "gemini" => {
                env_content.push_str(&format!("SIGMACODE_API_KEY={}\n", api_key));
                env_content.push_str("SIGMACODE_BASE_URL=https://generativelanguage.googleapis.com/v1beta/openai\n");
            }
            _ => {}
        }

        // Save to ~/.config/sigmacode/.env (global)
        if let Some(home) = dirs::home_dir() {
            let config_dir = home.join(".config").join("sigmacode");
            let _ = std::fs::create_dir_all(&config_dir);
            let _ = std::fs::write(config_dir.join(".env"), &env_content);
        }
        // Also save to current directory
        let _ = std::fs::write(".env", &env_content);

        // Reload config
        if let Some(home) = dirs::home_dir() {
            let config_path = home.join(".config").join("sigmacode").join(".env");
            if config_path.exists() {
                let _ = dotenvy::from_path(&config_path);
            }
        }
        let _ = dotenvy::dotenv();
        if let Ok(new_config) = load_config() {
            self.config = new_config;
        }

        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!(
                "Setup complete! Provider: {}, Model: {}\nConfig saved to .env",
                self.setup.provider_choice, self.setup.model
            ),
            diff: None,
        });

        self.state = AppState::Idle;
    }

    // ── Slash Commands ──

    fn handle_slash_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let args = parts.get(1).unwrap_or(&"");

        let response = match cmd.as_str() {
            "/help" => self.cmd_help(),
            "/clear" => {
                self.messages.clear();
                self.logs.clear();
                self.total_tokens = 0;
                self.token_display = "0 tokens".into();
                self.context_usage_pct = 0;
                self.cost = 0.0;
                "Chat cleared.".to_string()
            }
            "/memory" => self.cmd_memory(),
            "/resume" => self.cmd_resume(),
            "/models" => self.cmd_models(args),
            "/agents" => self.cmd_agents(),
            "/skills" => self.cmd_skills(),
            "/config" => self.cmd_config(),
            "/compact" => "Context will be compacted on next agent run.".to_string(),
            "/version" => {
                format!("sigmaCode v{}", env!("CARGO_PKG_VERSION"))
            }
            "/quit" | "/exit" => {
                self.should_quit = true;
                return;
            }
            _ => {
                format!("Unknown command: {}. Type /help for available commands.", cmd)
            }
        };

        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: input.to_string(),
            diff: None,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: response,
            diff: None,
        });
    }

    fn cmd_help(&self) -> String {
        r#"Available commands:

  /help       Show this help message
  /clear      Clear chat history and reset tokens
  /memory     Show working memory status
  /resume     Re-run the last task
  /models     Switch model (e.g., /models gpt-4o)
  /agents     Show agent information
  /skills     List available tools/skills
  /config     Show current configuration
  /compact    Compact context on next run
  /version    Show version info
  /quit       Exit sigmaCode

Keyboard shortcuts:
  i           Enter input mode
  Esc         Exit input mode / cancel
  j/k         Scroll down/up
  l           Switch to logs tab
  c           Switch to chat tab
  Ctrl+C      Quit"#
            .to_string()
    }

    fn cmd_memory(&self) -> String {
        format!(
            "Working memory: {} tokens used\nContext usage: {}%\nTotal tokens: {}\nCost: ${:.4}",
            self.token_display, self.context_usage_pct, self.total_tokens, self.cost
        )
    }

    fn cmd_resume(&mut self) -> String {
        if let Some(ref task) = self.last_task {
            let task = task.clone();
            self.spawn_agent(task);
            return "Resuming last task...".to_string();
        }
        "No previous task to resume.".to_string()
    }

    fn cmd_models(&mut self, args: &str) -> String {
        let model = args.trim();
        if model.is_empty() {
            return format!(
                "Current model: {}\n\nUsage: /models <model_name>\n\nExamples:\n  /models gpt-4o\n  /models claude-sonnet-4-20250514\n  /models mimo-v2.5",
                self.config.model
            );
        }
        self.config.model = model.to_string();
        let env_content = format!(
            "SIGMACODE_PROVIDER={}\nSIGMACODE_MODEL={}\nSIGMACODE_API_KEY={}\n",
            self.provider_name(),
            model,
            self.api_key_masked()
        );
        // Save to ~/.config/sigmacode/.env
        if let Some(home) = dirs::home_dir() {
            let config_dir = home.join(".config").join("sigmacode");
            let _ = std::fs::create_dir_all(&config_dir);
            let _ = std::fs::write(config_dir.join(".env"), &env_content);
        }
        let _ = std::fs::write(".env", &env_content);
        format!("Model switched to: {}", model)
    }

    fn cmd_agents(&self) -> String {
        format!(
            "Agent: sigmaCode v{}\nProvider: {}\nModel: {}\nWorkspace: {}",
            env!("CARGO_PKG_VERSION"),
            self.provider_name(),
            self.config.model,
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "?".into())
        )
    }

    fn cmd_skills(&self) -> String {
        r#"Available tools:

  bash        Execute shell commands
  read_file   Read file contents
  write_file  Create/overwrite files
  edit_file   Edit files with string replacement
  glob        Find files by pattern
  grep        Search file contents

Use these naturally - the agent decides which tool to use based on your task."#
            .to_string()
    }

    fn cmd_config(&self) -> String {
        format!(
            "Provider: {}\nModel: {}\nBase URL: {}",
            self.provider_name(),
            self.config.model,
            self.base_url_display()
        )
    }

    fn provider_name(&self) -> &str {
        match &self.config.provider {
            ProviderConfig::OpenAi { .. } => "openai",
            ProviderConfig::Anthropic { .. } => "anthropic",
            ProviderConfig::Ollama { .. } => "ollama",
            ProviderConfig::Gemini { .. } => "gemini",
        }
    }

    fn api_key_masked(&self) -> String {
        let key = match &self.config.provider {
            ProviderConfig::OpenAi { api_key, .. } => api_key,
            ProviderConfig::Anthropic { api_key, .. } => api_key,
            ProviderConfig::Gemini { api_key, .. } => api_key,
            _ => return String::new(),
        };
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            "****".into()
        }
    }

    fn base_url_display(&self) -> &str {
        match &self.config.provider {
            ProviderConfig::OpenAi { base_url, .. } => {
                base_url.as_deref().unwrap_or("https://api.openai.com/v1")
            }
            ProviderConfig::Ollama { base_url, .. } => {
                base_url.as_deref().unwrap_or("http://localhost:11434")
            }
            ProviderConfig::Anthropic { .. } => "https://api.anthropic.com",
            ProviderConfig::Gemini { .. } => "https://generativelanguage.googleapis.com",
        }
    }

    // ── Agent ──

    fn respond_permission(&mut self, allow: bool, always: bool) {
        if let Some(_req) = self.permission_pending.take() {
            self.state = AppState::Running;
            if let Some(ref tx) = self.event_tx {
                let _ = tx.send(AgentEvent::PermissionResponse {
                    allowed: allow,
                    always,
                });
            }
        }
    }

    pub async fn tick(&mut self) {
        self.tick_count += 1;

        let events: Vec<AgentEvent> = if let Some(rx) = &mut self.event_rx {
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        } else {
            Vec::new()
        };

        for event in events {
            self.handle_agent_event(event);
        }
    }

    fn spawn_agent(&mut self, task: String) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: task.clone(),
            diff: None,
        });

        self.state = AppState::Running;
        self.event_rx = Some(event_rx);
        self.event_tx = Some(event_tx.clone());
        self.scroll_offset = 0;

        let provider = create_provider(&self.config.provider);
        let tools = ToolRouter::default();
        let workspace = std::env::current_dir().unwrap_or_default();
        let project_name = workspace
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());

        let context_builder = ContextBuilder::new(&project_name);
        let agent = Agent::new(Box::from(provider), tools, context_builder);

        let mut state = AgentState {
            session_id: uuid::Uuid::new_v4(),
            task,
            messages: Vec::new(),
            plan: None,
            results: Vec::new(),
            working_memory: WorkingMemory::new(10_000),
            workspace,
            config: AgentConfig {
                model: self.config.model.clone(),
                api_key: String::new(),
                base_url: String::new(),
                max_tokens: 4096,
                max_iterations: 50,
                context_window: 128_000,
                temperature: 0.0,
                auto_compact: true,
                sandbox_policy: sigmacode_core::types::SandboxPolicy::DiskRead,
                mcp_servers: Vec::new(),
            },
            iteration: 0,
        };

        let cancel = tokio_util::sync::CancellationToken::new();

        self.agent_handle = Some(tokio::spawn(async move {
            match agent.run(&mut state, cancel, Some(event_tx.clone())).await {
                Ok(_) => {}
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::Error {
                        message: e.to_string(),
                    });
                }
            }
        }));
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Planning { goal } => {
                self.logs.push(format!("[plan] {}", goal));
            }
            AgentEvent::PlanCreated { tasks } => {
                self.logs
                    .push(format!("[plan] {} tasks created", tasks.len()));
            }
            AgentEvent::TaskStarted {
                task_type,
                instruction,
                ..
            } => {
                self.logs
                    .push(format!("[exec] {}: {}", task_type, instruction));
                let short: String = instruction.chars().take(80).collect();
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("→ {} {}", task_type, short),
                    diff: None,
                });
            }
            AgentEvent::TaskCompleted { success, .. } => {
                self.logs.push(format!(
                    "[done] {}",
                    if success { "ok" } else { "failed" }
                ));
            }
            AgentEvent::ToolCallStarted {
                tool_name,
                args_summary: _,
            } => {
                self.logs.push(format!("[tool] {}", tool_name));
                let icon = match tool_name.as_str() {
                    "bash" => "$",
                    "read_file" => "read",
                    "write_file" => "write",
                    "edit_file" => "edit",
                    "glob" => "glob",
                    "grep" => "grep",
                    _ => "tool",
                };
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("  {} {}", icon, tool_name),
                    diff: None,
                });
            }
            AgentEvent::ToolCallCompleted {
                tool_name,
                success,
            } => {
                self.logs
                    .push(format!("[tool] {} → {}", tool_name, if success { "ok" } else { "err" }));
            }
            AgentEvent::Streaming { token } => {
                self.total_tokens += token.len() / 4;
                self.token_display = format_tokens(self.total_tokens);
                self.context_usage_pct =
                    ((self.total_tokens as f64 / 128_000.0) * 100.0).min(100.0) as u32;
                self.cost = (self.total_tokens as f64 / 1_000_000.0) * 2.50;

                if let Some(last) = self.messages.last_mut() {
                    if last.role == MessageRole::Assistant {
                        last.content.push_str(&token);
                        if let Some(tc) = extract_tool_call_display(&last.content) {
                            last.content = last.content[..tc.raw_start].trim_end().to_string();
                            self.messages.push(ChatMessage {
                                role: MessageRole::Tool,
                                content: tc.formatted,
                                diff: None,
                            });
                        }
                        return;
                    }
                }
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: token,
                    diff: None,
                });
            }
            AgentEvent::Error { message } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("error: {}", message),
                    diff: None,
                });
                self.state = AppState::Done;
            }
            AgentEvent::Done { summary } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: summary,
                    diff: None,
                });
                self.state = AppState::Done;
            }
            AgentEvent::PermissionRequest {
                tool_name,
                description,
                args_summary,
            } => {
                self.state = AppState::Permission;
                self.permission_pending = Some(PermissionRequest {
                    tool_name,
                    description,
                    args_summary,
                    allow_always: false,
                });
            }
            AgentEvent::DiffGenerated {
                file_path,
                old_content,
                new_content,
            } => {
                let diff = build_diff_view(&file_path, &old_content, &new_content);
                self.messages.push(ChatMessage {
                    role: MessageRole::Diff,
                    content: format!("Edit {}", file_path),
                    diff: Some(diff),
                });
            }
            AgentEvent::Thinking { content } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content,
                    diff: None,
                });
            }
            _ => {}
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    #[allow(dead_code)]
    pub fn is_idle(&self) -> bool {
        self.state == AppState::Idle || self.state == AppState::Done
    }
}

fn build_diff_view(file_path: &str, old: &str, new: &str) -> DiffView {
    let old_lines: Vec<DiffLine> = old
        .lines()
        .enumerate()
        .map(|(i, l)| DiffLine {
            line_num: i + 1,
            content: l.to_string(),
            is_added: false,
            is_removed: false,
        })
        .collect();

    let new_lines: Vec<DiffLine> = new
        .lines()
        .enumerate()
        .map(|(i, l)| DiffLine {
            line_num: i + 1,
            content: l.to_string(),
            is_added: !old.lines().any(|ol| ol == l),
            is_removed: false,
        })
        .collect();

    DiffView {
        file_path: file_path.to_string(),
        old_lines,
        new_lines,
    }
}

fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

struct ToolCallDisplay {
    raw_start: usize,
    formatted: String,
}

fn extract_tool_call_display(content: &str) -> Option<ToolCallDisplay> {
    if let Some(start) = content.find("```tool_call") {
        let content_start = start + 11;
        if let Some(end) = content[content_start..].find("```") {
            let json_str = content[content_start..content_start + end].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(tool_name) = parsed["tool"].as_str() {
                    let args = parsed.get("args").cloned().unwrap_or_default();
                    let formatted = format_tool_call(tool_name, &args);
                    return Some(ToolCallDisplay {
                        raw_start: start,
                        formatted,
                    });
                }
            }
        }
    }

    if let Some(start) = content.find("```json") {
        let content_start = start + 7;
        if let Some(end) = content[content_start..].find("```") {
            let json_str = content[content_start..content_start + end].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(tool_name) = parsed["tool"].as_str() {
                    let args = parsed.get("args").cloned().unwrap_or_default();
                    let formatted = format_tool_call(tool_name, &args);
                    return Some(ToolCallDisplay {
                        raw_start: start,
                        formatted,
                    });
                }
            }
        }
    }

    None
}

fn format_tool_call(tool_name: &str, args: &serde_json::Value) -> String {
    let icon = match tool_name {
        "bash" => "$",
        "read_file" => "read",
        "write_file" => "write",
        "edit_file" => "edit",
        "glob" => "glob",
        "grep" => "grep",
        _ => "tool",
    };

    match tool_name {
        "bash" => {
            let cmd = args["command"].as_str().unwrap_or("...");
            format!("  {} {}", icon, cmd)
        }
        "read_file" => {
            let path = args["path"].as_str().unwrap_or("?");
            format!("  {} {}", icon, path)
        }
        "write_file" => {
            let path = args["path"].as_str().unwrap_or("?");
            format!("  {} {}", icon, path)
        }
        "edit_file" => {
            let path = args["path"].as_str().unwrap_or("?");
            format!("  {} {}", icon, path)
        }
        "glob" => {
            let pattern = args["pattern"].as_str().unwrap_or("?");
            format!("  {} {}", icon, pattern)
        }
        "grep" => {
            let pattern = args["pattern"].as_str().unwrap_or("?");
            format!("  {} {}", icon, pattern)
        }
        _ => {
            format!("  {} {} {}", icon, tool_name, args)
        }
    }
}

fn load_config() -> anyhow::Result<AppConfig> {
    let api_key = std::env::var("SIGMACODE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default();

    let base_url = std::env::var("SIGMACODE_BASE_URL")
        .or_else(|_| std::env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| "https://api.openai.com/v1".into());

    let model = std::env::var("SIGMACODE_MODEL")
        .or_else(|_| std::env::var("OPENAI_MODEL"))
        .unwrap_or_else(|_| "gpt-4o".into());

    let provider_type = std::env::var("SIGMACODE_PROVIDER").unwrap_or_else(|_| "openai".into());

    let provider = match provider_type.as_str() {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            ProviderConfig::Anthropic {
                api_key: key,
                model: model.clone(),
            }
        }
        "ollama" => ProviderConfig::Ollama {
            base_url: Some(base_url.clone()),
            model: model.clone(),
        },
        _ => ProviderConfig::OpenAi {
            api_key,
            base_url: Some(base_url.clone()),
            model: model.clone(),
        },
    };

    Ok(AppConfig { provider, model })
}
