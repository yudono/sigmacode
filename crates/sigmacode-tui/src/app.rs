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
    pub should_quit: bool,
    pub config: AppConfig,
    pub current_tab: Tab,
    pub logs: Vec<String>,
    pub scroll_offset: usize,
    pub tick_count: usize,
    pub token_display: String,
    pub context_usage_pct: u32,
    pub cost: f64,
    pub token_usage: String,
    pub total_tokens: usize,
}

#[derive(PartialEq)]
pub enum AppState {
    Idle,
    Input,
    Running,
    Done,
}

#[derive(PartialEq)]
pub enum Tab {
    Chat,
    Logs,
}

pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

pub struct AppConfig {
    pub provider: ProviderConfig,
    pub model: String,
}

impl App {
    pub async fn new() -> anyhow::Result<Self> {
        let config = load_config()?;

        Ok(Self {
            state: AppState::Idle,
            input: String::new(),
            messages: Vec::new(),
            agent_handle: None,
            event_rx: None,
            should_quit: false,
            config,
            current_tab: Tab::Chat,
            logs: Vec::new(),
            scroll_offset: 0,
            tick_count: 0,
            token_display: "0 tokens".into(),
            context_usage_pct: 0,
            cost: 0.0,
            token_usage: "0".into(),
            total_tokens: 0,
        })
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.state {
            AppState::Idle | AppState::Done => match key.code {
                crossterm::event::KeyCode::Char('i') => {
                    self.state = AppState::Input;
                }
                crossterm::event::KeyCode::Char('l') => {
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
                crossterm::event::KeyCode::Char('g') => {
                    self.scroll_offset = usize::MAX;
                }
                crossterm::event::KeyCode::Char('G') => {
                    self.scroll_offset = 0;
                }
                _ => {}
            },
            AppState::Input => match key.code {
                crossterm::event::KeyCode::Enter => {
                    if !self.input.trim().is_empty() {
                        let task = self.input.clone();
                        self.input.clear();
                        self.spawn_agent(task);
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
            AppState::Running => {}
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
        });

        self.state = AppState::Running;
        self.event_rx = Some(event_rx);
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
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("planning: {}", goal),
                });
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
                self.logs.push(format!("[exec] {}: {}", task_type, instruction));
                let short: String = instruction.chars().take(60).collect();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("{}: {}", task_type, short),
                });
            }
            AgentEvent::TaskCompleted {
                success, output: _, ..
            } => {
                self.logs.push(format!(
                    "[done] {}",
                    if success { "ok" } else { "failed" }
                ));
            }
            AgentEvent::ToolCallStarted { tool_name, .. } => {
                self.logs.push(format!("[tool] {}", tool_name));
                self.messages.push(ChatMessage {
                    role: MessageRole::Tool,
                    content: tool_name,
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
                self.token_usage = format_tokens(self.total_tokens);
                self.context_usage_pct = ((self.total_tokens as f64 / 128_000.0) * 100.0) as u32;
                self.cost = (self.total_tokens as f64 / 1_000_000.0) * 2.50;

                if let Some(last) = self.messages.last_mut() {
                    if last.role == MessageRole::Assistant {
                        last.content.push_str(&token);
                        return;
                    }
                }
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: token,
                });
            }
            AgentEvent::Error { message } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("error: {}", message),
                });
                self.state = AppState::Done;
            }
            AgentEvent::Done { summary } => {
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: summary,
                });
                self.state = AppState::Done;
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

    pub fn is_idle(&self) -> bool {
        self.state == AppState::Idle || self.state == AppState::Done
    }
}

fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M tokens", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K tokens", n as f64 / 1_000.0)
    } else {
        format!("{} tokens", n)
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
