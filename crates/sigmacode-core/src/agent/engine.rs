use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::context::ContextBuilder;
use crate::error::{Result, SigmaError};
use crate::llm::LlmProvider;
use crate::rate_limit::{LlmRateLimiter, RateLimitResult};
use crate::security::SecurityGuard;
use crate::tools::{ToolContext, ToolRouter};
use crate::types::*;

use async_trait::async_trait;
use graph_flow::{self as gf, Context, ExecutionStatus, FlowRunner, GraphBuilder, GraphError, InMemorySessionStorage, NextAction, Session, SessionStorage, TaskResult as GfTaskResult};

// ── Graph Pipeline Tasks ──

struct SecurityCheckTask {
    guard: SecurityGuard,
}

#[async_trait]
impl gf::Task for SecurityCheckTask {
    fn id(&self) -> &str { "security_check" }

    async fn run(&self, ctx: Context) -> std::result::Result<GfTaskResult, GraphError> {
        let task: String = ctx.get("task").await.unwrap_or_default();

        if let Err(e) = self.guard.scan_input(&task) {
            ctx.set("security_blocked", true).await;
            ctx.set("security_reason", e.to_string()).await;
            return Ok(GfTaskResult::new(
                Some(format!("Security blocked: {}", e)),
                NextAction::End,
            ));
        }

        ctx.set("security_blocked", false).await;
        Ok(GfTaskResult::new(None, NextAction::Continue))
    }
}

struct PlanTask {
    provider: Arc<dyn LlmProvider>,
    tool_defs: Vec<ToolDefinition>,
}

#[async_trait]
impl gf::Task for PlanTask {
    fn id(&self) -> &str { "plan" }

    async fn run(&self, ctx: Context) -> std::result::Result<GfTaskResult, GraphError> {
        let blocked: bool = ctx.get("security_blocked").await.unwrap_or(false);
        if blocked {
            let reason: String = ctx.get("security_reason").await.unwrap_or_default();
            return Ok(GfTaskResult::new(Some(reason), NextAction::End));
        }

        let task: String = ctx.get("task").await.unwrap_or_default();
        let _model: String = ctx.get("model").await.unwrap_or_default();
        let _api_key: String = ctx.get("api_key").await.unwrap_or_default();
        let _base_url: String = ctx.get("base_url").await.unwrap_or_default();

        let tool_list: Vec<String> = self.tool_defs
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect();

        let system_prompt = format!(
            r#"You are a task planner for a coding agent.

Available tools:
{}

Rules:
1. Each task should be atomic (one tool call per task)
2. Tasks should be ordered by dependency
3. Use specific file paths and commands when possible
4. If the task is simple, a single task is fine

Respond with a JSON object:
{{
  "goal": "brief description",
  "tasks": [
    {{
      "id": "task_1",
      "task_type": "tool_name",
      "instruction": "detailed instruction",
      "depends_on": []
    }}
  ]
}}"#,
            tool_list.join("\n")
        );

        let messages = vec![
            Message::System { content: system_prompt },
            Message::User { content: task.clone() },
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
        while let Some(event) = stream.next().await {
            match event {
                Ok(LlmEvent::ContentDelta(t)) => content.push_str(&t),
                Ok(LlmEvent::Done { .. }) => break,
                Ok(LlmEvent::Error(e)) => return Err(GraphError::TaskExecutionFailed(e)),
                _ => {}
            }
        }

        let plan: Plan = serde_json::from_str(&extract_json(&content)).unwrap_or_else(|_| Plan {
            goal: task.clone(),
            tasks: vec![crate::types::Task {
                id: "task_1".into(),
                task_type: "bash".into(),
                instruction: task.clone(),
                depends_on: vec![],
            }],
        });

        ctx.set("plan", plan).await;
        Ok(GfTaskResult::new(Some("Plan created".into()), NextAction::Continue))
    }
}

struct ExecuteTask {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRouter>,
    security: SecurityGuard,
    rate_limiter: LlmRateLimiter,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
}

#[async_trait]
impl gf::Task for ExecuteTask {
    fn id(&self) -> &str { "execute" }

