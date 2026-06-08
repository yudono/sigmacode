use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

        // ── Step 2: Add user message ──
        state.messages.push(Message::User {
            content: state.task.clone(),
        });

        // ── Step 3: ReAct Loop ──
        loop {
            if cancel.is_cancelled() {
                return Err(SigmaError::Cancelled);
            }

            if state.iteration >= state.config.max_iterations {
                return Err(SigmaError::MaxIterations(state.config.max_iterations));
            }

            state.iteration += 1;

            // Build context and get tool definitions
            let tool_defs = self.tools.definitions();
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
            while let Some(event) = stream.next().await {
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
                            // Accumulate arguments
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
                    Some(assistant_content)
                },
                tool_calls: tool_calls.clone(),
            });

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                let summary = state
                    .results
                    .last()
                    .map(|r| r.output.clone())
                    .unwrap_or_else(|| "Task completed.".into());

                send_event(AgentEvent::Done {
                    summary: summary.clone(),
                });
                return Ok(summary);
            }

            // ── Execute Tools ──
            let tool_context = ToolContext {
                workspace: state.workspace.clone(),
                state: state.clone(),
                signal: cancel.clone(),
            };

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

            // Auto-compact if context is getting too large
            if state.config.auto_compact {
                let total_tokens: usize = state.messages.iter().map(|m| m.token_estimate()).sum();
                if total_tokens > state.config.context_window * 80 / 100 {
                    self.auto_compact(state).await?;
                }
            }
        }
    }

    async fn auto_compact(&self, state: &mut AgentState) -> Result<()> {
        // Keep system message and last N exchanges
        let system_msg = state.messages.first().cloned();
        let recent_count = 6;
        let recent: Vec<Message> = state
            .messages
            .iter()
            .rev()
            .take(recent_count)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        state.messages.clear();
        if let Some(sys) = system_msg {
            state.messages.push(sys);
        }
        state.messages.push(Message::User {
            content: format!(
                "[Context compacted. Previous {} messages summarized. Working memory: {}]",
                state.messages.len(),
                state.working_memory.render()
            ),
        });
        state.messages.extend(recent);

        Ok(())
    }
}

fn parse_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    let mut tool_calls = Vec::new();

    // Look for ```tool_call ... ``` blocks
    if let Some(start) = text.find("```tool_call") {
        let content_start = start + 11; // len of "```tool_call"
        if let Some(end) = text[content_start..].find("```") {
            let json_str = text[content_start..content_start + end].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(tool_name) = parsed["tool"].as_str() {
                    let args = parsed.get("args").cloned().unwrap_or_default();
                    tool_calls.push(ToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: tool_name.to_string(),
                        arguments: args,
                    });
                }
            }
        }
    }

    // Also look for JSON blocks with tool_call format
    if tool_calls.is_empty() {
        if let Some(start) = text.find("```json") {
            let content_start = start + 7;
            if let Some(end) = text[content_start..].find("```") {
                let json_str = text[content_start..content_start + end].trim();
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(tool_name) = parsed["tool"].as_str() {
                        let args = parsed.get("args").cloned().unwrap_or_default();
                        tool_calls.push(ToolCall {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: tool_name.to_string(),
                            arguments: args,
                        });
                    }
                }
            }
        }
    }

    tool_calls
}
