# CLAUDE.md

This file provides guidance to Claude Code when working with this repository.

## Project Overview

Argus-Panoptes is a Rust multi-agent orchestration system that combines:
- **ZeroClaw** for intelligent triage and routing
- **swarms-rs** for specialist agent coordination
- **LanceDB** for dual-layer memory (working + persistent)
- **MCP** (Model Context Protocol) for tool integration

The system coordinates multiple AI agents to handle complex tasks across coding, research, writing, planning, and more.

## Build Commands

```bash
cargo build              # Build all crates
cargo build -p panoptes-coordinator  # Build specific crate
cargo run --bin pty-mcp-server       # Run PTY-MCP server
cargo test               # Run all tests
cargo test -p panoptes-memory        # Test specific crate
cargo clippy             # Run linter
cargo fmt                # Format code
```

## Architecture

```
argus-panoptes/
├── crates/
│   ├── coordinator/     # ZeroClaw-based triage (lib + future bin)
│   ├── pty-mcp/         # MCP server for PTY sessions
│   ├── agents/          # swarms-rs specialist agents
│   ├── memory/          # LanceDB integration
│   └── common/          # Shared types
├── docker/              # Container deployment
└── config/              # Configuration files
```

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

1. **Agents are stateless** — All state lives in memory crate or external services
2. **MCP for tool calls** — Agents communicate via MCP, not direct function calls
3. **Async-first** — Use tokio async throughout
4. **Trait-based** — Follow ZeroClaw's pattern of trait-defined subsystems

## Dependencies

Key external dependencies:
- `zeroclaw` — Local fork at `../zeroclaw`
- `swarms-rs` — Multi-agent orchestration
- `rmcp` — Rust MCP SDK
- `lancedb` — Vector database
- `portable-pty` — PTY management (from argus)

## TODOs

Current scaffold has placeholders marked with `// TODO:`. Key integration points:
1. ZeroClaw triage integration in `coordinator/src/triage.rs`
2. MCP ServerHandler impl in `pty-mcp/src/server.rs`
3. LanceDB connection in `memory/src/store.rs`
4. swarms-rs workflow integration in `agents/`
