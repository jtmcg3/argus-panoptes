# Argus-Panoptes

Argus-Panoptes is a Rust multi-agent system built around A2A-style JSON-RPC + SSE.
Each specialist agent runs as its own process, and a coordinator triages and delegates work.

## Architecture

- `panoptes-coordinator` (`:18080`) is the entrypoint.
- Specialist agents run independently:
  - `panoptes-research` (`:9001`)
  - `panoptes-writing` (`:9002`)
  - `panoptes-planning` (`:9003`)
  - `panoptes-review` (`:9004`)
  - `panoptes-testing` (`:9005`)
  - `panoptes-coding` (`:9006`)
- Optional chat bridge:
  - `panoptes-telegram` (Telegram Bot API long-polling -> coordinator)
- Shared services:
  - `panoptes-llm` abstraction (OpenAI-compatible + Anthropic)
  - `panoptes-memory` (LanceDB + fastembed)
  - `panoptes-pty-mcp` (PTY tool server used by coding flows)

Each agent serves:
- `GET /.well-known/agent.json`
- `POST /` (JSON-RPC: `message/send`, `message/stream`, `tasks/get`, `tasks/cancel`)
- `GET /health`

## Workspace Crates

- `crates/a2a` - shared A2A server/protocol layer
- `crates/coordinator` - triage + delegation orchestrator
- `crates/agents` - specialist agent implementations + binaries
- `crates/llm` - LLM client abstraction
- `crates/memory` - LanceDB memory store
- `crates/pty-mcp` - PTY MCP server
- `crates/common` - shared types/errors

## Prerequisites

- Rust 1.87+ (edition 2024)
- Ollama or another OpenAI-compatible endpoint (optional but recommended)
- SearXNG (optional, for richer research agent results)

## Build and Test

```bash
cargo build
cargo test --workspace
```

## Configuration

Main config is `config/default.toml` (or set `PANOPTES_CONFIG`).

Key sections:
- root: `port`, `agent_urls`, optional `delegate_timeout_ms`
- `[triage]` coordinator classifier model/provider settings
- `[llm]` specialist agent model/provider settings
- `[search]` research search endpoint
- `[memory]` shared memory settings
- `[telegram]` optional Telegram bridge settings

See `config/config.example.toml` for a complete template.

## Run Locally

```bash
# Build all binaries and start coordinator + all agents
./start-agents.sh
```

Or run binaries individually:

```bash
cargo run --bin panoptes-coordinator
cargo run --bin panoptes-research
```

Run Telegram bridge (optional):

```bash
export TELEGRAM_BOT_TOKEN="123456:ABCDEF..."
cargo run --bin panoptes-telegram
```

## Quick Health Checks

```bash
curl http://localhost:18080/health
curl http://localhost:9001/.well-known/agent.json
```

## Scheduled Research (Cron)

Use the helper:

```bash
cp scripts/research-topics.txt.example scripts/research-topics.txt
./scripts/research-cron.sh scripts/research-topics.txt
```

Typical cron entry (every 6 hours):

```bash
0 */6 * * * cd /home/jim/projects/argus-panoptes && HOST=localhost PORT=9001 ./scripts/research-cron.sh ./scripts/research-topics.txt >> ./logs/research/cron.log 2>&1
```

## Docker

```bash
# Production-like
docker-compose -f docker/docker-compose.yml up -d

# Development
docker-compose -f docker/docker-compose.yml -f docker/docker-compose.dev.yml up
```

## License

MIT
