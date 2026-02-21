//! LanceDB-backed memory system for Argus-Panoptes.
//!
//! This crate provides a dual-layer memory architecture:
//!
//! - **Hot Path**: Recent context kept in working memory
//! - **Cold Path**: Long-term storage in LanceDB with vector search
//!
//! # Memory Types
//!
//! - **Episodic**: Conversation history and task outcomes
//! - **Semantic**: Facts and knowledge extracted from interactions
//! - **Procedural**: Learned patterns and preferences
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    MEMORY SYSTEM                            │
//! ├─────────────────────────────────────────────────────────────┤
//! │                                                             │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │              Working Memory (Hot)                    │   │
//! │  │  - Recent messages                                   │   │
//! │  │  - Active task context                               │   │
//! │  │  - Session state                                     │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! │                         │                                   │
//! │                         ▼ save/recall                       │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │              Long-term Memory (Cold)                 │   │
//! │  │                                                      │   │
//! │  │  ┌──────────┐  ┌──────────┐  ┌──────────┐          │   │
//! │  │  │ Episodic │  │ Semantic │  │Procedural│          │   │
//! │  │  │ (events) │  │ (facts)  │  │(patterns)│          │   │
//! │  │  └──────────┘  └──────────┘  └──────────┘          │   │
//! │  │                                                      │   │
//! │  │  ┌──────────────────────────────────────────────┐   │   │
//! │  │  │              LanceDB Storage                  │   │   │
//! │  │  │  - Vector embeddings                          │   │   │
//! │  │  │  - Full-text search                           │   │   │
//! │  │  │  - Hybrid retrieval                           │   │   │
//! │  │  └──────────────────────────────────────────────┘   │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod embedding;
pub mod retrieval;
pub mod store;
pub mod types;

pub use retrieval::MemoryRetriever;
pub use store::MemoryStore;
pub use types::{Memory, MemoryConfig, MemoryType};