    async fn run(&self, ctx: Context) -> std::result::Result<GfTaskResult, GraphError> {
        let plan: Plan = ctx.get("plan").await.unwrap_or(Plan {
            goal: String::new(),
            tasks: vec![],
        });
        let workspace: String = ctx.get("workspace").await.unwrap_or_default();
        let tool_defs: Vec<ToolDefinition> = ctx.get("tool_defs").await.unwrap_or_default();

        let mut all_output = String::new();

        for task in &plan.tasks {
            let _ = self.event_tx.send(AgentEvent::TaskStarted {
                task_id: task.id.clone(),
                task_type: task.task_type.clone(),
                instruction: task.instruction.clone(),
            });

            if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_tool_execution(&task.task_type).await {
                let _ = self.event_tx.send(AgentEvent::TaskCompleted {
                    task_id: task.id.clone(),
                    success: false,
                    output: format!("Rate limited, retry after {:?}", retry_after),
                });
                continue;
            }

            let output = self.execute_task(task, &tool_defs, &workspace).await;

            let success = !output.starts_with("Error");
            let _ = self.event_tx.send(AgentEvent::TaskCompleted {
                task_id: task.id.clone(),
                success,
                output: output.clone(),
            });

            all_output.push_str(&output);
            all_output.push('\n');
        }

        Ok(GfTaskResult::new(Some(all_output), NextAction::End))
    }
}

impl ExecuteTask {
    async fn execute_task(
        &self,
        task: &crate::types::Task,
        tool_defs: &[ToolDefinition],
        workspace: &str,
    ) -> String {
        let sys = format!(
            "You are an agent. Execute this task using tools.\n\
             Tool: {}\nInstruction: {}\n\
             Output tool calls as: ```tool_call\n{{\"tool\":\"...\",\"args\":{{...}}}}\n```\n\
             After executing, respond with a brief summary.",
            task.task_type, task.instruction,
        );

        let mut messages = vec![
            Message::System { content: sys },
            Message::User { content: task.instruction.clone() },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(4096),
            ..Default::default()
        };

        let mut iteration = 0;
        let max_iterations = 10;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                return "Error: max iterations exceeded".into();
            }

            eprintln!("[EXEC] iter={} msgs={}", iteration, messages.len());

