//! Memory storage using LanceDB.
//!
//! This module provides the main memory store that combines:
//! - Working memory (hot): Recent items kept in a VecDeque for fast access
//! - Long-term memory (cold): LanceDB storage with vector similarity search
//!
//! # Security Features (SEC-008)
//!
//! - Memory isolation: Agents can only access their own memories
//! - Caller authentication via `caller_agent_id` parameter
//! - Authorization checks on memory retrieval

use crate::embedding::EmbeddingService;
use crate::types::{Memory, MemoryConfig, MemoryType};
use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::{DataType, Field, FieldRef, Schema};
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, Table};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

/// Name of the memories table in LanceDB.
const MEMORIES_TABLE: &str = "memories";

/// Sanitize a string for use in LanceDB SQL filters.
/// Escapes single quotes to prevent SQL injection.
fn sanitize_filter_value(value: &str) -> String {
    value.replace('\'', "''")
}

// ============================================================================
// SEC-008: Memory Access Control
// ============================================================================

/// Memory access control policy (SEC-008).
///
/// Defines which agents can access which memories.
#[derive(Debug, Clone)]
pub struct MemoryAccessControl {
    /// Agents with shared memory access permissions.
    /// Key: agent_id, Value: set of agent_ids that can access this agent's memories.
    shared_access: Arc<RwLock<std::collections::HashMap<String, HashSet<String>>>>,
}

