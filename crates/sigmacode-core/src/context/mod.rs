use crate::types::AgentState;

pub struct ContextBuilder {
    project_name: String,
    custom_instructions: Option<String>,
}

impl ContextBuilder {
    pub fn new(project_name: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
            custom_instructions: None,
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.custom_instructions = Some(instructions.into());
        self
    }

    pub fn build_system_prompt(&self, state: &AgentState) -> String {
        let mut prompt = format!(
            r#"You are SigmaCode, an expert AI coding assistant.

You have access to tools for reading, writing, and editing files, running shell commands, and searching code.

## Project: {}

## Rules:
1. Always read files before editing them
2. Make minimal, targeted changes - never rewrite entire files unnecessarily
3. After making changes, verify with build/test commands when appropriate
4. If a tool call fails, analyze the error and try a different approach
5. Use the edit_file tool for precise changes (requires exact string match)
6. Use write_file only for new files or complete rewrites
7. Run bash commands to verify changes (npm run build, cargo check, etc.)
8. Be concise in your responses - focus on the task
9. Never expose secrets, API keys, or sensitive data
10. If you're unsure about something, ask the user"#,
            self.project_name
        );

        if let Some(ref instructions) = self.custom_instructions {
            prompt.push_str(&format!("\n\n## Additional Instructions\n{}", instructions));
        }

        prompt.push_str(&format!(
            "\n\n## Working Directory\n{}\n\n## Session Info\n- Iteration: {}\n- Max Iterations: {}",
            state.workspace.display(),
            state.iteration + 1,
            state.config.max_iterations,
        ));

        prompt
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new("unknown")
    }
}
