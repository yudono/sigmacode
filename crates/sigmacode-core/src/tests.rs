#[cfg(test)]
mod types_tests {
    use crate::types::*;

    #[test]
    fn test_message_token_estimate() {
        let msg = Message::User {
            content: "Hello, world!".into(),
        };
        assert_eq!(msg.token_estimate(), 3);

        let msg = Message::System {
            content: "a".repeat(100),
        };
        assert_eq!(msg.token_estimate(), 25);
    }

    #[test]
    fn test_working_memory_append() {
        let mut memory = WorkingMemory::new(100);
        memory.append("task_1", "Result of first task");
        assert_eq!(memory.entries.len(), 1);
        assert_eq!(memory.entries[0].label, "task_1");
    }

    #[test]
    fn test_working_memory_trim_to_budget() {
        let mut memory = WorkingMemory::new(20);
        memory.append("t1", "a".repeat(100));
        memory.append("t2", "b".repeat(100));
        memory.append("t3", "c".repeat(100));
        assert!(memory.entries.len() < 3);
    }

    #[test]
    fn test_working_memory_render() {
        let mut memory = WorkingMemory::new(1000);
        memory.append("context", "Next.js app");
        memory.append("framework", "React");
        let rendered = memory.render();
        assert!(rendered.contains("[context]"));
        assert!(rendered.contains("Next.js app"));
    }

