//! PTY-MCP server binary.
//!
//! Run this as a subprocess or standalone service to expose PTY
//! capabilities via MCP.
//!
//! # Usage
//!
//! The server communicates over stdio using the MCP protocol:
//!
//! ```bash
//! cargo run --bin pty-mcp-server
//! ```
//!
//! Or configure it as an MCP server in your client's configuration.

use panoptes_pty_mcp::PtyMcpServer;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging to stderr (stdout is used for MCP communication)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,panoptes_pty_mcp=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    tracing::info!("Starting PTY-MCP server");

    let server = PtyMcpServer::new();

    // Create stdio transport
    let transport = stdio();

    // Serve the MCP server over stdio
    let running = server.serve(transport).await?;

    tracing::info!("PTY-MCP server ready and accepting requests");

    // Wait for the server to complete (client disconnects or ctrl+c)
    let quit_reason = running.waiting().await?;

    tracing::info!(?quit_reason, "PTY-MCP server shutting down");

    Ok(())
}
