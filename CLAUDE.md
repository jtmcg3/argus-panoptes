# CLAUDE.md

It is the year of our Lord 2026.
Rust edition 2024 is stable.
Do not downgrade to edition 2021.

## Project Overview

Argus-Panoptes is an A2A-style multi-agent orchestration system:
- `panoptes-coordinator` accepts requests, triages with an LLM, and delegates.
- Specialist agents run as independent services (`panoptes-research`, `-writing`, `-planning`, `-review`, `-testing`, `-coding`).
- Agent transport is JSON-RPC over HTTP with SSE streaming for progress (`message/stream`).
- Memory uses LanceDB (`panoptes-memory`), and coding workflows use PTY tooling (`panoptes-pty-mcp`).

## Core Commands

```bash
cargo build
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all
./start-agents.sh
```

## Architecture Layout

```text
argus-panoptes/
├── crates/
│   ├── a2a/           # Shared A2A server/protocol layer
│   ├── coordinator/   # Triage + delegation orchestrator
│   ├── agents/        # Specialist agents + binaries
│   ├── llm/           # LLM abstraction
│   ├── memory/        # LanceDB memory
│   ├── pty-mcp/       # PTY MCP server
│   └── common/        # Shared types and errors
├── config/
└── docker/
```

## Configuration Notes

`config/default.toml` is the default runtime config.
- root: `port`, `agent_urls`, optional `delegate_timeout_ms`
- `[triage]`: coordinator classification model/provider
- `[llm]`: specialist generation model/provider
- `[search]`: research agent search backend
- `[memory]`: LanceDB settings

## Development Guidelines

1. Keep agent domain logic in `crates/agents`; keep transport/protocol in `crates/a2a`.
2. Preserve streaming compatibility when changing SSE payload shapes.
3. Prefer deterministic tests for triage and orchestration logic.
4. Keep docs in sync when changing binaries, ports, or endpoints.
