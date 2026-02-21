//! Embedding generation for vector search using fastembed.
//!
//! This module provides a thread-safe embedding service that uses the
//! all-MiniLM-L6-v2 model (384 dimensions) for generating vector representations.

use std::sync::Arc;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;
use thiserror::Error;
use tokio::task;
use tracing::{debug, info, instrument};

/// Errors that can occur during embedding operations.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("Failed to initialize embedding model: {0}")]
    ModelInit(String),

    #[error("Failed to generate embeddings: {0}")]
    Generation(String),

    #[error("Blocking task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

/// Embedding service for generating vector representations.
///
/// Uses fastembed with lazy-loaded model initialization for efficient memory usage.
/// The model is initialized on first use and shared across all embedding calls.
pub struct EmbeddingService {
    model_name: EmbeddingModel,
    dimension: usize,
    /// Lazily initialized text embedding model
    model: OnceCell<Arc<TextEmbedding>>,
}

impl EmbeddingService {
    /// Creates a new embedding service with the specified model.
    ///
    /// The model is not loaded until the first embedding call.
    pub fn new(model_name: EmbeddingModel) -> Self {
        let dimension = match model_name {
            EmbeddingModel::AllMiniLML6V2 => 384,
            EmbeddingModel::AllMiniLML6V2Q => 384,
            EmbeddingModel::AllMiniLML12V2 => 384,
            EmbeddingModel::AllMiniLML12V2Q => 384,
            EmbeddingModel::BGEBaseENV15 => 768,
            EmbeddingModel::BGEBaseENV15Q => 768,
            EmbeddingModel::BGELargeENV15 => 1024,
            EmbeddingModel::BGELargeENV15Q => 1024,
            EmbeddingModel::BGESmallENV15 => 384,
            EmbeddingModel::BGESmallENV15Q => 384,
            EmbeddingModel::NomicEmbedTextV1 => 768,
            EmbeddingModel::NomicEmbedTextV15 => 768,
            EmbeddingModel::NomicEmbedTextV15Q => 768,
            EmbeddingModel::ParaphraseMLMiniLML12V2 => 384,
            EmbeddingModel::ParaphraseMLMiniLML12V2Q => 384,
            EmbeddingModel::ParaphraseMLMpnetBaseV2 => 768,
            EmbeddingModel::BGESmallZHV15 => 512,
            EmbeddingModel::MultilingualE5Small => 384,
            EmbeddingModel::MultilingualE5Base => 768,
            EmbeddingModel::MultilingualE5Large => 1024,
            EmbeddingModel::MxbaiEmbedLargeV1 => 1024,
            EmbeddingModel::MxbaiEmbedLargeV1Q => 1024,
            EmbeddingModel::GTEBaseENV15 => 768,
            EmbeddingModel::GTEBaseENV15Q => 768,
            EmbeddingModel::GTELargeENV15 => 1024,
            EmbeddingModel::GTELargeENV15Q => 1024,
            EmbeddingModel::JinaEmbeddingsV2BaseCode => 768,
            _ => 384, // Default fallback for any new/unknown models
        };

        Self {
            model_name,
            dimension,
            model: OnceCell::new(),
        }
    }

    /// Creates a new embedding service with a custom model and dimension.
    ///
    /// Use this when you need explicit control over the dimension or are using
    /// a model not in the predefined list.
    pub fn with_dimension(model_name: EmbeddingModel, dimension: usize) -> Self {
        Self {
            model_name,
            dimension,
            model: OnceCell::new(),
        }
    }

