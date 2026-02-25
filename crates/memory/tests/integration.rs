//! Integration tests for the memory store.
//!
//! These tests exercise the LanceDB-backed memory system end-to-end,
//! including persistence, search, and working memory.

use panoptes_memory::{Memory, MemoryConfig, MemoryStore, MemoryType};
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> MemoryConfig {
    MemoryConfig {
        db_path: dir.path().join("test_memory"),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_store_and_retrieve() {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::new(test_config(&dir)).await.unwrap();
    store.warmup().await.unwrap();

    let memory = Memory::semantic("Rust is a systems programming language", "test")
        .with_agent("research")
        .with_importance(0.8);

    store.add(memory, true).await.unwrap();

    let results = store
        .search("Rust programming", Some(MemoryType::Semantic), Some(5))
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("Rust"));
}

#[tokio::test]
async fn test_working_memory() {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::new(test_config(&dir)).await.unwrap();
    store.warmup().await.unwrap();

    let memory = Memory::episodic("User asked about async", "test").with_agent("research");

    store.add(memory, true).await.unwrap();

    let working = store.get_working_memory().await;
    assert!(!working.is_empty());
}

#[tokio::test]
async fn test_multiple_memory_types() {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::new(test_config(&dir)).await.unwrap();
    store.warmup().await.unwrap();

    let semantic = Memory::semantic("HTTP is a protocol", "test").with_agent("research");
    let episodic = Memory::episodic("User discussed HTTP yesterday", "test").with_agent("research");

    store.add(semantic, true).await.unwrap();
    store.add(episodic, true).await.unwrap();

    let semantic_results = store
        .search("HTTP protocol", Some(MemoryType::Semantic), Some(5))
        .await
        .unwrap();
    assert!(!semantic_results.is_empty());
}

#[tokio::test]
async fn test_memory_count() {
    let dir = TempDir::new().unwrap();
    let store = MemoryStore::new(test_config(&dir)).await.unwrap();
    store.warmup().await.unwrap();

    let count_before = store.count().await.unwrap();

    let memory = Memory::semantic("Test content", "test").with_agent("test");
    store.add(memory, true).await.unwrap();

    // Wait briefly for persistence
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let count_after = store.count().await.unwrap();
    assert_eq!(count_after, count_before + 1);
}
