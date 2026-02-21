//! Memory types and configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Type of memory being stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Conversation history and task outcomes
    Episodic,
    /// Facts and knowledge
    Semantic,
    /// Learned patterns and preferences
    Procedural,
}

/// A memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique ID
    pub id: String,

    /// Memory type
    pub memory_type: MemoryType,

    /// The actual content
    pub content: String,

    /// Source (agent, conversation, etc.)
    pub source: String,

    /// Associated agent ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Associated task ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,

    /// Tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,

    /// Importance score (0.0 - 1.0)
    #[serde(default = "default_importance")]
    pub importance: f32,

    /// Creation timestamp (Unix millis)
    pub created_at: u64,

    /// Last access timestamp
    pub last_accessed: u64,

    /// Access count (for LRU/importance)
    #[serde(default)]
    pub access_count: u32,

    /// Vector embedding (populated by embedding service)
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
}

fn default_importance() -> f32 {
    0.5
}

impl Memory {
    pub fn episodic(content: impl Into<String>, source: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            id: format!("mem_{:016x}", now),
            memory_type: MemoryType::Episodic,
            content: content.into(),
            source: source.into(),
            agent_id: None,
            task_id: None,
            tags: vec![],
            importance: default_importance(),
            created_at: now,
            last_accessed: now,
            access_count: 0,
            embedding: None,
        }
    }

    pub fn semantic(content: impl Into<String>, source: impl Into<String>) -> Self {
        let mut mem = Self::episodic(content, source);
        mem.memory_type = MemoryType::Semantic;
        mem
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

/// Configuration for the memory system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Path to LanceDB database
    pub db_path: PathBuf,

    /// Embedding model name
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Embedding dimension
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,

    /// Maximum memories to return in search
    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// Minimum similarity score for retrieval
    #[serde(default = "default_min_similarity")]
    pub min_similarity: f32,

    /// Weight for vector search (vs keyword)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,

    /// Maximum working memory items
    #[serde(default = "default_working_memory_size")]
    pub working_memory_size: usize,
}

fn default_embedding_model() -> String {
    "all-MiniLM-L6-v2".into()
}

fn default_embedding_dim() -> usize {
    384 // MiniLM dimension
}

fn default_max_results() -> usize {
    10
}

fn default_min_similarity() -> f32 {
    0.5
}

fn default_vector_weight() -> f32 {
    0.7
}

fn default_working_memory_size() -> usize {
    20
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("./data/memory"),
            embedding_model: default_embedding_model(),
            embedding_dim: default_embedding_dim(),
            max_results: default_max_results(),
            min_similarity: default_min_similarity(),
            vector_weight: default_vector_weight(),
            working_memory_size: default_working_memory_size(),
        }
    }
}
