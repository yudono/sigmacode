use crate::error::Result;
use crate::llm::LlmProvider;
use crate::types::*;
use std::sync::Arc;

pub struct Planner {
    provider: Arc<dyn LlmProvider>,
}

impl Planner {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn create_plan(
        &self,
        task: &str,
        tool_definitions: &[ToolDefinition],
    ) -> Result<Plan> {
        let tool_list: Vec<String> = tool_definitions
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect();

        let system_prompt = format!(
            r#"You are a task planner for a coding agent.

Available tools:
{}

Your job is to break down the user's goal into a sequence of concrete, executable tasks.

Rules:
1. Each task should be atomic (one tool call per task)
2. Tasks should be ordered by dependency
3. Use specific file paths and commands when possible
4. Focus on coding tasks: reading files, editing code, running commands, verifying builds
5. If the task is simple (e.g., "read a file"), a single task is fine
6. If the task is complex, break it into 3-10 steps

Respond with a JSON object:
{{
  "goal": "brief description of the overall goal",
  "tasks": [
    {{
      "id": "task_1",
      "task_type": "tool_name",
      "instruction": "detailed instruction for this task",
      "depends_on": []
    }}
  ]
}}

The task_type must be one of the available tool names."#,
            tool_list.join("\n")
        );

        let messages = vec![
            Message::System {
                content: system_prompt,
            },
            Message::User {
                content: task.to_string(),
            },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(2048),
            tool_choice: Some("none".into()),
        };

        let response = self
            .provider
            .complete(&messages, &[], &options)
            .await?;

        let content = response.content.unwrap_or_default();

        let json_str = extract_json(&content);

        let plan: Plan = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::SigmaError::Llm(format!("Failed to parse plan JSON: {}", e))
        })?;

        Ok(plan)
    }

    pub async fn replan_after_failure(
        &self,
        original_plan: &Plan,
        failed_task_id: &str,
        error: &str,
        completed_tasks: &[TaskResult],
    ) -> Result<Plan> {
        let completed_summary: Vec<String> = completed_tasks
            .iter()
            .map(|r| format!("- {} ({})", r.task_type, if r.success { "success" } else { "failed" }))
            .collect();

        let system_prompt = r#"You are a task planner for a coding agent.
A previous plan failed. Analyze the error and create a revised plan.

Respond with a JSON object containing the revised plan."#;

        let user_msg = format!(
            r#"Original goal: {}

Failed task: {} ({})
Error: {}

Completed tasks:
{}

Create a revised plan that handles the failure gracefully."#,
            original_plan.goal,
            failed_task_id,
            original_plan
                .tasks
                .iter()
                .find(|t| t.id == failed_task_id)
                .map(|t| t.instruction.as_str())
                .unwrap_or("unknown"),
            error,
            completed_summary.join("\n")
        );

        let messages = vec![
            Message::System {
                content: system_prompt.into(),
            },
            Message::User { content: user_msg },
        ];

        let options = CompletionOptions {
            temperature: Some(0.0),
            max_tokens: Some(2048),
            ..Default::default()
        };

        let response = self
            .provider
            .complete(&messages, &[], &options)
            .await?;

        let content = response.content.unwrap_or_default();
        let json_str = extract_json(&content);

        let plan: Plan = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::SigmaError::Llm(format!("Failed to parse replan JSON: {}", e))
        })?;

        Ok(plan)
    }
}

pub fn extract_json(text: &str) -> String {
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
    fn test_extract_json_from_markdown() {
        let text = r#"Here is the plan:

```json
{
  "goal": "Test",
  "tasks": []
}
```

That's the plan."#;

        let json = extract_json(text);
        assert!(json.contains("Test"));
        assert!(json.starts_with('{'));
    }

    #[test]
    fn test_extract_json_raw() {
        let text = r#"{"goal": "Add OAuth", "tasks": [{"id": "t1"}]}"#;
        let json = extract_json(text);
        assert_eq!(json, text);
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let text = r#"I'll create a plan for you. {"goal": "Test", "tasks": []} Here it is."#;
        let json = extract_json(text);
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_extract_json_empty_object() {
        let text = r#"```json
{}
```"#;
        let json = extract_json(text);
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_extract_json_nested() {
        let text = r#"{"goal": "test", "tasks": [{"id": "1", "nested": {"a": 1}}]}"#;
        let json = extract_json(text);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tasks"][0]["nested"]["a"], 1);
    }
}
