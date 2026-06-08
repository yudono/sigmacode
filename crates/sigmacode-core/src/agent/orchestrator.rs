use std::path::PathBuf;
use std::sync::Arc;
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::agent::analyzer::Analyzer;
use crate::agent::critic::Critic;
use crate::agent::memory::MemoryManager;
use crate::agent::reviewer::Reviewer;
use crate::agent::verifier::Verifier;
use crate::llm::LlmProvider;
use crate::tools::{ToolContext, ToolRouter};
use crate::types::{AgentEvent, AgentState, CompletionOptions, Message};

const MAX_REPLAN_ATTEMPTS: u32 = 3;

#[allow(dead_code)]
pub struct Orchestrator {
    analyzer: Analyzer,
    verifier: Verifier,
    critic: Critic,
    reviewer: Reviewer,
    memory: MemoryManager,
    tools: Arc<ToolRouter>,
    provider: Arc<dyn LlmProvider>,
}

impl Orchestrator {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRouter>,
        workspace: PathBuf,
    ) -> Self {
        let analyzer = Analyzer::new(provider.clone());
        let verifier = Verifier::new(workspace.clone());
        let critic = Critic::new(provider.clone());
        let reviewer = Reviewer::new(provider.clone());
        let memory = MemoryManager::new(workspace);

        Self {
            analyzer,
            verifier,
            critic,
            reviewer,
            memory,
            tools,
            provider,
        }
    }

    pub async fn run(
        &mut self,
        request: &str,
        state: &mut AgentState,
        cancel: tokio_util::sync::CancellationToken,
        event_tx: &Option<mpsc::UnboundedSender<AgentEvent>>,
    ) -> crate::error::Result<String> {
        // ── Phase 1: Analyze ──
        let analysis = self.analyzer.analyze(request, state).await?;
        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::AnalysisComplete {
                goals: analysis.goals.clone(),
                constraints: analysis.constraints.clone(),
                success_criteria: analysis.success_criteria.clone(),
            });
        }
        self.memory.record_action(format!("Analyzed task: {}", analysis.intent));

        // ── Phase 2: Plan (via LLM) ──
        let plan_text = self.create_plan(request, &analysis).await?;
        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::Thinking {
                content: format!("Plan:\n{}", plan_text),
            });
        }
        self.memory.record_action("Created execution plan".into());

        // ── Phase 3: Execute with replan loop ──
        let mut replan_attempt = 0u32;
        let mut last_error = String::new();
        let _ = &last_error;

        loop {
            if cancel.is_cancelled() {
                return Ok("Task cancelled".into());
            }

            // Execute the plan via ReAct loop
            let exec_result = self.execute_plan(request, state, &cancel, &event_tx).await;

            match exec_result {
                Ok(output) => {
                    // ── Phase 4: Verify ──
                    let verification = self.verifier.verify_all(&event_tx).await;

                    if verification.passed {
                        // ── Phase 5: Review ──
                        let modified_files = self.get_modified_files(&output);
                        let file_contents = self.read_files(&modified_files);
                        let review = self.reviewer.review(
                            &modified_files,
                            &file_contents,
                            &event_tx,
                        ).await?;

                        self.memory.record_action(format!(
                            "Completed with review score: {}/100, {} issues",
                            review.score,
                            review.issues.len()
                        ));

                        // ── Phase 6: Finalize ──
                        let summary = self.finalize(
                            request,
                            &output,
                            &verification,
                            &review,
                        );

                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::Finalizing);
                        }

                        return Ok(summary);
                    } else {
                        // Verification failed — critic analysis
                        last_error = verification.errors.join("; ");

                        if replan_attempt >= MAX_REPLAN_ATTEMPTS {
                            return Ok(format!(
                                "Task completed but verification failed after {} attempts.\n\
                                 Last error: {}\n\
                                 Output: {}",
                                MAX_REPLAN_ATTEMPTS,
                                last_error,
                                output
                            ));
                        }

                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::Replanning {
                                reason: last_error.clone(),
                                attempt: replan_attempt + 1,
                            });
                        }

                        self.memory.record_error(last_error.clone());
                        replan_attempt += 1;
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    self.memory.record_error(last_error.clone());

                    if replan_attempt >= MAX_REPLAN_ATTEMPTS {
                        return Ok(format!(
                            "Task failed after {} attempts.\nLast error: {}",
                            MAX_REPLAN_ATTEMPTS,
                            last_error
                        ));
                    }

                    if let Some(tx) = event_tx {
                        let _ = tx.send(AgentEvent::Replanning {
                            reason: last_error.clone(),
                            attempt: replan_attempt + 1,
                        });
                    }

                    replan_attempt += 1;
                }
            }
        }
    }

    async fn create_plan(
        &self,
        request: &str,
        analysis: &crate::types::TaskAnalysis,
    ) -> crate::error::Result<String> {
        let memory_context = self.memory.get_context_for_planning();

        let sys = format!(
            r#"You are a task planner. Create a detailed execution plan for the user's request.

Goals: {}
Constraints: {}
Success Criteria: {}

{}
Break the task into concrete steps. Each step should be specific and actionable.
Use tools like bash, read_file, write_file, edit_file as needed.
Always include verification steps (build, test, lint) after code changes.

Respond with a numbered list of steps."#,
            analysis.goals.join(", "),
            analysis.constraints.join(", "),
            analysis.success_criteria.join(", "),
            if memory_context.is_empty() { String::new() } else { format!("Context from previous actions:\n{}", memory_context) }
        );

        let messages = vec![
            Message::System { content: sys },
            Message::User { content: request.into() },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(2048),
            ..Default::default()
        };

        let response = self.provider.complete(&messages, &[], &options).await?;
        Ok(response.content.unwrap_or_else(|| "No plan generated".into()))
    }

    async fn execute_plan(
        &mut self,
        request: &str,
        state: &mut AgentState,
        cancel: &tokio_util::sync::CancellationToken,
        event_tx: &Option<mpsc::UnboundedSender<AgentEvent>>,
    ) -> crate::error::Result<String> {
        let tool_defs: Vec<_> = self.tools.definitions();
        let mut messages = vec![
            Message::System {
                content: self.build_system_prompt(state),
            },
            Message::User {
                content: format!(
                    "Task: {}\n\nExecute this task step by step. Use tools as needed. \
                     After each code change, verify with build/test commands. \
                     Be thorough and methodical.",
                    request
                ),
            },
        ];

        let options = CompletionOptions {
            temperature: Some(state.config.temperature),
            max_tokens: Some(state.config.max_tokens),
            ..Default::default()
        };

        let mut all_output = String::new();
        let mut iteration = 0;
        let max_iterations = state.config.max_iterations;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                return Ok(format!("Reached max iterations ({}). Partial result:\n{}", max_iterations, all_output));
            }

            if cancel.is_cancelled() {
                return Ok(all_output);
            }

            let mut stream = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                self.provider.complete_stream(&messages, &tool_defs, &options),
            ).await
            .map_err(|_| crate::error::SigmaError::Llm("Stream timeout".into()))?
            .map_err(|e| crate::error::SigmaError::Llm(e.to_string()))?;

            let mut assistant_content = String::new();

            loop {
                let event: Result<Option<Result<crate::types::LlmEvent, crate::error::SigmaError>>, _> = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    stream.next(),
                ).await;

                match event {
                    Ok(Some(Ok(crate::types::LlmEvent::ContentDelta(t)))) => {
                        assistant_content.push_str(&t);
                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::Streaming { token: t });
                        }
                    }
                    Ok(Some(Ok(crate::types::LlmEvent::Done { .. }))) => break,
                    Ok(Some(Ok(_))) => continue,
                    Ok(Some(Err(e))) => return Err(crate::error::SigmaError::Llm(e.to_string())),
                    Ok(None) => break,
                    Err(_) => return Err(crate::error::SigmaError::Llm("Stream timeout".into())),
                }
            }

            // Parse tool calls from the response
            let tool_calls = crate::agent::engine::parse_tool_calls_from_text(&assistant_content);

            if tool_calls.is_empty() {
                // No more tool calls — final response
                all_output.push_str(&assistant_content);
                break;
            }

            // Execute tool calls
            messages.push(Message::Assistant {
                content: Some(assistant_content.clone()),
                tool_calls: tool_calls.clone(),
            });

            for tc in &tool_calls {
                if let Some(tx) = event_tx {
                    let _ = tx.send(AgentEvent::ToolCallStarted {
                        tool_name: tc.name.clone(),
                        args_summary: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    });
                }

                self.memory.record_action(format!("Executing {} tool", tc.name));

                let tool_context = ToolContext {
                    workspace: state.workspace.clone(),
                    state: AgentState {
                        session_id: state.session_id,
                        task: state.task.clone(),
                        messages: state.messages.clone(),
                        plan: None,
                        results: Vec::new(),
                        working_memory: crate::types::WorkingMemory::new(state.config.context_window),
                        workspace: state.workspace.clone(),
                        config: state.config.clone(),
                        iteration,
                        event_tx: event_tx.clone(),
                    },
                    signal: cancel.clone(),
                    output_tx: None,
                };

                let result = self.tools.execute(&tc.name, tc.arguments.clone(), &tool_context).await
                    .unwrap_or_else(|e| crate::types::ToolResult {
                        tool_call_id: tc.id.clone(),
                        content: format!("Error: {}", e),
                        is_error: true,
                    });

                if let Some(tx) = event_tx {
                    let _ = tx.send(AgentEvent::ToolCallCompleted {
                        tool_name: tc.name.clone(),
                        success: !result.is_error,
                    });
                }

                // Record file modifications
                if tc.name == "write_file" || tc.name == "edit_file" {
                    if let Some(path) = tc.arguments["path"].as_str() {
                        self.memory.record_file_modified(path.to_string());
                    }
                }

                messages.push(Message::Tool {
                    tool_call_id: tc.id.clone(),
                    content: result.content.clone(),
                });

                all_output.push_str(&format!("\n[Tool: {}] {}\n", tc.name, if result.is_error { "FAILED" } else { "OK" }));
            }
        }

        Ok(all_output)
    }

    fn build_system_prompt(&self, state: &AgentState) -> String {
        let tool_list: Vec<String> = self.tools.definitions()
            .iter()
            .map(|t| {
                let params = serde_json::to_string_pretty(&t.parameters).unwrap_or_default();
                format!("- {}: {}\n  Parameters: {}", t.name, t.description, params)
            })
            .collect();

        let memory_context = self.memory.get_context_for_planning();

        format!(
            r#"You are SigmaCode, an expert AI coding assistant.

## Project: {}

## Available Tools

{}

To use a tool, output a JSON block like this:
```tool_call
{{"tool": "tool_name", "args": {{"param": "value"}}}}
```

## Rules:
1. Always read files before editing them
2. Make minimal, targeted changes
3. After code changes, ALWAYS verify with build/test commands
4. If a tool call fails, analyze the error and try a different approach
5. Use edit_file for precise changes, write_file for new files
6. Be concise — focus on the task
7. Never expose secrets or API keys

## Working Directory: {}

{}"#,
            state.workspace.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into()),
            tool_list.join("\n\n"),
            state.workspace.display(),
            if memory_context.is_empty() { String::new() } else { format!("\n## Session Memory\n{}", memory_context) }
        )
    }

    fn get_modified_files(&self, _output: &str) -> Vec<String> {
        self.memory.get_session_memory().files_modified.clone()
    }

    fn read_files(&self, files: &[String]) -> Vec<(String, String)> {
        files.iter()
            .filter_map(|path| {
                let content = std::fs::read_to_string(path).ok()?;
                Some((path.clone(), content))
            })
            .collect()
    }

    fn finalize(
        &self,
        request: &str,
        _output: &str,
        verification: &crate::types::VerificationResult,
        review: &crate::types::ReviewResult,
    ) -> String {
        let memory = self.memory.get_session_memory();

        let mut summary = format!("## Task Complete\n\n{}\n\n", request);

        summary.push_str("### Modified Files\n");
        if memory.files_modified.is_empty() {
            summary.push_str("- None\n");
        } else {
            for file in &memory.files_modified {
                summary.push_str(&format!("- {}\n", file));
            }
        }

        summary.push_str(&format!("\n### Verification: {}\n", if verification.passed { "PASSED" } else { "FAILED" }));
        if !verification.passed {
            summary.push_str(&format!("- Errors: {}\n", verification.errors.join(", ")));
        }

        summary.push_str(&format!("\n### Code Review: {}/100\n", review.score));
        if !review.issues.is_empty() {
            for issue in &review.issues {
                summary.push_str(&format!(
                    "- [{:?}] {}: {}\n",
                    issue.severity,
                    issue.category,
                    issue.message
                ));
            }
        }

        if !memory.errors_encountered.is_empty() {
            summary.push_str("\n### Errors During Execution\n");
            for error in &memory.errors_encountered {
                summary.push_str(&format!("- {}\n", error));
            }
        }

        summary
    }
}
