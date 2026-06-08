use crate::agent::engine::Agent;
use crate::llm::LlmProvider;
use crate::rate_limit::LlmRateLimiter;
use crate::security::SecurityGuard;
use crate::types::{AgentConfig, AgentEvent, CompletionOptions, LlmEvent, Message, Plan, WorkingMemory};
use async_trait::async_trait;
use graph_flow::{self as gf, Context, ExecutionStatus, FlowRunner, GraphBuilder, GraphError, InMemorySessionStorage, NextAction, Session, SessionStorage, TaskResult};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

// ── Node: Security Check ──

struct SecurityCheckNode {
    guard: SecurityGuard,
}

#[async_trait]
impl gf::Task for SecurityCheckNode {
    fn id(&self) -> &str { "security_check" }

    async fn run(&self, ctx: Context) -> std::result::Result<TaskResult, GraphError> {
        let task: String = ctx.get("task").await.unwrap_or_default();

        if let Err(e) = self.guard.scan_input(&task) {
            ctx.set("security_blocked", true).await;
            ctx.set("security_reason", e.to_string()).await;
            return Ok(TaskResult::new(
                Some(format!("Security blocked: {}", e)),
                NextAction::End,
            ));
        }

        ctx.set("security_blocked", false).await;
        Ok(TaskResult::new(None, NextAction::Continue))
    }
}

// ── Node: Plan (uses LLM directly) ──

struct PlanNode {
    provider: Arc<dyn LlmProvider>,
}

#[async_trait]
impl gf::Task for PlanNode {
    fn id(&self) -> &str { "plan" }

    async fn run(&self, ctx: Context) -> std::result::Result<TaskResult, GraphError> {
        let blocked: bool = ctx.get("security_blocked").await.unwrap_or(false);
        if blocked {
            let reason: String = ctx.get("security_reason").await.unwrap_or_default();
            return Ok(TaskResult::new(Some(reason), NextAction::End));
        }

        let task: String = ctx.get("task").await.unwrap_or_default();

        let system = "You are a planner. Given a user task, output a JSON plan.\n\n\
            Output ONLY valid JSON:\n\
            {\"goal\":\"...\",\"tasks\":[{\"id\":\"task_1\",\"task_type\":\"write_file\",\"instruction\":\"...\",\"depends_on\":[]}]}\n\n\
            task_type must be one of: write_file, read_file, edit_file, bash, grep, glob"
            .to_string();

        let messages = vec![
            Message::System { content: system },
            Message::User { content: format!("Create a plan for: {task}") },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(2048),
            ..Default::default()
        };

        let mut stream = self
            .provider
            .complete_stream(&messages, &[], &options)
            .await
            .map_err(|e| GraphError::TaskExecutionFailed(e.to_string()))?;

        let mut content = String::new();
        use futures::StreamExt;
        while let Some(event) = stream.next().await {
            match event {
                Ok(LlmEvent::ContentDelta(t)) => content.push_str(&t),
                Ok(LlmEvent::Done { .. }) => break,
                Ok(LlmEvent::Error(e)) => return Err(GraphError::TaskExecutionFailed(e)),
                _ => {}
            }
        }

        let plan: Plan = serde_json::from_str(&content).unwrap_or_else(|_| Plan {
            goal: task.clone(),
            tasks: vec![crate::types::Task {
                id: "task_1".into(),
                task_type: "bash".into(),
                instruction: task.clone(),
                depends_on: vec![],
            }],
        });

        ctx.set("plan", plan).await;
        Ok(TaskResult::new(Some("Plan created".into()), NextAction::Continue))
    }
}

// ── Node: Execute tasks (uses Agent::run_task for ReAct loop) ──

struct ExecuteNode {
    agent: Arc<Agent>,
    #[allow(dead_code)]
    security: SecurityGuard,
    #[allow(dead_code)]
    rate_limiter: LlmRateLimiter,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
}

#[async_trait]
impl gf::Task for ExecuteNode {
    fn id(&self) -> &str { "execute" }