    /// Creates an embedding service from a model name string.
    ///
    /// Returns an error if the model name is not recognized.
    pub fn from_model_str(model_name: &str) -> Result<Self, EmbeddingError> {
        let model = match model_name {
            "all-MiniLM-L6-v2" | "AllMiniLML6V2" => EmbeddingModel::AllMiniLML6V2,
            "all-MiniLM-L6-v2-q" | "AllMiniLML6V2Q" => EmbeddingModel::AllMiniLML6V2Q,
            "all-MiniLM-L12-v2" | "AllMiniLML12V2" => EmbeddingModel::AllMiniLML12V2,
            "all-MiniLM-L12-v2-q" | "AllMiniLML12V2Q" => EmbeddingModel::AllMiniLML12V2Q,
            "bge-base-en-v1.5" | "BGEBaseENV15" => EmbeddingModel::BGEBaseENV15,
            "bge-small-en-v1.5" | "BGESmallENV15" => EmbeddingModel::BGESmallENV15,
            "bge-large-en-v1.5" | "BGELargeENV15" => EmbeddingModel::BGELargeENV15,
            "nomic-embed-text-v1" | "NomicEmbedTextV1" => EmbeddingModel::NomicEmbedTextV1,
            "nomic-embed-text-v1.5" | "NomicEmbedTextV15" => EmbeddingModel::NomicEmbedTextV15,
            "multilingual-e5-small" | "MultilingualE5Small" => EmbeddingModel::MultilingualE5Small,
            "multilingual-e5-base" | "MultilingualE5Base" => EmbeddingModel::MultilingualE5Base,
            "multilingual-e5-large" | "MultilingualE5Large" => EmbeddingModel::MultilingualE5Large,
            _ => {
                return Err(EmbeddingError::ModelInit(format!(
                    "Unknown embedding model: '{}'. Supported models: all-MiniLM-L6-v2, bge-base-en-v1.5, nomic-embed-text-v1.5, etc.",
                    model_name
                )));
            }
        };
        Ok(Self::new(model))
    }

    /// Creates an embedding service from config, validating dimension matches.
    ///
    /// Returns an error if the model is unknown or if the configured dimension
    /// doesn't match the model's actual dimension.
    pub fn from_config(model_name: &str, expected_dim: usize) -> Result<Self, EmbeddingError> {
        let service = Self::from_model_str(model_name)?;
        if service.dimension != expected_dim {
            return Err(EmbeddingError::ModelInit(format!(
                "Dimension mismatch: model '{}' produces {}-dim vectors but config specifies {}",
                model_name, service.dimension, expected_dim
            )));
        }
        Ok(service)
    }

    /// Initializes the embedding model if not already done.
    ///
    /// This is called automatically on first embed call, but can be called
    /// explicitly to pre-warm the model.
    #[instrument(skip(self))]
    fn get_or_init_model(&self) -> Result<Arc<TextEmbedding>, EmbeddingError> {
        self.model
            .get_or_try_init(|| {
                info!(model = ?self.model_name, "Initializing embedding model");

                let mut options = InitOptions::new(self.model_name.clone());
                options.show_download_progress = true;
                let model = TextEmbedding::try_new(options)
                .map_err(|e| EmbeddingError::ModelInit(e.to_string()))?;

                info!(
                    model = ?self.model_name,
                    dimension = self.dimension,
                    "Embedding model initialized successfully"
                );

                Ok(Arc::new(model))
            })
            .cloned()
    }

    /// Generate embedding for text.
    ///
    /// The model is lazily loaded on first call. Subsequent calls reuse the
    /// loaded model.
    #[instrument(skip(self, text), fields(text_len = text.len()))]
    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let model = self.get_or_init_model()?;
        let text = text.to_string();

        // Run embedding generation in a blocking task since fastembed is synchronous
        let embedding = task::spawn_blocking(move || {
            model
                .embed(vec![text], None)
                .map_err(|e| EmbeddingError::Generation(e.to_string()))
        })
        .await??;

        debug!(dimension = embedding[0].len(), "Generated embedding");

