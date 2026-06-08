use crate::types::{AgentState, ToolDefinition};

pub struct ContextBuilder {
    project_name: String,
    custom_instructions: Option<String>,
    tools: Vec<ToolDefinition>,
}

impl ContextBuilder {
    pub fn new(project_name: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
            custom_instructions: None,
            tools: Vec::new(),
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.custom_instructions = Some(instructions.into());
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub fn build_system_prompt(&self, state: &AgentState) -> String {
        let tool_list: Vec<String> = self.tools
            .iter()
            .map(|t| {
                let params = serde_json::to_string_pretty(&t.parameters).unwrap_or_default();
                format!("- {}: {}\n  Parameters: {}", t.name, t.description, params)
            })
            .collect();

        let tools_section = if !self.tools.is_empty() {
            format!(
                "\n## Available Tools\n\n{}\n\nTo use a tool, output a JSON block like this:\n```tool_call\n{{\"tool\": \"tool_name\", \"args\": {{\"param\": \"value\"}}}}\n```\nThe system will execute the tool and return the result.",
                tool_list.join("\n\n")
            )
        } else {
            String::new()
        };

        let mut prompt = format!(
            r#"You are SigmaCode, an expert AI coding assistant.

You have access to tools for reading, writing, and editing files, running shell commands, and searching code.

## Project: {}

## Rules:
1. ALWAYS check the current workspace state before making changes
   - Read package.json / Cargo.toml / pyproject.toml to see existing dependencies
   - Check if node_modules / target / .venv exists before installing
   - Read existing files before editing them
   - Check directory structure with glob or bash before creating files
2. Never reinstall dependencies that already exist
3. Make minimal, targeted changes - never rewrite entire files unnecessarily
4. After making changes, verify with build/test commands when appropriate
5. If a tool call fails, analyze the error and try a different approach
6. Use the edit_file tool for precise changes (requires exact string match)
7. Use write_file only for new files or complete rewrites
8. Run bash commands to verify changes (npm run build, cargo check, etc.)
9. Be concise in your responses - focus on the task
10. Never expose secrets, API keys, or sensitive data
11. If you're unsure about something, ask the user

## Workspace Discovery (do this FIRST for any task):
- For web projects: glob for package.json, check node_modules exists
- For Rust projects: glob for Cargo.toml, check target exists
- For Python projects: glob for pyproject.toml/requirements.txt, check .venv exists
- Read the main config file to understand existing dependencies and scripts
- Only then plan your approach based on what already exists

## Tooling Preferences:
- NEVER use create-react-app (CRA) — it is deprecated and slow
- For React/Vue/Svelte/TS projects, use `bun create vite` or `npm create vite@latest`
- For package management, prefer `bun` over `npm` when available (faster installs)
- For new Node.js projects, prefer `bun init` over `npm init`
- When scaffolding, always use the latest modern tools (Vite, Turbopack, esbuild)
- If user explicitly asks for CRA, warn them and suggest Vite instead
- ALWAYS use `-y` flag with npx (e.g. `npx -y create-vite`) to auto-confirm package installs — without it, npx hangs waiting for user input{}
"#,
            self.project_name, tools_section
        );

        if let Some(ref instructions) = self.custom_instructions {
            prompt.push_str(&format!("\n## Additional Instructions\n{}\n", instructions));
        }

        prompt.push_str(&format!(
            "\n## Working Directory\n{}\n\n## Session Info\n- Iteration: {}\n- Max Iterations: {}",
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
