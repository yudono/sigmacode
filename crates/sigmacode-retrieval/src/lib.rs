use std::path::Path;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file: String,
    pub line: usize,
    pub content: String,
    pub score: f32,
}

pub struct RetrievalEngine;

impl RetrievalEngine {
    pub fn new() -> Self {
        Self
    }

    pub async fn search(&self, _query: &str, _workspace: &Path) -> Vec<SearchResult> {
        // Placeholder - will implement ripgrep + AST + symbol search
        Vec::new()
    }
}

impl Default for RetrievalEngine {
    fn default() -> Self {
        Self::new()
    }
}
