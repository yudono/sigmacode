use std::collections::VecDeque;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use serde::{Deserialize, Serialize};
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



const CONFIG_DIR: &str = ".sigma";
const CONFIG_FILE: &str = "config.yml";

// ── YAML Config ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigmaConfig {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Default for SigmaConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            api_key: String::new(),
            base_url: None,
            workspace: None,
        }
    }
}

fn config_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(CONFIG_DIR))
}

fn config_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join(CONFIG_FILE))
}

fn load_sigma_config() -> SigmaConfig {
    if let Some(path) = config_path() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_yaml::from_str::<SigmaConfig>(&content) {
                return cfg;
            }
        }
    }
    SigmaConfig::default()
}

fn save_sigma_config(cfg: &SigmaConfig) -> anyhow::Result<()> {
    let dir = config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    std::fs::create_dir_all(&dir)?;
    let content = serde_yaml::to_string(cfg)?;
    std::fs::write(dir.join(CONFIG_FILE), content)?;
    Ok(())
}

fn sigma_config_to_provider_config(cfg: &SigmaConfig) -> ProviderConfig {
    let base = cfg.base_url.clone().unwrap_or_default();
    match cfg.provider.as_str() {
        "anthropic" => ProviderConfig::Anthropic {
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        },
        "ollama" => ProviderConfig::Ollama {
            base_url: Some(if base.is_empty() { "http://localhost:11434".into() } else { base }),
            model: cfg.model.clone(),
        },
        "gemini" => ProviderConfig::Gemini {
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        },
        _ => ProviderConfig::OpenAi {
            api_key: cfg.api_key.clone(),
            base_url: Some(if base.is_empty() { "https://api.openai.com/v1".into() } else { base }),
            model: cfg.model.clone(),
        },
    }
}

// ── App ──

pub struct App {
    pub state: AppState,
    pub input: String,
    pub messages: Vec<ChatMessage>,
    pub agent_handle: Option<tokio::task::JoinHandle<()>>,
    pub event_rx: Option<mpsc::UnboundedReceiver<AgentEvent>>,
    pub event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    pub should_quit: bool,
    pub sigma_config: SigmaConfig,
    pub provider_config: ProviderConfig,
    pub current_tab: Tab,
    pub logs: Vec<String>,
    pub scroll_offset: usize,
    pub follow: bool,
    pub tick_count: usize,
    pub token_display: String,
    pub context_usage_pct: u32,
    pub cost: f64,
    pub total_tokens: usize,
    pub permission_pending: Option<PermissionRequest>,
    pub setup: SetupState,
    pub last_task: Option<String>,
    pub cmd_choices: Vec<CmdChoice>,
    pub cmd_selected: usize,
    pub queue: VecDeque<String>,
    pub pending_tool_call: String,
    pub in_tool_result: bool,
    pub in_tool_call_text: bool,
}

pub struct CmdChoice {
    pub name: &'static str,
    pub desc: &'static str,
}