        embedding
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Generation("Empty embedding result".into()).into())
    }

    /// Generate embeddings for multiple texts.
    ///
    /// More efficient than calling embed() multiple times as it batches the
    /// computation.
    #[instrument(skip(self, texts), fields(batch_size = texts.len()))]
    pub async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let model = self.get_or_init_model()?;
        let texts: Vec<String> = texts.iter().map(|s| s.to_string()).collect();

        // Run embedding generation in a blocking task
        let embeddings = task::spawn_blocking(move || {
            model
                .embed(texts, None)
                .map_err(|e| EmbeddingError::Generation(e.to_string()))
        })
        .await??;

        debug!(
            batch_size = embeddings.len(),
            dimension = embeddings.first().map(|e| e.len()).unwrap_or(0),
            "Generated batch embeddings"
        );

        Ok(embeddings)
    }

    /// Returns the embedding dimension for this model.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the model name.
    pub fn model_name(&self) -> &EmbeddingModel {
        &self.model_name
    }

    /// Pre-warms the model by loading it into memory.
    ///
    /// Call this during application startup if you want to avoid latency
    /// on the first embedding request.
    #[instrument(skip(self))]
    pub async fn warmup(&self) -> anyhow::Result<()> {
        let model_name = self.model_name.clone();

        // Clone self's OnceCell reference for the closure
        let model_cell = &self.model;

        if model_cell.get().is_some() {
            debug!("Model already initialized, skipping warmup");
            return Ok(());
        }

        info!(model = ?model_name, "Warming up embedding model");

        // Initialize model
        self.get_or_init_model()?;

        info!(model = ?model_name, "Model warmup complete");
        Ok(())
    }
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new(EmbeddingModel::AllMiniLML6V2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test - doesn't load model
    #[tokio::test]
    async fn test_embedding_dimension() {
        let service = EmbeddingService::default();
        assert_eq!(service.dimension(), 384);
    }

    // Unit test - validates model name parsing
    #[test]
    fn test_from_model_str() {
        assert!(EmbeddingService::from_model_str("all-MiniLM-L6-v2").is_ok());
        assert!(EmbeddingService::from_model_str("unknown-model").is_err());
    }

    // Unit test - validates dimension matching
    #[test]
    fn test_from_config_dimension_mismatch() {
        // MiniLM is 384-dim, so 512 should fail
        let result = EmbeddingService::from_config("all-MiniLM-L6-v2", 512);
        assert!(result.is_err());

        // Correct dimension should work
        let result = EmbeddingService::from_config("all-MiniLM-L6-v2", 384);
        assert!(result.is_ok());
    }

    // Integration test - downloads model, run with: cargo test --ignored
    #[tokio::test]
    #[ignore = "Downloads model from network, slow"]
    async fn test_embed_single() {
        let service = EmbeddingService::default();
        let text = "Hello, world!";

        let embedding = service.embed(text).await.unwrap();

        assert_eq!(embedding.len(), 384);
        // Verify it's not all zeros (real embedding)
        assert!(embedding.iter().any(|&x| x != 0.0));
    }

    // Integration test - downloads model, run with: cargo test --ignored
    #[tokio::test]
    #[ignore = "Downloads model from network, slow"]
    async fn test_embed_batch() {
        let service = EmbeddingService::default();
        let texts = vec!["Hello", "World", "Test"];

        let embeddings = service.embed_batch(&texts).await.unwrap();

        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), 384);
            assert!(emb.iter().any(|&x| x != 0.0));
        }
    }

    // Integration test - run with: cargo test --ignored
    #[tokio::test]
    #[ignore = "Downloads model from network, slow"]
    async fn test_embed_batch_empty() {
        let service = EmbeddingService::default();
        let texts: Vec<&str> = vec![];

        let embeddings = service.embed_batch(&texts).await.unwrap();

        assert!(embeddings.is_empty());
    }

    // Integration test - downloads model, run with: cargo test --ignored
    #[tokio::test]
    #[ignore = "Downloads model from network, slow"]
    async fn test_similar_texts_have_similar_embeddings() {
        let service = EmbeddingService::default();

        let emb1 = service.embed("The cat sat on the mat").await.unwrap();
        let emb2 = service.embed("A cat is sitting on a mat").await.unwrap();
        let emb3 = service.embed("Quantum physics is complex").await.unwrap();

        // Compute cosine similarity
        fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
            let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (norm_a * norm_b)
        }

        let sim_similar = cosine_similarity(&emb1, &emb2);
        let sim_different = cosine_similarity(&emb1, &emb3);

        // Similar sentences should have higher similarity
        assert!(
            sim_similar > sim_different,
            "Similar texts should have higher cosine similarity: {} vs {}",
            sim_similar,
            sim_different
        );
    }
}
