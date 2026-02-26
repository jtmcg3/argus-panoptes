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
use panoptes_llm::LlmConfig;
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
    let mut no_llm = false;

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
            "--no-llm" => {
                no_llm = true;
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
                println!(
                    "      --no-llm             Disable LLM even if [llm] section is in config"
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

    // Load coordinator configuration and optional LLM config
    let resolved_config_path = config_path.or_else(|| {
        let default = "config/default.toml";
        if std::path::Path::new(default).exists() {
            tracing::info!(path = default, "Found default config file");
            Some(default.to_string())
        } else {
            None
        }
    });

    let (config, llm_config, search_url) = if let Some(ref path) = resolved_config_path {
        tracing::info!(path = %path, "Loading configuration");
        let coordinator = CoordinatorConfig::from_file(path)?;

        // Parse [llm] section from the same file
        let llm = if no_llm {
            tracing::info!("LLM disabled via --no-llm flag");
            None
        } else {
            let raw = std::fs::read_to_string(path)?;
            let table: toml::Table = toml::from_str(&raw)?;
            if let Some(llm_value) = table.get("llm") {
                match llm_value.clone().try_into::<LlmConfig>() {
                    Ok(llm_cfg) => {
                        tracing::info!(
                            provider = %llm_cfg.provider,
                            model = %llm_cfg.model,
                            "LLM configuration loaded"
                        );
                        Some(llm_cfg)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to parse [llm] config, continuing without LLM");
                        None
                    }
                }
            } else {
                tracing::info!("No [llm] section in config, running in template-only mode");
                None
            }
        };

        // Parse optional [search] section
        let search_url = {
            let raw = std::fs::read_to_string(path)?;
            let table: toml::Table = toml::from_str(&raw)?;
            table
                .get("search")
                .and_then(|s| s.get("url"))
                .and_then(|v| v.as_str())
                .map(String::from)
        };
        if let Some(ref url) = search_url {
            tracing::info!(url = %url, "SearXNG search configured");
        }

        (coordinator, llm, search_url)
    } else {
        tracing::info!("Using default configuration (no config file)");
        (CoordinatorConfig::default(), None, None)
    };

    // Build memory config: --memory-path flag > config file [memory] section > default
    let memory_config = if enable_memory {
        let mem_cfg = if let Some(path) = memory_path {
            MemoryConfig {
                db_path: path,
                ..Default::default()
            }
        } else if resolved_config_path.is_some() {
            // Map coordinator's [memory] fields to panoptes_memory::MemoryConfig
            MemoryConfig {
                db_path: config.memory.db_path.clone(),
                embedding_model: config.memory.embedding_model.clone(),
                ..Default::default()
            }
        } else {
            default_memory_config()
        };
        Some(mem_cfg)
    } else if llm_config.is_some() && resolved_config_path.is_some() {
        // When LLM is enabled via config, also enable memory from config
        tracing::info!("Enabling memory (implied by LLM config)");
        Some(MemoryConfig {
            db_path: config.memory.db_path.clone(),
            embedding_model: config.memory.embedding_model.clone(),
            ..Default::default()
        })
    } else {
        None
    };

    // Create application state
    let mut state = if let Some(llm_cfg) = llm_config {
        if let Some(mem_cfg) = memory_config {
            tracing::info!(
                db_path = %mem_cfg.db_path.display(),
                "Initializing with LLM + memory"
            );
            AppState::with_llm(config, llm_cfg, Some(mem_cfg), search_url).await?
        } else {
            tracing::info!("Initializing with LLM (no memory)");
            AppState::with_llm(config, llm_cfg, None, search_url).await?
        }
    } else if let Some(mem_cfg) = memory_config {
        tracing::info!(
            db_path = %mem_cfg.db_path.display(),
            "Initializing with memory enabled"
        );
        AppState::with_memory(config, mem_cfg, search_url).await?
    } else {
        tracing::info!("Starting in template-only mode (no LLM, no memory)");
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
