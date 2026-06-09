use crate::types::{AgentMode, AgentState, ToolDefinition};

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
        self.build_prompt_for_mode(&AgentMode::Builder, state)
    }

    pub fn build_prompt_for_mode(&self, mode: &AgentMode, state: &AgentState) -> String {
        match mode {
            AgentMode::Chat => self.build_chat_prompt(),
            AgentMode::Planner => self.build_planner_prompt(state),
            AgentMode::Builder => self.build_builder_prompt(state),
        }
    }

    pub fn build_chat_prompt(&self) -> String {
        format!(
            r#"You are SigmaCode, a helpful AI assistant.

## Project: {}

Answer questions concisely and helpfully. You are in **Chat mode** — no tools, no file operations, just conversation.

Rules:
- Be concise and direct
- Use markdown formatting when helpful
- If the user asks for code changes, suggest switching to Builder mode
- If the user asks for a plan, suggest switching to Planner mode"#,
            self.project_name
        )
    }

    fn build_planner_prompt(&self, state: &AgentState) -> String {
        let tool_list: Vec<String> = self.tools
            .iter()
            .map(|t| {
                let params = serde_json::to_string_pretty(&t.parameters).unwrap_or_default();
                format!("- {}: {}\n  Parameters: {}", t.name, t.description, params)
            })
            .collect();

        let tools_section = if !self.tools.is_empty() {
            format!(
                "\n## Available Tools\n\n{}\n\nThese are the tools available for execution. Use them to inform your plan.",
                tool_list.join("\n\n")
            )
        } else {
            String::new()
        };

        let mut prompt = format!(
            r#"You are SigmaCode, an expert AI coding assistant in **Planner mode**.

Your job is to analyze the user's request and produce a clear, step-by-step plan. Do NOT execute anything — just plan.

## Project: {}

## Rules:
1. Analyze the request thoroughly
2. Identify what files need to be created, modified, or read
3. Detect the project framework (React/Next.js/Vue/Rust/Python/etc.)
4. Check existing dependencies before planning installs
5. Produce a numbered plan with clear steps
6. Estimate complexity and risk

## Output Format:
Provide your plan as a numbered list. Each step should be specific and actionable.
Include which tools would be used for each step.

## Workspace: {}{}
"#,
            self.project_name,
            state.workspace.display(),
            tools_section,
        );

        if let Some(ref instructions) = self.custom_instructions {
            prompt.push_str(&format!("\n## Additional Instructions\n{}\n", instructions));
        }

        prompt
    }

    fn build_builder_prompt(&self, state: &AgentState) -> String {
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
2. NEVER run framework-specific commands without first detecting the framework
   - Read package.json and check dependencies to determine: React, Next.js, Vue, Svelte, etc.
   - If "next" is in dependencies → it's a Next.js project
   - If "react" is in dependencies and "next" is NOT → it's a plain React project
   - If "vue" is in dependencies → it's a Vue project
   - Use ONLY commands appropriate for the detected framework
3. Never reinstall dependencies that already exist
4. Make minimal, targeted changes - never rewrite entire files unnecessarily
5. After making changes, verify with build/test commands when appropriate
6. If a tool call fails, analyze the error and try a different approach
7. Use the edit_file tool for precise changes (requires exact string match)
8. Use write_file only for new files or complete rewrites
9. Run bash commands to verify changes (npm run build, cargo check, etc.)
10. Be concise in your responses - focus on the task
11. Never expose secrets, API keys, or sensitive data
12. If you're unsure about something, ask the user

## Workspace Discovery (do this FIRST for any task):
- For web projects: read package.json to detect framework (React/Next.js/Vue/etc.)
- Check dependencies to determine framework: "next" = Next.js, "react" without "next" = React
- Check if node_modules / target / .venv exists before installing
- For Rust projects: glob for Cargo.toml, check target exists
- For Python projects: glob for pyproject.toml/requirements.txt, check .venv exists
- Read the main config file to understand existing dependencies and scripts
- Only then plan your approach based on what already exists

## Framework-Specific Commands:
### React (Vite) — when package.json has "react" but NOT "next":
- Scaffold: `npm create vite@latest app -- --template react-ts`
- Dev: `npm run dev`
- Build: `npm run build`
- Add packages: `npm install <package>`

### Next.js — when package.json has "next":
- Scaffold: `npx create-next-app@latest`
- Dev: `npm run dev`
- Build: `npm run build`
- Add packages: `npm install <package>`
- shadcn: `npx shadcn@latest init`

### Vue — when package.json has "vue":
- Scaffold: `npm create vue@latest`
- Dev: `npm run dev`
- Build: `npm run build`

## Tooling Preferences:
- NEVER use create-react-app (CRA) — it is deprecated and slow
- For React/Vue/Svelte/TS projects, use `bun create vite` or `npm create vite@latest`
- For package management, prefer `bun` over `npm` when available
- When scaffolding, always use the latest modern tools (Vite, Turbopack, esbuild)
- ALWAYS use `-y` flag with npx to auto-confirm package installs{}
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
