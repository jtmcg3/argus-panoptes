# CLAUDE.md

It is the year of our Lord 2026.
2024 is a stable edition of Rust.
DO NOT REVERT to the 2021 edition.
Please update your knowledge before using new packages.
This file provides guidance to Claude Code when working with this repository.

## Project Overview

Argus-Panoptes is a Rust multi-agent orchestration system that combines:
- **ZeroClaw** for intelligent triage and routing (with keyword fallback when no API key)
- **swarms-rs** for specialist agent coordination
- **LanceDB** for dual-layer memory (working + persistent) with fastembed embeddings
- **MCP** (Model Context Protocol) for tool integration
- **LLM abstraction** supporting Anthropic and OpenAI/Ollama providers

The system coordinates multiple AI agents to handle tasks across coding, research, writing, planning, code review, and testing.

## Build Commands

```bash
cargo build                          # Build all crates
cargo build -p panoptes-coordinator  # Build specific crate
cargo build --release                # Optimized build (LTO, stripped)
cargo run --bin panoptes-api         # Run API server
cargo run --bin pty-mcp-server       # Run PTY-MCP server
cargo test                           # Run all tests (258 passing)
cargo test -p panoptes-memory        # Test specific crate
cargo clippy                         # Run linter (clean)
cargo fmt                            # Format code
```

## Architecture

```
argus-panoptes/
├── crates/
│   ├── api/             # REST/WebSocket API gateway (binary: panoptes-api)
│   ├── coordinator/     # ZeroClaw triage + workflow orchestration
│   ├── agents/          # swarms-rs specialist agents (6 types)
│   ├── llm/             # LLM client abstraction (Anthropic, OpenAI/Ollama)
│   ├── memory/          # LanceDB dual-layer memory + fastembed
│   ├── pty-mcp/         # MCP server for PTY sessions
│   └── common/          # Shared types and error handling
├── config/              # Configuration (default.toml)
└── docker/              # Container deployment
```

## Config

`config/default.toml` is auto-detected by the API server. Sections:
- `[provider]` -- ZeroClaw triage provider settings
- `[llm]` -- LLM client config (provider, model, api_url, retry, concurrency)
- `[memory]` -- LanceDB path, embedding model, token limits

## Key Patterns

### Agent Trait
All agents implement `panoptes_agents::Agent`:
```rust
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> &[AgentCapability];
    async fn process_task(&self, task: &Task) -> Result<AgentMessage>;
}
```

### LLM Client Trait
LLM providers implement `panoptes_llm::LlmClient`:
```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
    fn model_name(&self) -> &str;
}
```
Implementations: `OpenAiClient` (also used for Ollama), `AnthropicClient`. Wrapped in `RetryingClient` and `SemaphoredClient` for production use.

### Routing Decision
The coordinator routes requests via `AgentRoute`:
```rust
pub enum AgentRoute {
    PtyCoding { instruction, working_dir, permission_mode },
    Research { query, sources },
    Writing { task_type, context },
    Planning { scope, context },
    CodeReview { target, review_type },
    Testing { target, test_type },
    Direct { response },
    Workflow { tasks },
}
```

### Memory Types
```rust
pub enum MemoryType {
    Episodic,    // Events, conversations
    Semantic,    // Facts, knowledge
    Procedural,  // Patterns, preferences
}
```

## Development Guidelines

1. **Agents are stateless** -- All state lives in memory crate or external services
2. **MCP for tool calls** -- Agents communicate via MCP, not direct function calls
3. **Async-first** -- Use tokio async throughout
4. **Trait-based** -- Follow ZeroClaw's pattern of trait-defined subsystems
5. **LLM with fallback** -- Agents use LLM for content generation; template fallback when unconfigured

## Dependencies

Key external dependencies:
- `zeroclaw` -- From git (https://github.com/zeroclaw-labs/zeroclaw.git, main branch)
- `swarms-rs` -- Multi-agent orchestration
- `rmcp` -- Rust MCP SDK
- `lancedb` -- Vector database
- `fastembed` -- Local embedding model
- `portable-pty` -- PTY management
- `reqwest` / `scraper` -- HTTP + HTML parsing for research agent
