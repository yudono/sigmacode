use crate::llm::LlmProvider;
use crate::types::{AgentState, CompletionOptions, Message, TaskAnalysis};

pub struct Analyzer {
    provider: Box<dyn LlmProvider>,
}

impl Analyzer {
    pub fn new(provider: Box<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn analyze(
        &self,
        request: &str,
        _state: &AgentState,
    ) -> crate::error::Result<TaskAnalysis> {
        let sys = r#"You are a task analyzer. Given a user request, extract:
1. Intent: What the user wants to accomplish (one sentence)
2. Goals: Specific, measurable outcomes (list)
3. Constraints: Technical or business constraints (list)
4. Success Criteria: How to verify the task is complete (list)

Respond ONLY with valid JSON:
{
  "intent": "...",
  "goals": ["..."],
  "constraints": ["..."],
  "success_criteria": ["..."]
}

Be specific. For coding tasks, success criteria should include build/test verification."#;

        let messages = vec![
            Message::System { content: sys.into() },
            Message::User { content: request.into() },
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
                    "intent": request,
                    "goals": [request],
                    "constraints": [],
                    "success_criteria": ["Task completes without errors"]
                })
            });

        Ok(TaskAnalysis {
            intent: parsed["intent"].as_str().unwrap_or(request).to_string(),
            goals: parsed["goals"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_else(|| vec![request.to_string()]),
            constraints: parsed["constraints"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
            success_criteria: parsed["success_criteria"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_else(|| vec!["Task completes without errors".into()]),
        })
    }
}
