use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::planner::Planner;
use crate::context::ContextBuilder;
use crate::error::{Result, SigmaError};
use crate::llm::LlmProvider;
use crate::tools::{ToolContext, ToolRouter};
use crate::types::*;

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRouter,
    context_builder: ContextBuilder,
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
            tools,
            context_builder,
        }
    }

    pub async fn run(
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

        // ── Step 1: Build system prompt ──
        let system_prompt = self.context_builder.build_system_prompt(state);
        state.messages.push(Message::System {
            content: system_prompt,
        });

        // ── Step 2: Create plan using LLM ──
        send_event(AgentEvent::Thinking {
            content: "Creating plan...".into(),
        });

        let planner = Planner::new(self.provider.clone());
        let tool_defs = self.tools.definitions();

        let plan = match planner.create_plan(&state.task, &tool_defs).await {
            Ok(plan) => {
                send_event(AgentEvent::PlanCreated {
                    tasks: plan.tasks.clone(),
                });
                plan
            }
            Err(_) => {
                // If planning fails, create a simple single-task plan locally (no extra API call)
                let task_type = detect_tool_type(&state.task);
                Plan {
                    goal: state.task.clone(),
                    tasks: vec![Task {
                        id: "task_1".into(),
                        task_type: task_type.clone(),
                        instruction: state.task.clone(),
                        depends_on: vec![],
                    }],
                }
            }
        };

        state.plan = Some(plan.clone());

        // ── Step 3: Execute each task ──
        for task in plan.tasks.iter() {
            if cancel.is_cancelled() {
                return Err(SigmaError::Cancelled);
            }

            send_event(AgentEvent::TaskStarted {
                task_id: task.id.clone(),
                task_type: task.task_type.clone(),
                instruction: task.instruction.clone(),
            });

            // Build prompt for this specific task
            let task_prompt = format!(
                r#"Execute this task:
Tool: {}
Instruction: {}
Previous results: {}

Output a tool call like this:
```tool_call
{{"tool": "{}", "args": {{...}}}}
```

Or if you need to read a file first:
```tool_call
{{"tool": "read_file", "args": {{"path": "file.rs"}}}}
```

After executing, respond with a brief summary of what was done."#,
                task.task_type,
                task.instruction,
                state.results.iter().map(|r| format!("- {}: {}", r.task_type, if r.success { "ok" } else { "failed" })).collect::<Vec<_>>().join("\n"),
                task.task_type,
            );

            state.messages.push(Message::User {
                content: task_prompt,
            });

            // ── ReAct Loop for this task ──
            let mut sub_iteration = 0;

            while sub_iteration < 10 {
                if cancel.is_cancelled() {
                    return Err(SigmaError::Cancelled);
                }

                sub_iteration += 1;
                state.iteration += 1;

                if state.iteration >= state.config.max_iterations {
                    return Err(SigmaError::MaxIterations(state.config.max_iterations));
                }

                let options = CompletionOptions {
                    temperature: Some(state.config.temperature),
                    max_tokens: Some(state.config.max_tokens),
                    ..Default::default()
                };

                // ── LLM Reasoning ──
                let response = self
                    .provider
                    .complete_stream(&state.messages, &tool_defs, &options)
                    .await?;

                // Process streaming response
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
                            send_event(AgentEvent::ToolCallStarted {
                                tool_name: current_tool_call.as_ref().unwrap().name.clone(),
                                args_summary: "...".into(),
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
                            send_event(AgentEvent::Error {
                                message: e.clone(),
                            });
                            return Err(SigmaError::Llm(e));
                        }
                    }
                        }
                        Ok(None) => break, // stream ended
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

                // Parse tool calls from text if no native tool calls
                if tool_calls.is_empty() && !assistant_content.is_empty() {
                    tool_calls = parse_tool_calls_from_text(&assistant_content);
                }

                // Add assistant message
                state.messages.push(Message::Assistant {
                    content: if assistant_content.is_empty() {
                        None
                    } else {
                        Some(assistant_content.clone())
                    },
                    tool_calls: tool_calls.clone(),
                });

                // If no tool calls, task is done
                if tool_calls.is_empty() {
                    send_event(AgentEvent::TaskCompleted {
                        task_id: task.id.clone(),
                        success: true,
                        output: assistant_content,
                    });
                    break;
                }

                // ── Execute Tools ──
                let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<String>(64);
                let tool_context = ToolContext {
                    workspace: state.workspace.clone(),
                    state: state.clone(),
                    signal: cancel.clone(),
                    output_tx: Some(output_tx),
                };

                // Forward tool output lines as events
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

                // Send ToolCallStarted for each text-parsed tool call
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

                // Wait for output forwarding to complete
                let _ = forward_handle.await;
            }
        }

        // ── Step 4: Final summary ──
        let summary = state
            .results
            .iter()
            .map(|r| {
                format!(
                    "  {} {}",
                    if r.success { "✓" } else { "✗" },
                    r.task_type,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let final_summary = if summary.is_empty() {
            "Done.".to_string()
        } else {
            summary
        };

        send_event(AgentEvent::Done {
            summary: final_summary.clone(),
        });

        Ok(final_summary)
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

    // Also detect raw JSON tool calls without markdown wrappers
    // e.g. {"tool": "write_file", "args": {"path": "...", ...}}
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

fn detect_tool_type(task: &str) -> String {
    let lower = task.to_lowercase();
    if lower.contains("create") || lower.contains("write") || lower.contains("add") {
        "write_file".to_string()
    } else if lower.contains("read") || lower.contains("show") || lower.contains("cat") {
        "read_file".to_string()
    } else if lower.contains("edit") || lower.contains("change") || lower.contains("modify") || lower.contains("fix") {
        "edit_file".to_string()
    } else if lower.contains("search") || lower.contains("find") || lower.contains("grep") {
        "grep".to_string()
    } else if lower.contains("list") || lower.contains("glob") || lower.contains("files") {
        "glob".to_string()
    } else {
        "bash".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_call_from_text() {
        let text = r#"I'll list the files for you.

```tool_call
{"tool": "bash", "args": {"command": "ls -la"}}
```
"#;
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
```
"#;
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

    #[test]
    fn test_detect_tool_type() {
        assert_eq!(detect_tool_type("create a new file"), "write_file");
        assert_eq!(detect_tool_type("read the config"), "read_file");
        assert_eq!(detect_tool_type("edit line 42"), "edit_file");
        assert_eq!(detect_tool_type("search for pattern"), "grep");
        assert_eq!(detect_tool_type("list all files"), "glob");
        assert_eq!(detect_tool_type("run cargo build"), "bash");
    }
}