    #[test]
    fn test_plan_deserialize() {
        let json = r#"{
            "goal": "Add Google OAuth",
            "tasks": [
                {
                    "id": "task_1",
                    "task_type": "read_file",
                    "instruction": "Read auth.ts",
                    "depends_on": []
                },
                {
                    "id": "task_2",
                    "task_type": "edit_file",
                    "instruction": "Add Google provider",
                    "depends_on": ["task_1"]
                }
            ]
        }"#;

        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.goal, "Add Google OAuth");
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].task_type, "read_file");
        assert_eq!(plan.tasks[1].depends_on, vec!["task_1"]);
    }

    #[test]
    fn test_tool_call_serialize() {
        let tc = ToolCall {
            id: "call_123".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({ "path": "src/main.rs" }),
        };

        let json = serde_json::to_string(&tc).unwrap();
        assert!(json.contains("call_123"));
        assert!(json.contains("read_file"));
    }

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.context_window, 128_000);
        assert_eq!(config.temperature, 0.0);
    }

    #[test]
    fn test_task_result() {
        let result = TaskResult {
            task_id: "t1".into(),
            task_type: "bash".into(),
            output: "Success".into(),
            success: true,
        };
        assert!(result.success);
        assert_eq!(result.task_type, "bash");
    }

    #[test]
    fn test_agent_event_clone() {
        let event = AgentEvent::Streaming {
            token: "hello".into(),
        };
        let cloned = event.clone();
        match cloned {
            AgentEvent::Streaming { token } => assert_eq!(token, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_tool_definition() {
        let def = ToolDefinition {
            name: "test".into(),
            description: "A test tool".into(),
            parameters: serde_json::json!({ "type": "object" }),
        };
        assert_eq!(def.name, "test");
    }
}

#[cfg(test)]
mod planner_tests {
    use crate::agent::planner::extract_json;

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
    fn test_extract_json_nested() {
        let text = r#"{"goal": "test", "tasks": [{"id": "1", "nested": {"a": 1}}]}"#;
        let json = extract_json(text);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tasks"][0]["nested"]["a"], 1);
    }
}

#[cfg(test)]
mod context_tests {
    use crate::context::ContextBuilder;
    use crate::types::*;

    fn make_state(task: &str) -> AgentState {
        AgentState {
            session_id: uuid::Uuid::new_v4(),
            task: task.into(),
            messages: Vec::new(),
            plan: None,
            results: Vec::new(),
            working_memory: WorkingMemory::new(10_000),
            workspace: std::path::PathBuf::from("/tmp/test"),
            config: AgentConfig::default(),
            iteration: 0,
            event_tx: None,
        }
    }

    #[test]
    fn test_context_builder_system_prompt() {
        let builder = ContextBuilder::new("my-project");
        let state = make_state("Add OAuth");
        let prompt = builder.build_system_prompt(&state);

        assert!(prompt.contains("SigmaCode"));
        assert!(prompt.contains("my-project"));
        assert!(prompt.contains("Iteration: 1"));
    }

    #[test]
    fn test_context_builder_with_instructions() {
        let builder = ContextBuilder::new("app").with_instructions("Use TypeScript strict mode");
        let state = make_state("Fix bug");
        let prompt = builder.build_system_prompt(&state);

        assert!(prompt.contains("TypeScript strict mode"));
    }

    #[test]
    fn test_context_builder_includes_rules() {
        let builder = ContextBuilder::new("test");
        let state = make_state("test");
        let prompt = builder.build_system_prompt(&state);

        assert!(prompt.contains("Read existing files before editing"));
        assert!(prompt.contains("minimal, targeted changes"));
        assert!(prompt.contains("Never expose secrets"));
    }
}

#[cfg(test)]
mod tool_tests {
    use crate::tools::ToolRouter;
    use crate::types::*;
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    fn make_context(workspace: &str) -> crate::tools::ToolContext {
        crate::tools::ToolContext {
            workspace: PathBuf::from(workspace),
            state: AgentState {
                session_id: uuid::Uuid::new_v4(),
                task: "test".into(),
                messages: Vec::new(),
                plan: None,
                results: Vec::new(),
                working_memory: WorkingMemory::new(10_000),
                workspace: PathBuf::from(workspace),
                config: AgentConfig::default(),
                iteration: 0,
                event_tx: None,
            },
            signal: CancellationToken::new(),
            output_tx: None,
        }
    }

    #[test]
    fn test_tool_router_default_has_tools() {
        let router = ToolRouter::default();
        let names = router.names();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"grep"));
    }

    #[test]
    fn test_tool_router_get_tool() {
        let router = ToolRouter::default();
        assert!(router.get("read_file").is_some());
        assert!(router.get("nonexistent").is_none());
    }

    #[test]
    fn test_tool_router_definitions() {
        let router = ToolRouter::default();
        let defs = router.definitions();
        assert_eq!(defs.len(), 6);
        assert!(defs.iter().any(|d| d.name == "bash"));
    }

    #[tokio::test]
    async fn test_read_file_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, "Hello\nWorld\nThird line").unwrap();

        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "read_file",
                serde_json::json!({ "path": "test.txt" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Hello"));
        assert!(result.content.contains("World"));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "read_file",
                serde_json::json!({ "path": "nonexistent.txt" }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_file_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "write_file",
                serde_json::json!({ "path": "output.txt", "content": "Written by test" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Successfully"));

        let content = std::fs::read_to_string(tmp.path().join("output.txt")).unwrap();
        assert_eq!(content, "Written by test");
    }

    #[tokio::test]
    async fn test_write_file_creates_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "write_file",
                serde_json::json!({ "path": "src/deep/file.rs", "content": "fn main() {}" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(tmp.path().join("src/deep/file.rs").exists());
    }

    #[tokio::test]
    async fn test_edit_file_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("edit.txt");
        std::fs::write(&file_path, "Hello World").unwrap();

        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "edit_file",
                serde_json::json!({
                    "path": "edit.txt",
                    "old_string": "World",
                    "new_string": "Rust"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello Rust");
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "edit_file",
                serde_json::json!({
                    "path": "nope.txt",
                    "old_string": "a",
                    "new_string": "b"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_string_not_matched() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("edit.txt");
        std::fs::write(&file_path, "Hello World").unwrap();

        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "edit_file",
                serde_json::json!({
                    "path": "edit.txt",
                    "old_string": "NotExist",
                    "new_string": "X"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("multi.txt");
        std::fs::write(&file_path, "aaa bbb aaa ccc aaa").unwrap();

        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "edit_file",
                serde_json::json!({
                    "path": "multi.txt",
                    "old_string": "aaa",
                    "new_string": "zzz",
                    "replaceAll": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "zzz bbb zzz ccc zzz");
    }

    #[tokio::test]
    async fn test_bash_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "bash",
                serde_json::json!({ "command": "echo hello" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(
            result.content.contains("hello") || result.content.contains("Hello"),
            "Output did not contain expected text: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_bash_tool_error() {
        let tmp = tempfile::tempdir().unwrap();
        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "bash",
                serde_json::json!({ "command": "exit 1" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_glob_tool() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn test() {}").unwrap();
        std::fs::write(tmp.path().join("c.txt"), "text").unwrap();

        let router = ToolRouter::default();
        let ctx = make_context(tmp.path().to_str().unwrap());

        let result = router
            .execute(
                "glob",
                serde_json::json!({ "pattern": "*.rs" }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("a.rs"));
        assert!(result.content.contains("b.rs"));
        assert!(!result.content.contains("c.txt"));
    }
}
