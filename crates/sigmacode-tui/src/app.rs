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
        })
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.state {
            AppState::Idle | AppState::Done => {
                match key.code {
                    crossterm::event::KeyCode::Char('i') => {
                        self.state = AppState::Input;
                    }
                    crossterm::event::KeyCode::Char('l') => {
                        self.current_tab = Tab::Logs;
                    }
                    crossterm::event::KeyCode::Char('c') => {
                        self.current_tab = Tab::Chat;
                    }
                    _ => {}
                }
            }
            AppState::Input => match key.code {
                crossterm::event::KeyCode::Enter => {
                    if !self.input.trim().is_empty() {
                        let task = self.input.clone();
                        self.input.clear();
                        self.spawn_agent(task);
                    }
                }
                crossterm::event::KeyCode::Esc => {
                    self.state = AppState::Idle;
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
                self.logs.push(format!("Planning: {}", goal));
            }
            AgentEvent::PlanCreated { tasks } => {
                self.logs
                    .push(format!("Plan created with {} tasks", tasks.len()));
                let task_list: Vec<String> = tasks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("  {}. {}", i + 1, t.instruction))
                    .collect();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("Plan:\n{}", task_list.join("\n")),
                });
            }
            AgentEvent::TaskStarted {
                task_type,
                instruction,
                ..
            } => {
                self.logs
                    .push(format!("Task: {} - {}", task_type, instruction));
            }
            AgentEvent::TaskCompleted {
                success, output, ..
            } => {
                self.logs.push(format!(
                    "Task completed: {}",
                    if success { "success" } else { "failed" }
                ));
                if !output.is_empty() && output.len() < 500 {
                    self.logs.push(format!("  Output: {}", output));
                }
            }
            AgentEvent::ToolCallStarted { tool_name, .. } => {
                self.logs.push(format!("Tool call: {}", tool_name));
            }
            AgentEvent::ToolCallCompleted {
                tool_name,
                success,
            } => {
                self.logs.push(format!(
                    "Tool {} completed: {}",
                    tool_name,
                    if success { "ok" } else { "failed" }
                ));
            }
            AgentEvent::Streaming { token } => {
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
                    content: format!("Error: {}", message),
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
