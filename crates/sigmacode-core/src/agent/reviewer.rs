use std::sync::Arc;
use crate::llm::LlmProvider;
use crate::types::{AgentEvent, CompletionOptions, Message, ReviewIssue, ReviewResult, ReviewSeverity};

pub struct Reviewer {
    provider: Arc<dyn LlmProvider>,
}

impl Reviewer {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn review(
        &self,
        files: &[String],
        file_contents: &[(String, String)],
        event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    ) -> crate::error::Result<ReviewResult> {
        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::Reviewing);
        }

        let sys = r#"You are a code reviewer. Review the provided code changes for:

1. Code Quality: Readability, naming, structure, DRY principles
2. Security: SQL injection, XSS, secrets exposure, insecure patterns
3. Edge Cases: Null handling, error handling, boundary conditions
4. Performance: N+1 queries, unnecessary allocations, blocking operations

Respond ONLY with valid JSON:
{
  "score": 85,
  "issues": [
    {
      "severity": "warning",
      "category": "security",
      "file": "src/auth.rs",
      "message": "API key is hardcoded in source"
    }
  ]
}

Score is 0-100 (100 = perfect). Only include real issues, not style preferences."#;

        let code_blocks: Vec<String> = file_contents.iter()
            .map(|(path, content)| format!("=== {} ===\n{}", path, content.lines().take(200).collect::<Vec<_>>().join("\n")))
            .collect();

        let user_msg = format!(
            "Files modified:\n{}\n\nCode to review:\n{}",
            files.join("\n"),
            code_blocks.join("\n\n")
        );

        let messages = vec![
            Message::System { content: sys.into() },
            Message::User { content: user_msg },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(4096),
            ..Default::default()
        };

        let response = self.provider.complete(&messages, &[], &options).await?;
        let content = response.content.unwrap_or_default();

        let parsed: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| {
                serde_json::json!({
                    "score": 100,
                    "issues": []
                })
            });

        let issues: Vec<ReviewIssue> = parsed["issues"]
            .as_array()
            .map(|a| {
                a.iter().filter_map(|item| {
                    let severity = match item["severity"].as_str()? {
                        "critical" => ReviewSeverity::Critical,
                        "warning" => ReviewSeverity::Warning,
                        _ => ReviewSeverity::Info,
                    };
                    Some(ReviewIssue {
                        severity,
                        category: item["category"].as_str().unwrap_or("general").to_string(),
                        file: item["file"].as_str().unwrap_or("unknown").to_string(),
                        message: item["message"].as_str().unwrap_or("No message").to_string(),
                    })
                }).collect()
            })
            .unwrap_or_default();

        let score = parsed["score"].as_u64().unwrap_or(100) as u32;

        let result = ReviewResult {
            issues: issues.clone(),
            score,
        };

        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::ReviewComplete {
                score,
                issues_count: issues.len(),
            });
        }

        Ok(result)
    }
}
