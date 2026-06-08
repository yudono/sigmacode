use std::sync::Arc;
use crate::llm::LlmProvider;
use crate::types::{AgentEvent, CompletionOptions, CriticResult, Message, VerificationResult};

pub struct Critic {
    provider: Arc<dyn LlmProvider>,
}

impl Critic {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn analyze(
        &self,
        verification: &VerificationResult,
        original_task: &str,
        event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    ) -> crate::error::Result<CriticResult> {
        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::Criticking {
                errors: verification.errors.clone(),
            });
        }

        let sys = r#"You are a coding critic. When a build/test/lint step fails, analyze the error and provide:

1. Root Cause: What exactly went wrong (be specific, reference file names and line numbers)
2. Error Classification: Type of error (syntax, type, import, dependency, configuration, runtime, logic, test_failure)
3. Fix Recommendation: Specific steps to fix the issue (include code changes if possible)

Respond ONLY with valid JSON:
{
  "root_cause": "...",
  "error_class": "...",
  "fix_recommendation": "..."
}

Be precise. Reference specific files and line numbers from the error output."#;

        let user_msg = format!(
            "Original task: {}\n\nVerification step: {}\nPassed: {}\nErrors:\n{}\n\nOutput:\n{}",
            original_task,
            verification.step,
            verification.passed,
            verification.errors.join("\n"),
            verification.output.lines().take(50).collect::<Vec<_>>().join("\n")
        );

        let messages = vec![
            Message::System { content: sys.into() },
            Message::User { content: user_msg },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(2048),
            ..Default::default()
        };

        let response = self.provider.complete(&messages, &[], &options).await?;
        let content = response.content.unwrap_or_default();

        let parsed: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| {
                serde_json::json!({
                    "root_cause": verification.errors.first().unwrap_or(&"Unknown error".into()).clone(),
                    "error_class": "unknown",
                    "fix_recommendation": "Review the error output and fix the issue"
                })
            });

        let result = CriticResult {
            root_cause: parsed["root_cause"].as_str().unwrap_or("Unknown").to_string(),
            error_class: parsed["error_class"].as_str().unwrap_or("unknown").to_string(),
            fix_recommendation: parsed["fix_recommendation"].as_str().unwrap_or("Review error").to_string(),
            affected_steps: Vec::new(),
        };

        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::CriticResult {
                root_cause: result.root_cause.clone(),
                fix: result.fix_recommendation.clone(),
            });
        }

        Ok(result)
    }
}
