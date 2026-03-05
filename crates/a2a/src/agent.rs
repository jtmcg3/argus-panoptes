//! A2aAgent trait — the bridge between existing agent logic and the A2A protocol.

use crate::types::{AgentSkill, ProgressEvent};
use async_trait::async_trait;
use panoptes_common::PanoptesError;
use tokio::sync::broadcast;

/// The bridge trait. Each specialist agent implements this to
/// expose itself as an A2A server.
#[async_trait]
pub trait A2aAgent: Send + Sync + 'static {
    /// Agent identity.
    fn name(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str;

    /// Version string.
    fn version(&self) -> &str {
        "0.1.0"
    }

    /// Skills (maps from AgentCapability).
    fn skills(&self) -> Vec<AgentSkill>;

    /// Whether streaming is supported.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Handle a task. Emit progress via the sender if streaming.
    ///
    /// Returns the result text on success.
    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError>;
}
