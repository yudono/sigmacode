use std::path::PathBuf;
use crate::types::SessionMemory;

#[allow(dead_code)]
pub struct MemoryManager {
    session: SessionMemory,
    workspace_path: PathBuf,
}

impl MemoryManager {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            session: SessionMemory {
                actions: Vec::new(),
                files_modified: Vec::new(),
                errors_encountered: Vec::new(),
                patterns_learned: Vec::new(),
            },
            workspace_path: workspace,
        }
    }

    pub fn record_action(&mut self, action: String) {
        self.session.actions.push(action);
    }

    pub fn record_file_modified(&mut self, file: String) {
        if !self.session.files_modified.contains(&file) {
            self.session.files_modified.push(file);
        }
    }

    pub fn record_error(&mut self, error: String) {
        self.session.errors_encountered.push(error);
    }

    pub fn record_pattern(&mut self, pattern: String) {
        if !self.session.patterns_learned.contains(&pattern) {
            self.session.patterns_learned.push(pattern);
        }
    }

    pub fn get_context_for_planning(&self) -> String {
        let mut context = String::new();

        if !self.session.actions.is_empty() {
            context.push_str("Previous actions:\n");
            for action in self.session.actions.iter().take(10) {
                context.push_str(&format!("- {}\n", action));
            }
        }

        if !self.session.files_modified.is_empty() {
            context.push_str("\nFiles modified:\n");
            for file in &self.session.files_modified {
                context.push_str(&format!("- {}\n", file));
            }
        }

        if !self.session.errors_encountered.is_empty() {
            context.push_str("\nErrors encountered:\n");
            for error in self.session.errors_encountered.iter().take(5) {
                context.push_str(&format!("- {}\n", error));
            }
        }

        if !self.session.patterns_learned.is_empty() {
            context.push_str("\nPatterns learned:\n");
            for pattern in &self.session.patterns_learned {
                context.push_str(&format!("- {}\n", pattern));
            }
        }

        context
    }

    pub fn get_session_memory(&self) -> &SessionMemory {
        &self.session
    }

    pub fn clear(&mut self) {
        self.session = SessionMemory {
            actions: Vec::new(),
            files_modified: Vec::new(),
            errors_encountered: Vec::new(),
            patterns_learned: Vec::new(),
        };
    }
}
