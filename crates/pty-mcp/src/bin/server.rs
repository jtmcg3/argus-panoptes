//! PTY-MCP server binary.
//!
//! Run this as a subprocess or standalone service to expose PTY
//! capabilities via MCP.

use panoptes_pty_mcp::PtyMcpServer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,panoptes_pty_mcp=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    tracing::info!("Starting PTY-MCP server");

    let _server = PtyMcpServer::new();

    // TODO: Implement actual MCP transport
    // For stdio transport:
    // server.serve((std::io::stdin(), std::io::stdout())).await?;
    //
    // For HTTP transport:
    // let listener = TcpListener::bind("127.0.0.1:3001").await?;
    // server.serve_http(listener).await?;

    tracing::info!("PTY-MCP server ready (transport not yet implemented)");

    // Keep running
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutting down PTY-MCP server");
    Ok(())
}