            let mut stream = match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                self.provider.complete_stream(&messages, tool_defs, &options),
            ).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    eprintln!("[EXEC] LLM error: {}", e);
                    return format!("Error: {}", e);
                }
                Err(_) => {
                    eprintln!("[EXEC] LLM timeout");
                    return "Error: LLM request timed out (120s)".into();
                }
            };

            let mut assistant_content = String::new();
            loop {
                let event = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    stream.next(),
                ).await;

                match event {
                    Ok(Some(Ok(LlmEvent::ContentDelta(t)))) => {
                        assistant_content.push_str(&t);
                        let _ = self.event_tx.send(AgentEvent::Streaming { token: t });
                    }
                    Ok(Some(Ok(LlmEvent::Done { .. }))) => break,
                    Ok(Some(Ok(LlmEvent::Error(e)))) => {
                        eprintln!("[EXEC] Stream error: {}", e);
                        return format!("Error: {}", e);
                    }
                    Ok(Some(Ok(_))) => {}
                    Ok(None) => break,
                    Err(_) => {
                        eprintln!("[EXEC] Stream timeout (120s)");
                        break;
                    }
                    Ok(Some(Err(e))) => {
                        eprintln!("[EXEC] Stream err: {}", e);
                        return format!("Error: {}", e);
                    }
                }
            }

            eprintln!("[EXEC] Got {} chars", assistant_content.len());
            let tool_calls = parse_tool_calls_from_text(&assistant_content);
            eprintln!("[EXEC] {} tool calls", tool_calls.len());

            messages.push(Message::Assistant {
                content: Some(assistant_content.clone()),
                tool_calls: tool_calls.clone(),
            });

            if tool_calls.is_empty() {
                return assistant_content;
            }

            let workspace_path = std::path::PathBuf::from(workspace);
            let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<String>(64);

            let forward_tx = self.event_tx.clone();
            let forward_handle = tokio::spawn(async move {
                while let Some(line) = output_rx.recv().await {
                    let _ = forward_tx.send(AgentEvent::ToolOutput {
                        tool_call_id: String::new(),
                        line,
                    });
                }
            });

            for tc in &tool_calls {
                eprintln!("[EXEC] tool={} args={}", tc.name, tc.arguments);
                let _ = self.event_tx.send(AgentEvent::ToolCallStarted {
                    tool_name: tc.name.clone(),
                    args_summary: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                });

                if let Err(e) = self.security.scan_tool_call(&tc.name, &tc.arguments) {
                    let _ = self.event_tx.send(AgentEvent::ToolCallCompleted {
                        tool_name: tc.name.clone(),
                        success: false,
                    });
                    messages.push(Message::Tool {
                        tool_call_id: tc.id.clone(),
                        content: format!("Security blocked: {}", e),
                    });
                    continue;
                }

                let tool_context = ToolContext {
                    workspace: workspace_path.clone(),
                    state: AgentState {
                        session_id: uuid::Uuid::new_v4(),
                        task: task.instruction.clone(),
                        messages: messages.clone(),
                        plan: None,
                        results: vec![],
                        working_memory: WorkingMemory::new(10_000),
                        workspace: workspace_path.clone(),
                        config: AgentConfig::default(),
                        iteration: 0,
                        event_tx: None,
                    },
                    signal: CancellationToken::new(),
                    output_tx: Some(output_tx.clone()),
                };

                let result = self.tools.execute(&tc.name, tc.arguments.clone(), &tool_context).await;

                let tool_result = match result {
                    Ok(mut r) => {
                        r.tool_call_id = tc.id.clone();
                        let _ = self.event_tx.send(AgentEvent::ToolCallCompleted {
                            tool_name: tc.name.clone(),
                            success: !r.is_error,
                        });
                        r
                    }
                    Err(e) => {
                        let _ = self.event_tx.send(AgentEvent::ToolCallCompleted {
                            tool_name: tc.name.clone(),
                            success: false,
                        });
                        ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: format!("Error: {}", e),
                            is_error: true,
                        }
                    }
                };

                messages.push(Message::Tool {
                    tool_call_id: tc.id.clone(),
                    content: tool_result.content,
                });
            }

            drop(output_tx);
            let _ = forward_handle.await;
        }
    }
}

struct SummarizeTask;

#[async_trait]
impl gf::Task for SummarizeTask {
    fn id(&self) -> &str { "summarize" }

    async fn run(&self, ctx: Context) -> std::result::Result<GfTaskResult, GraphError> {
        let plan: Plan = ctx.get("plan").await.unwrap_or(Plan {
            goal: String::new(),
            tasks: vec![],
        });

        let summary = plan.tasks.iter().map(|t| format!("  ✓ {}", t.task_type)).collect::<Vec<_>>().join("\n");
        let final_summary = if summary.is_empty() { "Done.".to_string() } else { summary };

        ctx.set("final_summary", final_summary.clone()).await;
        Ok(GfTaskResult::new(Some(final_summary), NextAction::End))
    }
}

// ── Agent ──

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRouter>,
    #[allow(dead_code)]
    context_builder: ContextBuilder,
    pub(crate) security: SecurityGuard,
    pub(crate) rate_limiter: LlmRateLimiter,
}

impl Agent {
    pub fn new(
        provider: Box<dyn LlmProvider>,
        tools: ToolRouter,
        mut context_builder: ContextBuilder,
    ) -> Self {
        let tool_defs = tools.definitions();
        context_builder = context_builder.with_tools(tool_defs);
        Self {
            provider: Arc::from(provider),
            tools: Arc::new(tools),
            context_builder,
            security: SecurityGuard::new(),
            rate_limiter: LlmRateLimiter::new(),
        }
    }

    pub fn with_security(mut self, security: SecurityGuard) -> Self {
        self.security = security;
        self
    }

