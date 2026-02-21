//! Error types for Argus-Panoptes.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PanoptesError {
    #[error("Agent error: {0}")]
    Agent(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Triage error: {0}")]
    Triage(String),

    #[error("PTY error: {0}")]
    Pty(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, PanoptesError>;
