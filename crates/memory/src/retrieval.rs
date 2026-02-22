//! Memory retrieval with context building.

use crate::store::MemoryStore;
use crate::types::{Memory, MemoryType};
use std::sync::Arc;
use tracing::debug;

/// Retrieves and formats memories for agent context.
pub struct MemoryRetriever {
    store: Arc<MemoryStore>,
    max_context_tokens: usize,
}

impl MemoryRetriever {
    pub fn new(store: Arc<MemoryStore>, max_context_tokens: usize) -> Self {
        Self {
            store,
            max_context_tokens,
        }
    }

    /// Build context for an agent from relevant memories.
    pub async fn build_context(
        &self,
        query: &str,
        agent_id: Option<&str>,
    ) -> anyhow::Result<String> {
        debug!(query = %query, agent_id = ?agent_id, "Building memory context");

        let mut context_parts = Vec::new();
        let mut token_count = 0;

        // Get working memory first (most recent/relevant)
        let working = self.store.get_working_memory().await;
        for mem in working.iter().rev().take(5) {
            let part = format_memory(mem);
            let tokens = estimate_tokens(&part);
            if token_count + tokens > self.max_context_tokens {
                break;
            }
            context_parts.push(part);
            token_count += tokens;
        }

        // Search for relevant memories
        let search_results = self.store.search(query, None, Some(5)).await?;
        for mem in search_results {
            let part = format_memory(&mem);
            let tokens = estimate_tokens(&part);
            if token_count + tokens > self.max_context_tokens {
                break;
            }
            // Avoid duplicates
            if !context_parts.contains(&part) {
                context_parts.push(part);
                token_count += tokens;
            }
        }

        // Get agent-specific memories if specified
        if let Some(aid) = agent_id {
            let agent_mems = self.store.get_by_agent(aid).await;
            for mem in agent_mems.iter().take(3) {
                let part = format_memory(mem);
                let tokens = estimate_tokens(&part);
                if token_count + tokens > self.max_context_tokens {
                    break;
                }
                if !context_parts.contains(&part) {
                    context_parts.push(part);
                    token_count += tokens;
                }
            }
        }

        if context_parts.is_empty() {
            return Ok(String::new());
        }

        let context = format!(
            "## Relevant Context from Memory\n\n{}",
            context_parts.join("\n\n")
        );

        debug!(
            memory_count = context_parts.len(),
            estimated_tokens = token_count,
            "Built memory context"
        );

        Ok(context)
    }

    /// Get a summary of recent activities.
    pub async fn get_recent_summary(&self) -> String {
        let working = self.store.get_working_memory().await;
        if working.is_empty() {
            return "No recent activity.".into();
        }

        let recent: Vec<String> = working
            .iter()
            .rev()
            .take(5)
            .map(|m| format!("- {}: {}", m.source, truncate(&m.content, 100)))
            .collect();

        format!("Recent activity:\n{}", recent.join("\n"))
    }
}

fn format_memory(mem: &Memory) -> String {
    let type_str = match mem.memory_type {
        MemoryType::Episodic => "Event",
        MemoryType::Semantic => "Fact",
        MemoryType::Procedural => "Pattern",
    };

    format!(
        "**[{}]** ({})\n{}",
        type_str,
        mem.source,
        mem.content
    )
}

fn estimate_tokens(text: &str) -> usize {
    // Rough estimate: ~4 chars per token
    text.len() / 4
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        match s.char_indices().nth(max_len) {
            Some((idx, _)) => format!("{}...", &s[..idx]),
            None => s.to_string(),
        }
    }
}