pub fn all_commands() -> Vec<CmdChoice> {
    vec![
        CmdChoice { name: "/help", desc: "Show available commands" },
        CmdChoice { name: "/clear", desc: "Clear chat history" },
        CmdChoice { name: "/memory", desc: "Show token/cost status" },
        CmdChoice { name: "/resume", desc: "Re-run last task" },
        CmdChoice { name: "/models", desc: "Switch model" },
        CmdChoice { name: "/agents", desc: "Show agent info" },
        CmdChoice { name: "/skills", desc: "List available tools" },
        CmdChoice { name: "/config", desc: "Show configuration" },
        CmdChoice { name: "/compact", desc: "Compact context" },
        CmdChoice { name: "/version", desc: "Show version" },
        CmdChoice { name: "/quit", desc: "Exit sigmaCode" },
    ]
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
    pub tool_output: bool,
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

#[allow(dead_code)]
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
    Workspace,
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
        let sigma_config = load_sigma_config();
        let needs_setup = needs_setup_wizard(&sigma_config);
        let provider_config = sigma_config_to_provider_config(&sigma_config);

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
            sigma_config,
            provider_config,
            current_tab: Tab::Chat,
            logs: Vec::new(),
            scroll_offset: 0,
            follow: true,
            tick_count: 0,
            token_display: "0 tokens".into(),
            context_usage_pct: 0,
            cost: 0.0,
            total_tokens: 0,
            permission_pending: None,
            setup: SetupState::default(),
            last_task: None,
            cmd_choices: Vec::new(),
            cmd_selected: 0,
            queue: VecDeque::new(),
            pending_tool_call: String::new(),
            in_tool_result: false,
            in_tool_call_text: false,
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
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    self.follow = false;
                }
                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                    if self.scroll_offset == 0 {
                        self.follow = true;
                    }
                }
                _ => {}
            },
            AppState::Input => {
                if !self.cmd_choices.is_empty() {
                    // Command autocomplete mode
                    match key.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Down => {
                            self.cmd_selected = (self.cmd_selected + 1) % self.cmd_choices.len();
                        }
                        crossterm::event::KeyCode::Up => {
                            if self.cmd_selected == 0 {
                                self.cmd_selected = self.cmd_choices.len() - 1;
                            } else {
                                self.cmd_selected -= 1;
                            }
                        }
                        crossterm::event::KeyCode::Enter => {
                            if let Some(choice) = self.cmd_choices.get(self.cmd_selected) {
                                self.input = choice.name.to_string();
                                self.cmd_choices.clear();
                                self.cmd_selected = 0;
                            }
                        }
                        crossterm::event::KeyCode::Esc => {
                            self.cmd_choices.clear();
                            self.cmd_selected = 0;
                        }
                        crossterm::event::KeyCode::Backspace => {
                            self.input.pop();
                            self.update_cmd_choices();
                            if self.input.is_empty() || !self.input.starts_with('/') {
                                self.cmd_choices.clear();
                                self.cmd_selected = 0;
                            }
                        }
                        crossterm::event::KeyCode::Char(c) => {
                            self.input.push(c);
                            self.update_cmd_choices();
                            if !self.input.starts_with('/') {
                                self.cmd_choices.clear();
                                self.cmd_selected = 0;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Normal input mode
                    match key.code {
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
                            if self.input == "/" {
                                self.update_cmd_choices();
                            }
                        }
                        _ => {}
                    }
                }
            }
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
            AppState::Running => match key.code {
                crossterm::event::KeyCode::Enter => {
                    if !self.input.trim().is_empty() {
                        let task = self.input.trim().to_string();
                        self.input.clear();
                        if task.starts_with('/') {
                            self.handle_slash_command(&task);
                        } else {
                            self.queue.push_back(task);
                        }
                    }
                }
                crossterm::event::KeyCode::Backspace => {
                    self.input.pop();
                }
                crossterm::event::KeyCode::Char(c) => {
                    self.input.push(c);
                }
                _ => {}
            },
        }
    }

    fn update_cmd_choices(&mut self) {
        let query = self.input.trim_start_matches('/');
        self.cmd_choices = all_commands()
            .into_iter()
            .filter(|c| c.name[1..].contains(query))
            .collect();
        self.cmd_selected = 0;
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.scroll_offset > 0 {
                    self.scroll_offset = self.scroll_offset.saturating_sub(3);
                    self.follow = false;
                }
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Click to enter input mode if in idle/done state
                if self.state == AppState::Idle || self.state == AppState::Done {
                    self.state = AppState::Input;
                }
            }
            _ => {}
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
                            self.setup.base_url = "http://localhost:11434".into();
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
                    // Ask for custom base_url (optional)
                    self.setup.step = SetupStep::BaseUrl;
                }
                SetupStep::BaseUrl => {
                    let url = self.input.trim().to_string();
                    self.input.clear();
                    if !url.is_empty() {
                        self.setup.base_url = url;
                    }
                    self.setup.step = SetupStep::Workspace;
                }
                SetupStep::Workspace => {
                    let ws = self.input.trim().to_string();
                    self.input.clear();
                    if !ws.is_empty() {
                        self.sigma_config.workspace = Some(ws);
                    }
                    self.setup.step = SetupStep::Done;
                    self.finish_setup();
                }
                SetupStep::Done => {}
            },
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
        self.sigma_config = SigmaConfig {
            provider: self.setup.provider_choice.clone(),
            model: self.setup.model.clone(),
            api_key: self.setup.api_key.clone(),
            base_url: if self.setup.base_url.is_empty() {
                None
            } else {
                Some(self.setup.base_url.clone())
            },
            workspace: self.sigma_config.workspace.clone(),
        };

        let _ = save_sigma_config(&self.sigma_config);
        self.provider_config = sigma_config_to_provider_config(&self.sigma_config);

        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!(
                "Setup complete!\nProvider: {}\nModel: {}\nConfig saved to ~/.sigma/config.yml",
                self.sigma_config.provider, self.sigma_config.model
            ),
            diff: None,
        tool_output: false,
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
        tool_output: false,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: response,
            diff: None,
        tool_output: false,
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
                self.sigma_config.model
            );
        }
        self.sigma_config.model = model.to_string();
        self.provider_config = sigma_config_to_provider_config(&self.sigma_config);
        let _ = save_sigma_config(&self.sigma_config);
        format!("Model switched to: {}", model)
    }

    fn cmd_agents(&self) -> String {
        format!(
            "Agent: sigmaCode v{}\nProvider: {}\nModel: {}\nWorkspace: {}",
            env!("CARGO_PKG_VERSION"),
            self.sigma_config.provider,
            self.sigma_config.model,
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
        let workspace = self.sigma_config.workspace.as_deref().unwrap_or("(current dir)");
        format!(
            "Provider: {}\nModel: {}\nBase URL: {}\nWorkspace: {}\nConfig: ~/.sigma/config.yml",
            self.sigma_config.provider,
            self.sigma_config.model,
            self.sigma_config.base_url.as_deref().unwrap_or("(default)"),
            workspace
        )
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
        tool_output: false,
        });

        self.state = AppState::Running;
        self.event_rx = Some(event_rx);
        self.event_tx = Some(event_tx.clone());
        self.scroll_offset = 0;

        let provider = create_provider(&self.provider_config);
        let tools = ToolRouter::default();
        let workspace = self.sigma_config.workspace.as_ref()
            .and_then(|w| {
                let p = std::path::PathBuf::from(w);
                if p.exists() { Some(p) } else { None }
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
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
                model: self.sigma_config.model.clone(),
                api_key: self.sigma_config.api_key.clone(),
                base_url: self.sigma_config.base_url.clone().unwrap_or_default(),
                max_tokens: 4096,
                max_iterations: 50,
                context_window: 128_000,
                temperature: 0.0,
                auto_compact: true,
                sandbox_policy: sigmacode_core::types::SandboxPolicy::DiskRead,
                mcp_servers: Vec::new(),
            },
            iteration: 0,
            event_tx: None,
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
                tool_output: false,
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
                args_summary,
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
                let detail = match tool_name.as_str() {
                    "bash" => {
                        if let Some(cmd) = args_summary.strip_prefix("command=") {
                            cmd.trim_matches('"').to_string()
                        } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&args_summary) {
                            parsed["command"].as_str().unwrap_or(&args_summary).to_string()
                        } else {
                            args_summary
                        }
                    }
                    "write_file" | "edit_file" | "read_file" => {
                        if let Some(path) = args_summary.split(", ").next() {
                            if let Some(p) = path.strip_prefix("path=") {
                                p.trim_matches('"').to_string()
                            } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&args_summary) {
                                parsed["path"].as_str().unwrap_or(&args_summary).to_string()
                            } else {
                                args_summary
                            }
                        } else {
                            args_summary
                        }
                    }
                    _ => {
                        if args_summary.is_empty() {
                            tool_name.clone()
                        } else {
                            args_summary
                        }
                    }
                };
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("  {} {}", icon, detail),
                    diff: None,
                tool_output: false,
                });
            }
            AgentEvent::ToolCallCompleted {
                tool_name,
                success,
            } => {
                self.logs
                    .push(format!("[tool] {} → {}", tool_name, if success { "ok" } else { "err" }));
                let status_icon = if success { "✓" } else { "✗" };
                let status_text = if success { "done" } else { "failed" };
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("  {} {} {}", status_icon, tool_name, status_text),
                    diff: None,
                    tool_output: true,
                });
            }
            AgentEvent::ToolOutput { tool_call_id: _, line } => {
                let line = strip_tool_result_tags(&line);
                if line.trim().is_empty() { return; }
                self.logs.push(format!("[output] {}", line));
                // Append to last assistant message or create a new tool output message
                if let Some(last) = self.messages.last_mut() {
                    if last.role == MessageRole::Tool && last.tool_output {
                        last.content.push('\n');
                        last.content.push_str(&line);
                        return;
                    }
                }
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: line,
                    diff: None,
                    tool_output: true,
                });
            }
            AgentEvent::Streaming { token } => {
                self.total_tokens += token.len() / 4;
                self.token_display = format_tokens(self.total_tokens);
                self.context_usage_pct =
                    ((self.total_tokens as f64 / 128_000.0) * 100.0).min(100.0) as u32;
                self.cost = (self.total_tokens as f64 / 1_000_000.0) * 2.50;

                self.pending_tool_call.push_str(&token);

                // State machine: suppress <tool_result>...</tool_result> blocks from MiMo API
                if self.in_tool_result {
                    if let Some(end) = self.pending_tool_call.find("</tool_result>") {
                        let after = &self.pending_tool_call[end + 14..];
                        self.pending_tool_call = after.to_string();
                        self.in_tool_result = false;
                    } else if let Some(end) = self.pending_tool_call.find("</tool_result") {
                        let after = &self.pending_tool_call[end + 13..];
                        self.pending_tool_call = after.to_string();
                        self.in_tool_result = false;
                    } else {
                        self.pending_tool_call.clear();
                        return;
                    }
                }

                if self.pending_tool_call.contains("<tool_result>") {
                    if let Some(end) = self.pending_tool_call.find("</tool_result>") {
                        let before_end = self.pending_tool_call[..self.pending_tool_call.find("<tool_result>").unwrap()].to_string();
                        let after = &self.pending_tool_call[end + 14..];
                        self.pending_tool_call = format!("{}{}", before_end, after);
                    } else {
                        let before = self.pending_tool_call[..self.pending_tool_call.find("<tool_result>").unwrap()].to_string();
                        self.pending_tool_call = before;
                        self.in_tool_result = true;
                        if self.pending_tool_call.trim().is_empty() {
                            return;
                        }
                    }
                }

                // Suppress tool_call text markers from MiMo API
                if self.in_tool_call_text {
                    // Count braces to find end of JSON object
                    let mut depth = 0i32;
                    let mut ended = false;
                    for (i, ch) in self.pending_tool_call.char_indices() {
                        match ch {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    // Found end of JSON — discard everything up to here
                                    self.pending_tool_call = self.pending_tool_call[i + 1..].to_string();
                                    self.in_tool_call_text = false;
                                    ended = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if !ended {
                        self.pending_tool_call.clear();
                        return;
                    }
                }

                // Detect tool_call text blocks: "tool_call\n{...}" or "tool_call\n"
                {
                    let pt = self.pending_tool_call.clone();
                    if let Some(tc_pos) = pt.find("tool_call") {
                        let after_tc = &pt[tc_pos + 9..];
                        if after_tc.starts_with('{') || after_tc.starts_with('\n') || after_tc.is_empty() {
                            let before = pt[..tc_pos].to_string();
                            self.pending_tool_call = before;
                            if after_tc.starts_with('{') {
                                self.in_tool_call_text = true;
                                // Process the opening brace immediately
                            } else if after_tc.starts_with('\n') {
                                self.pending_tool_call = after_tc[1..].to_string();
                            }
                            if self.pending_tool_call.trim().is_empty() {
                                return;
                            }
                        }
                    }
                }

                // If we're inside a tool_call block, just buffer and wait for it to complete.
                let pt = &self.pending_tool_call;
                let in_marker_block = pt.contains("```tool_call") || pt.contains("```json");
                // Also detect raw JSON tool calls without markdown wrappers: {"tool": "...", "args": ...}
                let looks_like_raw_tool_json = pt.contains("\"tool\":") && pt.contains("\"args\"") && pt.len() > 20;

                if in_marker_block {
                    // Find the closing ``` to know when the block is done
                    let open_marker = if let Some(p) = self.pending_tool_call.find("```tool_call") {
                        p + 12 // "```tool_call" is 12 chars
                    } else if let Some(p) = self.pending_tool_call.find("```json") {
                        p + 7
                    } else {
                        0
                    };
                    let after_open = &self.pending_tool_call[open_marker..];
                    if after_open.contains("```") {
                        // Complete block found — discard it entirely (ToolCallStarted handles display)
                        self.pending_tool_call.clear();
                    }
                    // Don't display anything while inside a tool_call block
                    return;
                }

                // Handle raw JSON tool calls (no markdown wrappers)
                if looks_like_raw_tool_json {
                    // Try to detect if the JSON is complete by checking for balanced braces
                    let trimmed = self.pending_tool_call.trim();
                    // Find the last { ... } block that looks like a tool call
                    if let Some(start) = trimmed.rfind("{\"tool\":") {
                        let json_part = &trimmed[start..];
                        if json_part.ends_with('}') {
                            // Looks complete — discard it (engine will parse it)
                            self.pending_tool_call.clear();
                            return;
                        }
                    }
                    // Still incomplete — buffer and don't display
                    return;
                }

                // Check for partial tool_call markers at end of buffer (split across tokens)
                // If buffer ends with a prefix of "```tool_call" or "```json", keep buffering
                let p = &self.pending_tool_call;
                let might_be_marker = p.ends_with("```tool_call")
                    || p.ends_with("```tool_cal")
                    || p.ends_with("```tool_ca")
                    || p.ends_with("```tool_c")
                    || p.ends_with("```tool_")
                    || p.ends_with("```tool")
                    || p.ends_with("```too")
                    || p.ends_with("```to")
                    || p.ends_with("```t")
                    || p.ends_with("```json")
                    || p.ends_with("```jso")
                    || p.ends_with("```js")
                    || p.ends_with("```j");

                if might_be_marker {
                    // Might be entering a tool_call block, buffer and wait for more tokens
                    return;
                }

                // Normal text - find the last complete line and flush it
                if let Some(last_newline) = self.pending_tool_call.rfind('\n') {
                    let complete = self.pending_tool_call[..last_newline].to_string();
                    let remaining = self.pending_tool_call[last_newline + 1..].to_string();
                    self.pending_tool_call = remaining;

                    if !complete.is_empty() {
                        let complete = strip_tool_result_tags(&complete);
                        if !complete.trim().is_empty() {
                            if let Some(last) = self.messages.last_mut() {
                                if last.role == MessageRole::Assistant {
                                    if !last.content.is_empty() {
                                        last.content.push('\n');
                                    }
                                    last.content.push_str(&complete);
                                    return;
                                }
                            }
                            self.messages.push(ChatMessage {
                                role: MessageRole::Assistant,
                                content: complete,
                                diff: None,
                            tool_output: false,
                            });
                        }
                    }
                }
            }
            AgentEvent::Error { message } => {
                self.flush_pending_tool_call();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("error: {}", message),
                    diff: None,
                tool_output: false,
                });
                self.process_next_in_queue();
            }
            AgentEvent::Done { summary } => {
                self.flush_pending_tool_call();
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: summary,
                    diff: None,
                tool_output: false,
                });
                self.process_next_in_queue();
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
                    tool_output: false,
                });
            }
            AgentEvent::Thinking { content } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content,
                    diff: None,
                tool_output: false,
                });
            }
            AgentEvent::AnalysisComplete { goals, constraints, success_criteria } => {
                let mut content = String::from("**Analyzing task...**\n");
                if !goals.is_empty() {
                    content.push_str(&format!("Goals: {}\n", goals.join(", ")));
                }
                if !constraints.is_empty() {
                    content.push_str(&format!("Constraints: {}\n", constraints.join(", ")));
                }
                if !success_criteria.is_empty() {
                    content.push_str(&format!("Criteria: {}", success_criteria.join(", ")));
                }
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content,
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::PlanValidated { issues } => {
                let content = if issues.is_empty() {
                    "**Plan validated** — no issues".into()
                } else {
                    format!("**Plan validated** — {} issues: {}", issues.len(), issues.join(", "))
                };
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content,
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::VerificationStarted { step } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: format!("**Verifying:** {}", step),
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::VerificationPassed { step } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("  ✓ {} passed", step),
                    diff: None,
                    tool_output: true,
                });
            }
            AgentEvent::VerificationFailed { step, errors } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: format!("  ✗ {} failed: {}", step, errors.join(", ")),
                    diff: None,
                    tool_output: true,
                });
            }
            AgentEvent::Criticking { errors } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: format!("**Analyzing errors:** {}", errors.join(", ")),
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::CriticResult { root_cause, fix } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: format!("**Root cause:** {}\n**Fix:** {}", root_cause, fix),
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::Replanning { reason, attempt } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: format!("**Replanning** (attempt {}): {}", attempt, reason),
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::Reviewing => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: "**Reviewing code...**".into(),
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::ReviewComplete { score, issues_count } => {
                let content = format!("**Review complete** — score: {}/100, {} issues", score, issues_count);
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content,
                    diff: None,
                    tool_output: false,
                });
            }
            AgentEvent::Finalizing => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Thought,
                    content: "**Finalizing...**".into(),
                    diff: None,
                    tool_output: false,
                });
            }
            _ => {}
        }
    }

    fn process_next_in_queue(&mut self) {
        if let Some(task) = self.queue.pop_front() {
            self.spawn_agent(task);
        } else {
            self.state = AppState::Done;
        }
    }

    fn flush_pending_tool_call(&mut self) {
        if self.pending_tool_call.is_empty() {
            return;
        }
        let remaining = strip_tool_result_tags(&self.pending_tool_call.trim());
        self.pending_tool_call.clear();
        self.in_tool_result = false;
        self.in_tool_call_text = false;
        if remaining.is_empty() {
            return;
        }
        // If it looks like a partial tool_call, try to parse it anyway
        if let Some(tc) = extract_tool_call_display(&remaining) {
            let before = remaining[..tc.raw_start].trim();
            if !before.is_empty() {
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: before.to_string(),
                    diff: None,
                tool_output: false,
                });
            }
            self.messages.push(ChatMessage {
                role: MessageRole::Tool,
                content: tc.formatted,
                diff: None,
            tool_output: false,
            });
        } else {
            // Plain text
            self.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: remaining,
                diff: None,
            tool_output: false,
            });
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

// ── Helpers ──

fn needs_setup_wizard(config: &SigmaConfig) -> bool {
    if config.model.is_empty() {
        return true;
    }
    match config.provider.as_str() {
        "ollama" => false,
        _ => config.api_key.is_empty() || config.api_key == "your-api-key-here",
    }
}

fn strip_tool_result_tags(text: &str) -> String {
    let mut result = text.to_string();
    loop {
        if let Some(start) = result.find("<tool_result>") {
            if let Some(end) = result[start..].find("</tool_result>") {
                result = format!("{}{}", &result[..start], &result[start + end + 14..]);
            } else {
                result = result[..start].to_string();
                break;
            }
        } else {
            break;
        }
    }
    result
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
        let content_start = start + 12; // "```tool_call" is 12 chars
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