impl MemoryAccessControl {
    /// Create a new access control policy with no shared access.
    pub fn new() -> Self {
        Self {
            shared_access: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Check if `caller` can read `target`'s memories.
    ///
    /// SEC-008: Authorization check for memory access.
    pub async fn can_read(&self, caller: &str, target: &str) -> bool {
        // Own memories are always readable
        if caller == target {
            return true;
        }

        // Check for explicit shared access
        let shared = self.shared_access.read().await;
        if let Some(allowed) = shared.get(target) {
            return allowed.contains(caller);
        }

        false
    }

    /// Grant `caller` access to read `target`'s memories.
    pub async fn grant_access(&self, caller: &str, target: &str) {
        self.shared_access
            .write()
            .await
            .entry(target.to_string())
            .or_insert_with(HashSet::new)
            .insert(caller.to_string());

        info!(
            caller = caller,
            target = target,
            "Granted memory access"
        );
    }

    /// Revoke `caller`'s access to `target`'s memories.
    pub async fn revoke_access(&self, caller: &str, target: &str) {
        let mut shared = self.shared_access.write().await;
        if let Some(allowed) = shared.get_mut(target) {
            allowed.remove(caller);
            info!(
                caller = caller,
                target = target,
                "Revoked memory access"
            );
        }
    }
}

impl Default for MemoryAccessControl {
    fn default() -> Self {
        Self::new()
    }
}

/// The main memory store.
///
/// Combines working memory (hot) with LanceDB storage (cold).
pub struct MemoryStore {
    config: MemoryConfig,
    working_memory: Arc<RwLock<VecDeque<Memory>>>,
    db: Connection,
    table: Option<Table>,
    embedding_service: EmbeddingService,
}

impl MemoryStore {
    /// Create a new memory store.
    #[instrument(skip(config), fields(db_path = %config.db_path.display()))]
    pub async fn new(config: MemoryConfig) -> anyhow::Result<Self> {
        info!(
            db_path = %config.db_path.display(),
            embedding_dim = config.embedding_dim,
            "Initializing memory store"
        );

        // Ensure the database directory exists
        if let Some(parent) = config.db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Connect to LanceDB
        let db = lancedb::connect(config.db_path.to_string_lossy().as_ref())
            .execute()
            .await?;

        info!("Connected to LanceDB");

        // Initialize embedding service from config (validates model + dimension match)
        let embedding_service = EmbeddingService::from_config(
            &config.embedding_model,
            config.embedding_dim,
        ).map_err(|e| anyhow::anyhow!("Failed to initialize embedding service: {}", e))?;

        let mut store = Self {
            config,
            working_memory: Arc::new(RwLock::new(VecDeque::new())),
            db,
            table: None,
            embedding_service,
        };

        // Initialize or open the memories table
        store.init_table().await?;

        Ok(store)
    }

    /// Initialize or open the memories table.
    async fn init_table(&mut self) -> anyhow::Result<()> {
        // Check if table exists
        let table_names = self.db.table_names().execute().await?;

        if table_names.contains(&MEMORIES_TABLE.to_string()) {
            info!("Opening existing memories table");
            let table = self.db.open_table(MEMORIES_TABLE).execute().await?;
            self.table = Some(table);
        } else {
            info!("Creating new memories table");
            // Create an empty table with schema
            let schema = self.create_schema();
            let table = self
                .db
                .create_empty_table(MEMORIES_TABLE, schema)
                .execute()
                .await?;
            self.table = Some(table);
        }

        Ok(())
    }

    /// Create the Arrow schema for the memories table.
    fn create_schema(&self) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("memory_type", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("agent_id", DataType::Utf8, true),
            Field::new("task_id", DataType::Utf8, true),
            Field::new("tags", DataType::Utf8, true), // JSON-encoded array
            Field::new("importance", DataType::Float32, false),
            Field::new("created_at", DataType::UInt64, false),
            Field::new("last_accessed", DataType::UInt64, false),
            Field::new("access_count", DataType::UInt32, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.config.embedding_dim as i32,
                ),
                true,
            ),
        ]))
    }

    /// Convert a Memory struct to a RecordBatch.
    fn memory_to_batch(&self, memory: &Memory, embedding: Vec<f32>) -> anyhow::Result<RecordBatch> {
        let schema = self.create_schema();

        // Serialize tags to JSON
        let tags_json = serde_json::to_string(&memory.tags)?;

        // Build arrays for each column
        let id_array: ArrayRef = Arc::new(StringArray::from(vec![memory.id.as_str()]));
        let memory_type_array: ArrayRef = Arc::new(StringArray::from(vec![memory_type_to_str(
            memory.memory_type,
        )]));
        let content_array: ArrayRef = Arc::new(StringArray::from(vec![memory.content.as_str()]));
        let source_array: ArrayRef = Arc::new(StringArray::from(vec![memory.source.as_str()]));
        let agent_id_array: ArrayRef = Arc::new(StringArray::from(vec![memory
            .agent_id
            .as_deref()]));
        let task_id_array: ArrayRef =
            Arc::new(StringArray::from(vec![memory.task_id.as_deref()]));
        let tags_array: ArrayRef = Arc::new(StringArray::from(vec![Some(tags_json.as_str())]));
        let importance_array: ArrayRef = Arc::new(Float32Array::from(vec![memory.importance]));
        let created_at_array: ArrayRef = Arc::new(UInt64Array::from(vec![memory.created_at]));
        let last_accessed_array: ArrayRef =
            Arc::new(UInt64Array::from(vec![memory.last_accessed]));
        let access_count_array: ArrayRef = Arc::new(UInt32Array::from(vec![memory.access_count]));

        // Create the vector embedding as FixedSizeList
        let vector_array: ArrayRef = create_embedding_array(
            embedding,
            self.config.embedding_dim,
        )?;

        let batch = RecordBatch::try_new(
            schema,
            vec![
                id_array,
                memory_type_array,
                content_array,
                source_array,
                agent_id_array,
                task_id_array,
                tags_array,
                importance_array,
                created_at_array,
                last_accessed_array,
                access_count_array,
                vector_array,
            ],
        )?;

        Ok(batch)
    }

    /// Convert a RecordBatch row to a Memory struct.
    fn batch_to_memory(&self, batch: &RecordBatch, row: usize) -> anyhow::Result<Memory> {
        let id = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row).to_string())
            .ok_or_else(|| anyhow::anyhow!("Missing id column"))?;

        let memory_type_str = batch
            .column_by_name("memory_type")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow::anyhow!("Missing memory_type column"))?;
        let memory_type = str_to_memory_type(memory_type_str)?;

        let content = batch
            .column_by_name("content")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row).to_string())
            .ok_or_else(|| anyhow::anyhow!("Missing content column"))?;

        let source = batch
            .column_by_name("source")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row).to_string())
            .ok_or_else(|| anyhow::anyhow!("Missing source column"))?;

        let agent_id = batch
            .column_by_name("agent_id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    Some(a.value(row).to_string())
                }
            });

        let task_id = batch
            .column_by_name("task_id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    Some(a.value(row).to_string())
                }
            });

        let tags_json = batch
            .column_by_name("tags")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    Some(a.value(row))
                }
            });
        let tags: Vec<String> = tags_json
            .map(|j| serde_json::from_str(j).unwrap_or_default())
            .unwrap_or_default();

        let importance = batch
            .column_by_name("importance")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| a.value(row))
            .unwrap_or(0.5);

        let created_at = batch
            .column_by_name("created_at")
            .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
            .map(|a| a.value(row))
            .unwrap_or(0);

        let last_accessed = batch
            .column_by_name("last_accessed")
            .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
            .map(|a| a.value(row))
            .unwrap_or(0);

        let access_count = batch
            .column_by_name("access_count")
            .and_then(|c| c.as_any().downcast_ref::<UInt32Array>())
            .map(|a| a.value(row))
            .unwrap_or(0);

        // Extract embedding vector if present
        let embedding = batch
            .column_by_name("vector")
            .and_then(|c| c.as_any().downcast_ref::<FixedSizeListArray>())
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    let values = a.value(row);
                    values
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .map(|f| f.values().to_vec())
                }
            });

        Ok(Memory {
            id,
            memory_type,
            content,
            source,
            agent_id,
            task_id,
            tags,
            importance,
            created_at,
            last_accessed,
            access_count,
            embedding,
        })
    }

    /// Add a memory to working memory and optionally persist.
    #[instrument(skip(self, memory), fields(memory_id = %memory.id, memory_type = ?memory.memory_type))]
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
                    // Persist evicted memory if it's important
                    if evicted.importance > 0.7 {
                        // Don't await here to avoid blocking; spawn a task instead
                        let store_clone = self.clone_for_persist();
                        tokio::spawn(async move {
                            if let Err(e) = store_clone.persist_internal(&evicted).await {
                                warn!(error = %e, "Failed to persist evicted memory");
                            }
                        });
                    }
                }
            }
        }

        if persist {
            self.persist(&memory).await?;
        }

        Ok(())
    }

    /// Clone necessary parts for persistence in a spawned task
    fn clone_for_persist(&self) -> MemoryStorePersister {
        MemoryStorePersister {
            db_path: self.config.db_path.clone(),
            embedding_dim: self.config.embedding_dim,
        }
    }

    /// Persist a memory to LanceDB.
    #[instrument(skip(self, memory), fields(memory_id = %memory.id))]
    pub async fn persist(&self, memory: &Memory) -> anyhow::Result<()> {
        debug!(memory_id = %memory.id, "Persisting memory to LanceDB");

        // Generate embedding
        let embedding = self.embedding_service.embed(&memory.content).await?;

        // Create record batch
        let batch = self.memory_to_batch(memory, embedding)?;
        let schema = batch.schema();

        // Add to table
        if let Some(ref table) = self.table {
            let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
            table.add(batches).execute().await?;
            info!(memory_id = %memory.id, "Memory persisted to LanceDB");
        } else {
            warn!("No table available for persistence");
        }

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

    /// Search memories by query using vector similarity.
    #[instrument(skip(self), fields(query_len = query.len()))]
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

        // First, search working memory with keyword matching
        // NOTE: Lock scope is limited to avoid holding lock across async operations
        let mut results: Vec<Memory> = {
            let working = self.working_memory.read().await;
            working
                .iter()
                .filter(|m| {
                    // Filter by type if specified
                    if let Some(mt) = memory_type {
                        if m.memory_type != mt {
                            return false;
                        }
                    }
                    // Simple keyword match for working memory
                    m.content.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect()
            // Lock is dropped here before async embedding/vector search
        };

        // Search LanceDB with vector similarity (no lock held during these awaits)
        if let Some(ref table) = self.table {
            // Generate query embedding
            let query_embedding = self.embedding_service.embed(query).await?;

            // Build and execute vector search
            let mut vector_query = table.vector_search(query_embedding)?;
            vector_query = vector_query.limit(limit);

            // Add type filter if specified
            if let Some(mt) = memory_type {
                let type_str = memory_type_to_str(mt);
                vector_query = vector_query.only_if(format!("memory_type = '{}'", type_str));
            }

            // Execute query
            let mut stream = vector_query.execute().await?;

            // Collect results from stream
            use futures::StreamExt;
            while let Some(batch_result) = stream.next().await {
                match batch_result {
                    Ok(batch) => {
                        for row in 0..batch.num_rows() {
                            match self.batch_to_memory(&batch, row) {
                                Ok(mem) => {
                                    // Avoid duplicates (already in working memory)
                                    if !results.iter().any(|r| r.id == mem.id) {
                                        results.push(mem);
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to parse memory from batch");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Error reading search results");
                    }
                }
            }
        }

        // Sort by importance and recency
        results.sort_by(|a, b| {
            let score_a = a.importance + (a.access_count as f32 * 0.1);
            let score_b = b.importance + (b.access_count as f32 * 0.1);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(limit);
        Ok(results)
    }

    /// Get memories by agent from both working memory and LanceDB.
    ///
    /// Note: This is the unprotected version. For production use with
    /// multi-agent systems, use `get_by_agent_authorized()` instead.
    #[instrument(skip(self))]
    pub async fn get_by_agent(&self, agent_id: &str) -> Vec<Memory> {
        let mut results: Vec<Memory> = self
            .working_memory
            .read()
            .await
            .iter()
            .filter(|m| m.agent_id.as_deref() == Some(agent_id))
            .cloned()
            .collect();

        // Also query LanceDB
        if let Some(ref table) = self.table {
            let filter = format!("agent_id = '{}'", sanitize_filter_value(agent_id));
            match table
                .query()
                .only_if(filter)
                .limit(self.config.max_results)
                .execute()
                .await
            {
                Ok(mut stream) => {
                    use futures::StreamExt;
                    while let Some(batch_result) = stream.next().await {
                        if let Ok(batch) = batch_result {
                            for row in 0..batch.num_rows() {
                                if let Ok(mem) = self.batch_to_memory(&batch, row) {
                                    if !results.iter().any(|r| r.id == mem.id) {
                                        results.push(mem);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to query LanceDB for agent memories");
                }
            }
        }

        results
    }

    /// Get memories by task from both working memory and LanceDB.
    #[instrument(skip(self))]
    pub async fn get_by_task(&self, task_id: &str) -> Vec<Memory> {
        let mut results: Vec<Memory> = self
            .working_memory
            .read()
            .await
            .iter()
            .filter(|m| m.task_id.as_deref() == Some(task_id))
            .cloned()
            .collect();

        // Also query LanceDB
        if let Some(ref table) = self.table {
            let filter = format!("task_id = '{}'", sanitize_filter_value(task_id));
            match table
                .query()
                .only_if(filter)
                .limit(self.config.max_results)
                .execute()
                .await
            {
                Ok(mut stream) => {
                    use futures::StreamExt;
                    while let Some(batch_result) = stream.next().await {
                        if let Ok(batch) = batch_result {
                            for row in 0..batch.num_rows() {
                                if let Ok(mem) = self.batch_to_memory(&batch, row) {
                                    if !results.iter().any(|r| r.id == mem.id) {
                                        results.push(mem);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to query LanceDB for task memories");
                }
            }
        }

        results
    }

    /// Get memories by agent with authorization check (SEC-008).
    ///
    /// # Arguments
    /// * `caller_agent_id` - The agent making the request
    /// * `target_agent_id` - The agent whose memories to retrieve
    /// * `access_control` - The access control policy to check
    ///
    /// # Returns
    /// * `Ok(Vec<Memory>)` - Memories if authorized
    /// * `Err` - If the caller is not authorized to access the target's memories
    #[instrument(skip(self, access_control))]
    pub async fn get_by_agent_authorized(
        &self,
        caller_agent_id: &str,
        target_agent_id: &str,
        access_control: &MemoryAccessControl,
    ) -> anyhow::Result<Vec<Memory>> {
        // SEC-008: Check authorization
        if !access_control.can_read(caller_agent_id, target_agent_id).await {
            warn!(
                caller = caller_agent_id,
                target = target_agent_id,
                "Unauthorized memory access attempt"
            );
            anyhow::bail!(
                "Agent '{}' is not authorized to access memories of agent '{}'",
                caller_agent_id,
                target_agent_id
            );
        }

        debug!(
            caller = caller_agent_id,
            target = target_agent_id,
            "Authorized memory access"
        );

        Ok(self.get_by_agent(target_agent_id).await)
    }

    /// Add a memory with ownership validation (SEC-008).
    ///
    /// Ensures the memory's agent_id matches the caller_agent_id.
    #[instrument(skip(self, memory))]
    pub async fn add_authorized(
        &self,
        caller_agent_id: &str,
        memory: Memory,
        persist: bool,
    ) -> anyhow::Result<()> {
        // SEC-008: Validate that caller can only create memories for themselves
        if let Some(ref agent_id) = memory.agent_id {
            if agent_id != caller_agent_id {
                warn!(
                    caller = caller_agent_id,
                    memory_agent = agent_id,
                    "Attempt to create memory for different agent"
                );
                anyhow::bail!(
                    "Agent '{}' cannot create memories for agent '{}'",
                    caller_agent_id,
                    agent_id
                );
            }
        }

        self.add(memory, persist).await
    }

    /// Get the total number of memories in LanceDB.
    pub async fn count(&self) -> anyhow::Result<usize> {
        if let Some(ref table) = self.table {
            let count = table.count_rows(None).await?;
            Ok(count)
        } else {
            Ok(0)
        }
    }

    /// Create a vector index on the memories table.
    ///
    /// Call this after adding a significant number of memories to improve search performance.
    #[instrument(skip(self))]
    pub async fn create_index(&self) -> anyhow::Result<()> {
        if let Some(ref table) = self.table {
            info!("Creating vector index on memories table");
            table
                .create_index(&["vector"], lancedb::index::Index::Auto)
                .execute()
                .await?;
            info!("Vector index created successfully");
        }
        Ok(())
    }

    /// Pre-warm the embedding model.
    ///
    /// Call this during application startup to avoid latency on first search.
    pub async fn warmup(&self) -> anyhow::Result<()> {
        self.embedding_service.warmup().await
    }
}

/// Helper struct for persisting memories in spawned tasks.
struct MemoryStorePersister {
    db_path: std::path::PathBuf,
    embedding_dim: usize,
}

impl MemoryStorePersister {
    async fn persist_internal(&self, memory: &Memory) -> anyhow::Result<()> {
        // Open a new connection for this task
        let db = lancedb::connect(self.db_path.to_string_lossy().as_ref())
            .execute()
            .await?;
        let table = db.open_table(MEMORIES_TABLE).execute().await?;

        // Create embedding service
        let embedding_service = EmbeddingService::default();
        let embedding = embedding_service.embed(&memory.content).await?;

        // Create schema
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("memory_type", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("agent_id", DataType::Utf8, true),
            Field::new("task_id", DataType::Utf8, true),
            Field::new("tags", DataType::Utf8, true),
            Field::new("importance", DataType::Float32, false),
            Field::new("created_at", DataType::UInt64, false),
            Field::new("last_accessed", DataType::UInt64, false),
            Field::new("access_count", DataType::UInt32, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.embedding_dim as i32,
                ),
                true,
            ),
        ]));

        // Serialize tags to JSON
        let tags_json = serde_json::to_string(&memory.tags)?;

        // Build arrays
        let id_array: ArrayRef = Arc::new(StringArray::from(vec![memory.id.as_str()]));
        let memory_type_array: ArrayRef = Arc::new(StringArray::from(vec![memory_type_to_str(
            memory.memory_type,
        )]));
        let content_array: ArrayRef = Arc::new(StringArray::from(vec![memory.content.as_str()]));
        let source_array: ArrayRef = Arc::new(StringArray::from(vec![memory.source.as_str()]));
        let agent_id_array: ArrayRef = Arc::new(StringArray::from(vec![memory
            .agent_id
            .as_deref()]));
        let task_id_array: ArrayRef =
            Arc::new(StringArray::from(vec![memory.task_id.as_deref()]));
        let tags_array: ArrayRef = Arc::new(StringArray::from(vec![Some(tags_json.as_str())]));
        let importance_array: ArrayRef = Arc::new(Float32Array::from(vec![memory.importance]));
        let created_at_array: ArrayRef = Arc::new(UInt64Array::from(vec![memory.created_at]));
        let last_accessed_array: ArrayRef =
            Arc::new(UInt64Array::from(vec![memory.last_accessed]));
        let access_count_array: ArrayRef = Arc::new(UInt32Array::from(vec![memory.access_count]));

        let vector_array: ArrayRef = create_embedding_array(
            embedding,
            self.embedding_dim,
        )?;

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                id_array,
                memory_type_array,
                content_array,
                source_array,
                agent_id_array,
                task_id_array,
                tags_array,
                importance_array,
                created_at_array,
                last_accessed_array,
                access_count_array,
                vector_array,
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;

        Ok(())
    }
}

/// Convert MemoryType to string for storage.
fn memory_type_to_str(mt: MemoryType) -> &'static str {
    match mt {
        MemoryType::Episodic => "episodic",
        MemoryType::Semantic => "semantic",
        MemoryType::Procedural => "procedural",
    }
}

/// Convert string to MemoryType.
fn str_to_memory_type(s: &str) -> anyhow::Result<MemoryType> {
    match s {
        "episodic" => Ok(MemoryType::Episodic),
        "semantic" => Ok(MemoryType::Semantic),
        "procedural" => Ok(MemoryType::Procedural),
        _ => Err(anyhow::anyhow!("Unknown memory type: {}", s)),
    }
}

/// Create a FixedSizeListArray from an embedding vector.
///
/// Validates that the embedding length matches the expected dimension.
fn create_embedding_array(embedding: Vec<f32>, dim: usize) -> anyhow::Result<ArrayRef> {
    // Validate embedding length matches configured dimension
    if embedding.len() != dim {
        anyhow::bail!(
            "Embedding dimension mismatch: expected {} but got {} values",
            dim,
            embedding.len()
        );
    }

    let values = Float32Array::from(embedding);
    let field: FieldRef = Arc::new(Field::new("item", DataType::Float32, true));
    let array = FixedSizeListArray::try_new(
        field,
        dim as i32,
        Arc::new(values),
        None, // no nulls
    ).map_err(|e| anyhow::anyhow!("Failed to create embedding array: {}", e))?;
    Ok(Arc::new(array))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_memory_store_creation() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_add_to_working_memory() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            working_memory_size: 10,
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();

        let memory = Memory::episodic("Test content", "test");
        store.add(memory.clone(), false).await.unwrap();

        let working = store.get_working_memory().await;
        assert_eq!(working.len(), 1);
        assert_eq!(working[0].content, "Test content");
    }

    #[tokio::test]
    async fn test_working_memory_eviction() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            working_memory_size: 3, // Small size for testing
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();

        // Add more items than capacity
        for i in 0..5 {
            let memory = Memory::episodic(format!("Content {}", i), "test");
            store.add(memory, false).await.unwrap();
        }

        let working = store.get_working_memory().await;
        assert_eq!(working.len(), 3); // Should only have 3 items
        assert_eq!(working[0].content, "Content 2"); // First items evicted
    }

    #[tokio::test]
    async fn test_persist_and_search() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();

        // Add and persist a memory
        let memory = Memory::episodic("The quick brown fox jumps over the lazy dog", "test");
        store.add(memory, true).await.unwrap();

        // Wait a bit for persistence
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Search for it
        let results = store.search("fox", None, Some(5)).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("fox"));
    }

    #[tokio::test]
    async fn test_search_by_memory_type() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();

        let episodic = Memory::episodic("Episodic content about cats", "test");
        let semantic = Memory::semantic("Semantic knowledge about cats", "test");

        store.add(episodic, false).await.unwrap();
        store.add(semantic, false).await.unwrap();

        // Search only episodic
        let results = store
            .search("cats", Some(MemoryType::Episodic), Some(5))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Episodic"));
    }

    #[tokio::test]
    async fn test_get_by_agent() {
        let dir = tempdir().unwrap();
        let config = MemoryConfig {
            db_path: dir.path().join("test_memory"),
            ..Default::default()
        };

        let store = MemoryStore::new(config).await.unwrap();

        let mem1 = Memory::episodic("Agent A memory", "test").with_agent("agent_a");
        let mem2 = Memory::episodic("Agent B memory", "test").with_agent("agent_b");

        store.add(mem1, false).await.unwrap();
        store.add(mem2, false).await.unwrap();

        let results = store.get_by_agent("agent_a").await;
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Agent A"));
    }
}
