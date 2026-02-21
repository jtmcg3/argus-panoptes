//! Panoptes API server binary.
//!
//! Usage:
//!   panoptes-api --config config.toml
//!   panoptes-api --port 8080

use panoptes_api::{serve, AppState};
use panoptes_coordinator::CoordinatorConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,panoptes_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse command line arguments (simple for now)
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 8080;
    let mut config_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().expect("Invalid port number");
                    i += 1;
                }
            }
            "--config" | "-c" => {
                if i + 1 < args.len() {
                    config_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("Panoptes API Server");
                println!();
                println!("Usage: panoptes-api [OPTIONS]");
                println!();
                println!("Options:");
                println!("  -p, --port <PORT>      Port to listen on (default: 8080)");
                println!("  -c, --config <FILE>    Path to config.toml file");
                println!("  -h, --help             Show this help message");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    // Load configuration
    let config = if let Some(path) = config_path {
        tracing::info!(path = %path, "Loading configuration");
        CoordinatorConfig::from_file(&path)?
    } else {
        tracing::info!("Using default configuration");
        CoordinatorConfig::default()
    };

    // Create application state
    let state = Arc::new(AppState::new(config)?);

    // Start server
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    serve(state, addr).await?;

    Ok(())
}