    async fn run(&self, ctx: Context) -> std::result::Result<TaskResult, GraphError> {
        let plan: Plan = ctx.get("plan").await.unwrap_or(Plan {
            goal: String::new(),
            tasks: vec![],
        });
        let model: String = ctx.get("model").await.unwrap_or_default();
        let api_key: String = ctx.get("api_key").await.unwrap_or_default();
        let base_url: String = ctx.get("base_url").await.unwrap_or_default();
        let workspace: String = ctx.get("workspace").await.unwrap_or_default();

        let mut all_output = String::new();

        for task in &plan.tasks {
            let _ = self.event_tx.send(AgentEvent::TaskStarted {
                task_id: task.id.clone(),
                task_type: task.task_type.clone(),
                instruction: task.instruction.clone(),
            });

            let sys = format!(
                "You are an agent. Execute this task using tools.\n\
                 Tool: {}\nInstruction: {}\n\
                 Output tool calls as: ```tool_call\n{{\"tool\":\"...\",\"args\":{{...}}}}\n```",
                task.task_type, task.instruction,
            );

            let mut state = crate::types::AgentState {
                session_id: Uuid::new_v4(),
                task: task.instruction.clone(),
                messages: vec![Message::System { content: sys }],
                plan: Some(plan.clone()),
                results: vec![],
                working_memory: WorkingMemory::new(10_000),
                workspace: std::path::PathBuf::from(&workspace),
                config: AgentConfig {
                    model: model.clone(),
                    api_key: api_key.clone(),
                    base_url: base_url.clone(),
                    max_tokens: 4096,
                    max_iterations: 30,
                    context_window: 128_000,
                    temperature: 0.0,
                    auto_compact: true,
                    sandbox_policy: crate::types::SandboxPolicy::DiskRead,
                    mcp_servers: vec![],
                },
                iteration: 0,
                event_tx: None,
            };

            let cancel = tokio_util::sync::CancellationToken::new();
            let result = self
                .agent
                .run_task(&mut state, cancel, Some(self.event_tx.clone()))
                .await;

            let output = match result {
                Ok(o) => {
                    let _ = self.event_tx.send(AgentEvent::TaskCompleted {
                        task_id: task.id.clone(),
                        success: true,
                        output: o.clone(),
                    });
                    o
                }
                Err(e) => {
                    let msg = e.to_string();
                    let _ = self.event_tx.send(AgentEvent::TaskCompleted {
                        task_id: task.id.clone(),
                        success: false,
                        output: msg.clone(),
                    });
                    msg
                }
            };

            all_output.push_str(&output);
            all_output.push('\n');
        }

        Ok(TaskResult::new(Some(all_output), NextAction::End))
    }
}

// ── Graph Engine ──

pub struct GraphEngine {
    agent: Arc<Agent>,
    security: SecurityGuard,
    rate_limiter: LlmRateLimiter,
}

impl GraphEngine {
    pub fn new(
        agent: Agent,
        _tools: crate::tools::ToolRouter,
        _context_builder: crate::context::ContextBuilder,
    ) -> Self {
        let security = agent.security.clone();
        let rate_limiter = agent.rate_limiter.clone();
        Self {
            agent: Arc::new(agent),
            security,
            rate_limiter,
        }
    }

    pub async fn run(
        &self,
        task: &str,
        config: &AgentConfig,
        workspace: &std::path::Path,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<String, String> {
        let security_node = SecurityCheckNode {
            guard: self.security.clone(),
        };
        let plan_node = PlanNode {
            provider: self.agent.provider(),
        };
        let exec_node = ExecuteNode {
            agent: self.agent.clone(),
            security: self.security.clone(),
            rate_limiter: self.rate_limiter.clone(),
            event_tx: event_tx.clone(),
        };

        let graph = Arc::new(
            GraphBuilder::new("sigmacode")
                .add_task(Arc::new(security_node))
                .add_task(Arc::new(plan_node))
                .add_task(Arc::new(exec_node))
                .set_start_task("security_check")
                .add_edge("security_check", "plan")
                .add_edge("plan", "execute")
                .build(),
        );

        let storage = Arc::new(InMemorySessionStorage::new());
        let runner = FlowRunner::new(graph, storage.clone());

        let session_id = format!("session-{}", Uuid::new_v4());
        let session = Session::new_from_task(session_id.clone(), "security_check");
        session.context.set("task", task.to_string()).await;
        session.context.set("model", config.model.clone()).await;
        session.context.set("api_key", config.api_key.clone()).await;
        session.context.set("base_url", config.base_url.clone()).await;
        session.context.set("workspace", workspace.to_string_lossy().to_string()).await;
        storage.save(session).await.map_err(|e| e.to_string())?;

        let result = runner.run(&session_id).await.map_err(|e| e.to_string())?;

        match result.status {
            ExecutionStatus::Completed => Ok(result.response.unwrap_or_default()),
            ExecutionStatus::Error(e) => Err(e),
            ExecutionStatus::Paused { .. } => Ok(result.response.unwrap_or_default()),
            ExecutionStatus::WaitingForInput => Ok(result.response.unwrap_or_default()),
        }
    }
}
