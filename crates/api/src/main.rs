//! Panoptes API server binary.
//!
//! Usage:
//!   panoptes-api --config config.toml
//!   panoptes-api --port 8080
//!   panoptes-api --port 8080 --bind 0.0.0.0
//!   panoptes-api --port 8080 --memory  # Enable memory persistence
//!
//! # Environment Variables
//!
//! - `PANOPTES_API_KEY` - API authentication key (recommended)
//! - `PANOPTES_BIND_ADDR` - Server bind address (default: 127.0.0.1)
//! - `PANOPTES_CORS_ORIGINS` - CORS allowed origins (comma-separated)
//! - `OPENAI_API_KEY` - OpenAI API key for ZeroClaw triage

use panoptes_api::{ApiKeyConfig, AppState, default_memory_config, serve};
use panoptes_coordinator::CoordinatorConfig;
use panoptes_memory::MemoryConfig;
use std::net::SocketAddr;
use std::path::PathBuf;
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
    let mut enable_memory = false;
    let mut memory_path: Option<PathBuf> = None;
    let mut bind_addr: Option<String> = None;

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
            "--bind" | "-b" => {
                if i + 1 < args.len() {
                    bind_addr = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--memory" | "-m" => {
                enable_memory = true;
            }
            "--memory-path" => {
                if i + 1 < args.len() {
                    memory_path = Some(PathBuf::from(&args[i + 1]));
                    enable_memory = true;
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("Panoptes API Server");
                println!();
                println!("Usage: panoptes-api [OPTIONS]");
                println!();
                println!("Options:");
                println!("  -p, --port <PORT>        Port to listen on (default: 8080)");
                println!(
                    "  -b, --bind <ADDR>        Bind address (default: 127.0.0.1, env: PANOPTES_BIND_ADDR)"
                );
                println!("  -c, --config <FILE>      Path to config.toml file");
                println!("  -m, --memory             Enable memory persistence for research");
                println!(
                    "      --memory-path <DIR>  Path to LanceDB database (default: ./data/memory)"
                );
                println!("  -h, --help               Show this help message");
                println!();
                println!("Environment variables:");
                println!(
                    "  PANOPTES_API_KEY         API authentication key (recommended for production)"
                );
                println!(
                    "  PANOPTES_BIND_ADDR       Server bind address (overridden by --bind flag)"
                );
                println!("  PANOPTES_CORS_ORIGINS    CORS allowed origins (comma-separated)");
                println!("  OPENAI_API_KEY           OpenAI API key for ZeroClaw triage");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    // SEC-006: Determine bind address (CLI flag > env var > default 127.0.0.1)
    let host = bind_addr
        .or_else(|| std::env::var("PANOPTES_BIND_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    if host == "0.0.0.0" {
        tracing::warn!(
            "Server binding to 0.0.0.0 — this exposes the API to all network interfaces. \
             Ensure authentication is configured (PANOPTES_API_KEY) and a firewall is in place."
        );
    }

    // SEC-011: Read API key from environment
    let api_key = std::env::var("PANOPTES_API_KEY").ok();
    if api_key.is_none() {
        tracing::warn!(
            "PANOPTES_API_KEY not set — API will run without authentication. \
             This is acceptable for local development but NOT for production. \
             Set PANOPTES_API_KEY to enable bearer token authentication."
        );
    }

    // SEC-009b: Read CORS origins from environment
    let cors_origins: Option<Vec<String>> = std::env::var("PANOPTES_CORS_ORIGINS")
        .ok()
        .map(|s| s.split(',').map(|o| o.trim().to_string()).collect());

    // Load coordinator configuration
    let config = if let Some(path) = config_path {
        tracing::info!(path = %path, "Loading configuration");
        CoordinatorConfig::from_file(&path)?
    } else {
        tracing::info!("Using default configuration");
        CoordinatorConfig::default()
    };

    // Create application state
    let mut state = if enable_memory {
        let memory_config = if let Some(path) = memory_path {
            MemoryConfig {
                db_path: path,
                ..Default::default()
            }
        } else {
            default_memory_config()
        };

        tracing::info!(
            db_path = %memory_config.db_path.display(),
            "Initializing with memory enabled"
        );

        AppState::with_memory(config, memory_config).await?
    } else {
        tracing::info!("Starting without memory persistence");
        AppState::new(config)?
    };

    // SEC-011: Apply API key configuration
    if let Some(key) = api_key {
        state = state.with_api_key(ApiKeyConfig::new(key));
        tracing::info!("API key authentication enabled");
    }

    // Start server
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    serve(Arc::new(state), addr, cors_origins).await?;

    Ok(())
}