    pub fn with_rate_limiter(mut self, rate_limiter: LlmRateLimiter) -> Self {
        self.rate_limiter = rate_limiter;
        self
    }

    pub fn provider(&self) -> Arc<dyn LlmProvider> {
        self.provider.clone()
    }

    pub async fn run(
        &self,
        state: &mut AgentState,
        _cancel: CancellationToken,
        event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<String> {
        let send_event = |event: AgentEvent| {
            if let Some(ref tx) = event_tx {
                let _ = tx.send(event);
            }
        };

        send_event(AgentEvent::Thinking { content: "Creating pipeline...".into() });

        let tool_defs = self.tools.definitions();

        let security_task = SecurityCheckTask {
            guard: self.security.clone(),
        };
        let plan_task = PlanTask {
            provider: self.provider.clone(),
            tool_defs: tool_defs.clone(),
        };
        let exec_task = ExecuteTask {
            provider: self.provider.clone(),
            tools: self.tools.clone(),
            security: self.security.clone(),
            rate_limiter: self.rate_limiter.clone(),
            event_tx: event_tx.clone().unwrap_or_else(|| {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                tx
            }),
        };
        let summarize_task = SummarizeTask;

        let graph = Arc::new(
            GraphBuilder::new("sigmacode")
                .add_task(Arc::new(security_task))
                .add_task(Arc::new(plan_task))
                .add_task(Arc::new(exec_task))
                .add_task(Arc::new(summarize_task))
                .set_start_task("security_check")
                .add_edge("security_check", "plan")
                .add_edge("plan", "execute")
                .add_edge("execute", "summarize")
                .build(),
        );

        let storage = Arc::new(InMemorySessionStorage::new());
        let runner = FlowRunner::new(graph, storage.clone());

        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let session = Session::new_from_task(session_id.clone(), "security_check");
        session.context.set("task", state.task.clone()).await;
        session.context.set("model", state.config.model.clone()).await;
        session.context.set("api_key", state.config.api_key.clone()).await;
        session.context.set("base_url", state.config.base_url.clone()).await;
        session.context.set("workspace", state.workspace.to_string_lossy().to_string()).await;
        session.context.set("tool_defs", tool_defs).await;
        storage.save(session).await.map_err(|e| SigmaError::Other(e.to_string()))?;

        #[allow(unused_assignments)]
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(600),
            async {
                let mut last_result = None;
                loop {
                    match runner.run(&session_id).await {
                        Ok(result) => {
                            match &result.status {
                                ExecutionStatus::Completed | ExecutionStatus::WaitingForInput => {
                                    last_result = Some(result);
                                    break;
                                }
                                ExecutionStatus::Error(_) => {
                                    last_result = Some(result);
                                    break;
                                }
                                ExecutionStatus::Paused { next_task_id: _, .. } => {
                                    last_result = Some(result);
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            return Err(SigmaError::Other(e.to_string()));
                        }
                    }
                }
                Ok(last_result.unwrap_or_else(|| graph_flow::ExecutionResult {
                    response: None,
                    status: ExecutionStatus::Completed,
                }))
            }
        ).await
            .map_err(|_| SigmaError::Llm("Pipeline timed out (300s)".into()))?
            ?;

        match result.status {
            ExecutionStatus::Completed => {
                let summary = result.response.unwrap_or_default();
                send_event(AgentEvent::Done { summary: summary.clone() });
                Ok(summary)
            }
            ExecutionStatus::Error(e) => {
                send_event(AgentEvent::Error { message: e.clone() });
                Err(SigmaError::Llm(e))
            }
            ExecutionStatus::Paused { .. } => {
                let summary = result.response.unwrap_or_default();
                send_event(AgentEvent::Done { summary: summary.clone() });
                Ok(summary)
            }
            ExecutionStatus::WaitingForInput => {
                let summary = result.response.unwrap_or_default();
                send_event(AgentEvent::Done { summary: summary.clone() });
                Ok(summary)
            }
        }
    }

    pub async fn run_task(
        &self,
        state: &mut AgentState,
        cancel: CancellationToken,
        event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<String> {
        let send_event = |event: AgentEvent| {
            if let Some(ref tx) = event_tx {
                let _ = tx.send(event);
            }
        };

        let tool_defs = self.tools.definitions();

        loop {
            if cancel.is_cancelled() {
                return Err(SigmaError::Cancelled);
            }

            state.iteration += 1;
            if state.iteration >= state.config.max_iterations {
                return Err(SigmaError::MaxIterations(state.config.max_iterations));
            }

            let options = CompletionOptions {
                temperature: Some(state.config.temperature),
                max_tokens: Some(state.config.max_tokens),
                ..Default::default()
            };

            let response = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                self.provider.complete_stream(&state.messages, &tool_defs, &options),
            ).await
                .map_err(|_| SigmaError::Llm("LLM request timed out (120s)".into()))?
                ?;

            let mut assistant_content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut current_tool_call: Option<ToolCall> = None;

            let mut stream = response;
            loop {
                let event = tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    stream.next(),
                ).await;

                match event {
                    Ok(Some(event)) => {
                        if cancel.is_cancelled() {
                            return Err(SigmaError::Cancelled);
                        }

                        match event? {
                            LlmEvent::ContentDelta(token) => {
                                assistant_content.push_str(&token);
                                send_event(AgentEvent::Streaming { token });
                            }
                            LlmEvent::ToolUseStart { id: _, name } => {
                                if let Some(tc) = current_tool_call.take() {
                                    tool_calls.push(tc);
                                }
                                current_tool_call = Some(ToolCall {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name,
                                    arguments: serde_json::json!({}),
                                });
                            }
                            LlmEvent::ToolUseDelta {
                                id: _,
                                arguments_delta,
                            } => {
                                if let Some(ref mut tc) = current_tool_call {
                                    let existing = tc.arguments.to_string();
                                    let new_args = format!(
                                        "{}{}",
                                        existing.trim_end_matches('}').trim_start_matches('{'),
                                        arguments_delta
                                    );
                                    if let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(&format!("{{{}}}", new_args))
                                    {
                                        tc.arguments = parsed;
                                    }
                                }
                            }
                            LlmEvent::ToolUseEnd { id: _, name, arguments } => {
                                if let Some(tc) = current_tool_call.take() {
                                    tool_calls.push(ToolCall {
                                        id: tc.id,
                                        name: tc.name,
                                        arguments,
                                    });
                                } else {
                                    tool_calls.push(ToolCall {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        name,
                                        arguments,
                                    });
                                }
                            }
                            LlmEvent::Done { .. } => break,
                            LlmEvent::Error(e) => {
                                send_event(AgentEvent::Error { message: e.clone() });
                                return Err(SigmaError::Llm(e));
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        send_event(AgentEvent::Error {
                            message: "LLM stream timed out (60s)".into(),
                        });
                        break;
                    }
                }
            }

            if let Some(tc) = current_tool_call.take() {
                tool_calls.push(tc);
            }

            if tool_calls.is_empty() && !assistant_content.is_empty() {
                tool_calls = parse_tool_calls_from_text(&assistant_content);
            }

            state.messages.push(Message::Assistant {
                content: if assistant_content.is_empty() {
                    None
                } else {
                    Some(assistant_content.clone())
                },
                tool_calls: tool_calls.clone(),
            });

            if tool_calls.is_empty() {
                return Ok(assistant_content);
            }

            let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<String>(64);
            let tool_context = ToolContext {
                workspace: state.workspace.clone(),
                state: state.clone(),
                signal: cancel.clone(),
                output_tx: Some(output_tx),
            };

            let forward_tx = event_tx.clone();
            let forward_handle = tokio::spawn(async move {
                while let Some(line) = output_rx.recv().await {
                    if let Some(ref tx) = forward_tx {
                        let _ = tx.send(AgentEvent::ToolOutput {
                            tool_call_id: String::new(),
                            line,
                        });
                    }
                }
            });

            for tc in &tool_calls {
                let args_summary = if tc.arguments.is_object() {
                    let args: Vec<String> = tc.arguments
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect();
                    if args.is_empty() { "...".into() } else { args.join(", ") }
                } else {
                    "...".into()
                };
                send_event(AgentEvent::ToolCallStarted {
                    tool_name: tc.name.clone(),
                    args_summary,
                });
            }

            for tc in &tool_calls {
                if cancel.is_cancelled() {
                    return Err(SigmaError::Cancelled);
                }

                if let Err(e) = self.security.scan_tool_call(&tc.name, &tc.arguments) {
                    send_event(AgentEvent::ToolCallCompleted {
                        tool_name: tc.name.clone(),
                        success: false,
                    });
                    state.messages.push(Message::Tool {
                        tool_call_id: tc.id.clone(),
                        content: format!("Security blocked: {}", e),
                    });
                    continue;
                }

                if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_tool_execution(&tc.name).await {
                    send_event(AgentEvent::ToolCallCompleted {
                        tool_name: tc.name.clone(),
                        success: false,
                    });
                    state.messages.push(Message::Tool {
                        tool_call_id: tc.id.clone(),
                        content: format!("Rate limited, retry after {:?}", retry_after),
                    });
                    continue;
                }

                let result = self
                    .tools
                    .execute(&tc.name, tc.arguments.clone(), &tool_context)
                    .await;

                let tool_result = match result {
                    Ok(mut r) => {
                        r.tool_call_id = tc.id.clone();
                        send_event(AgentEvent::ToolCallCompleted {
                            tool_name: tc.name.clone(),
                            success: !r.is_error,
                        });
                        r
                    }
                    Err(e) => {
                        send_event(AgentEvent::ToolCallCompleted {
                            tool_name: tc.name.clone(),
                            success: false,
                        });
                        ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: format!("Error: {}", e),
                            is_error: true,
                        }
                    }
                };

                state.results.push(TaskResult {
                    task_id: tc.id.clone(),
                    task_type: tc.name.clone(),
                    output: tool_result.content.clone(),
                    success: !tool_result.is_error,
                });

                state.messages.push(Message::Tool {
                    tool_call_id: tc.id.clone(),
                    content: tool_result.content,
                });
            }

            let _ = forward_handle.await;
        }
    }
}

fn parse_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    let mut tool_calls = Vec::new();

    if let Some(start) = text.find("```tool_call") {
        let content_start = start + 12;
        if let Some(end) = text[content_start..].find("```") {
            let json_str = text[content_start..content_start + end].trim();
            if let Some(tc) = parse_tool_json(json_str) {
                tool_calls.push(tc);
            }
        }
    }

    if tool_calls.is_empty() {
        if let Some(start) = text.find("```json") {
            let content_start = start + 7;
            if let Some(end) = text[content_start..].find("```") {
                let json_str = text[content_start..content_start + end].trim();
                if let Some(tc) = parse_tool_json(json_str) {
                    tool_calls.push(tc);
                }
            }
        }
    }

    if tool_calls.is_empty() {
        let trimmed = text.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Some(tc) = parse_tool_json(trimmed) {
                tool_calls.push(tc);
            }
        }
    }

    tool_calls
}

fn parse_tool_json(json_str: &str) -> Option<ToolCall> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(tool_name) = parsed["tool"].as_str() {
            let args = parsed.get("args").cloned().unwrap_or_default();
            return Some(ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: tool_name.to_string(),
                arguments: args,
            });
        }
    }
    None
}

fn extract_json(text: &str) -> String {
    if let Some(start) = text.find("```json") {
        let json_start = start + 7;
        if let Some(end) = text[json_start..].find("```") {
            return text[json_start..json_start + end].trim().to_string();
        }
    }
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_call_from_text() {
        let text = r#"I'll list the files for you.

```tool_call
{"tool": "bash", "args": {"command": "ls -la"}}
```"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn test_parse_json_fallback() {
        let text = r#"Here's the tool call:

```json
{"tool": "read_file", "args": {"path": "src/main.rs"}}
```"#;
        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "src/main.rs");
    }

    #[test]
    fn test_parse_no_tool_call() {
        let text = "This is just regular assistant text with no tool calls.";
        let calls = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

}
