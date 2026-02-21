//! Memory storage using LanceDB.

use crate::types::{Memory, MemoryConfig, MemoryType};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// The main memory store.
///
/// Combines working memory (hot) with LanceDB storage (cold).
pub struct MemoryStore {
    config: MemoryConfig,
    working_memory: Arc<RwLock<VecDeque<Memory>>>,
    // TODO: LanceDB connection
    // db: lancedb::Connection,
}

impl MemoryStore {
    /// Create a new memory store.
    pub async fn new(config: MemoryConfig) -> anyhow::Result<Self> {
        info!(
            db_path = %config.db_path.display(),
            "Initializing memory store"
        );

        // TODO: Initialize LanceDB
        // let db = lancedb::connect(&config.db_path.to_string_lossy()).await?;
        // Create tables if needed

        Ok(Self {
            config,
            working_memory: Arc::new(RwLock::new(VecDeque::new())),
        })
    }

    /// Add a memory to working memory and optionally persist.
    pub async fn add(&self, memory: Memory, persist: bool) -> anyhow::Result<()> {
        debug!(
            memory_id = %memory.id,
            memory_type = ?memory.memory_type,
            persist = persist,
            "Adding memory"
        );

        // Add to working memory
        {
            let mut wm = self.working_memory.write().await;
            wm.push_back(memory.clone());

            // Evict old items if over capacity
            while wm.len() > self.config.working_memory_size {
                if let Some(evicted) = wm.pop_front() {
                    debug!(memory_id = %evicted.id, "Evicted from working memory");
                    // TODO: Persist evicted memory if important
                }
            }
        }

        if persist {
            self.persist(&memory).await?;
        }

        Ok(())
    }

    /// Persist a memory to LanceDB.
    async fn persist(&self, memory: &Memory) -> anyhow::Result<()> {
        debug!(memory_id = %memory.id, "Persisting memory to LanceDB");

        // TODO: Generate embedding
        // let embedding = self.embed(&memory.content).await?;

        // TODO: Insert into LanceDB
        // self.db.table("memories")?.add(&[memory]).await?;

        warn!("LanceDB persistence not yet implemented");
        Ok(())
    }

    /// Get working memory contents.
    pub async fn get_working_memory(&self) -> Vec<Memory> {
        self.working_memory.read().await.iter().cloned().collect()
    }

    /// Clear working memory.
    pub async fn clear_working_memory(&self) {
        self.working_memory.write().await.clear();
    }

    /// Search memories by query.
    pub async fn search(
        &self,
        query: &str,
        memory_type: Option<MemoryType>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<Memory>> {
        let limit = limit.unwrap_or(self.config.max_results);

        debug!(
            query = %query,
            memory_type = ?memory_type,
            limit = limit,
            "Searching memories"
        );

        // First, search working memory
        let working = self.working_memory.read().await;
        let mut results: Vec<Memory> = working
            .iter()
            .filter(|m| {
                // Filter by type if specified
                if let Some(mt) = memory_type {
                    if m.memory_type != mt {
                        return false;
                    }
                }
                // Simple keyword match for now
                m.content.to_lowercase().contains(&query.to_lowercase())
            })
            .cloned()
            .collect();

        // TODO: Also search LanceDB
        // let embedding = self.embed(query).await?;
        // let db_results = self.db.table("memories")?
        //     .search(&embedding)
        //     .limit(limit)
        //     .execute()
        //     .await?;

        // Sort by importance and recency
        results.sort_by(|a, b| {
            let score_a = a.importance + (a.access_count as f32 * 0.1);
            let score_b = b.importance + (b.access_count as f32 * 0.1);
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(limit);
        Ok(results)
    }

    /// Get memories by agent.
    pub async fn get_by_agent(&self, agent_id: &str) -> Vec<Memory> {
        self.working_memory
            .read()
            .await
            .iter()
            .filter(|m| m.agent_id.as_deref() == Some(agent_id))
            .cloned()
            .collect()
    }

    /// Get memories by task.
    pub async fn get_by_task(&self, task_id: &str) -> Vec<Memory> {
        self.working_memory
            .read()
            .await
            .iter()
            .filter(|m| m.task_id.as_deref() == Some(task_id))
            .cloned()
            .collect()
    }
}
