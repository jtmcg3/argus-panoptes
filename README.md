# Argus-Panoptes

> The hundred-eyed guardian -- A multi-agent orchestration system built on ZeroClaw and swarms-rs.

Argus-Panoptes is a Rust-native AI agent swarm that coordinates specialist agents for coding, research, writing, planning, code review, and testing. It uses ZeroClaw for triage/routing, swarms-rs for orchestration, LanceDB for vector memory, and an LLM abstraction layer supporting Ollama and OpenAI-compatible providers.

## Architecture

```
                          ARGUS-PANOPTES

  User Request
       |
       v
  +-----------+       +-----------+
  |  API      | ----> | ZeroClaw  |
  |  Gateway  |       | Triage    |
  +-----------+       +-----------+
       |                    |
       |         MCP (Model Context Protocol)
       |                    |
       v                    v
  +------+------+------+------+------+------+
  | Code | Rsrch| Write| Plan | Revw | Test |
  +------+------+------+------+------+------+
       |              |              |
       v              v              v
  +-----------+  +-----------+  +-----------+
  |  LLM      |  |  Memory   |  |  PTY-MCP  |
  |  Client   |  |  (LanceDB)|  |  Server   |
  +-----------+  +-----------+  +-----------+
```

## Crates

| Crate | Description |
|-------|-------------|
| `panoptes-api` | REST/WebSocket API gateway with auth, rate limiting, CORS |
| `panoptes-coordinator` | ZeroClaw triage and workflow orchestration |
| `panoptes-agents` | Specialist agents (coding, research, writing, planning, review, testing) |
| `panoptes-llm` | LLM client abstraction (Anthropic, OpenAI/Ollama) with retry + semaphore |
| `panoptes-memory` | LanceDB dual-layer memory with fastembed embeddings |
| `panoptes-pty-mcp` | MCP server exposing PTY sessions for Claude CLI |
| `panoptes-common` | Shared types and error handling |

## Quick Start

### Prerequisites

- Rust 1.87+ (2024 edition)
- Ollama (for local LLM inference)
- Docker (optional, for containerized deployment)

### Build and Test

```bash
git clone https://github.com/jtmcg3/argus-panoptes
cd argus-panoptes
cargo build
cargo test
```

### Configure

The API server auto-detects `config/default.toml`. Key sections:

```toml
[provider]
provider_type = "openai"        # "openai" works for Ollama too
model = "lfm2:24b"
api_url = "http://localhost:11434"

[llm]
provider = "openai"
model = "lfm2:24b"
api_url = "http://localhost:11434"
max_concurrent_requests = 2

[memory]
db_path = "./data/memory"
embedding_model = "all-MiniLM-L6-v2"
```

### Run

```bash
# Start Ollama
ollama serve
ollama pull lfm2:24b    # or any compatible model

# Run the API server
cargo run --bin panoptes-api

# Or with options
cargo run --bin panoptes-api -- --port 8080 --memory
```

### Docker

```bash
# Development
docker-compose -f docker/docker-compose.yml -f docker/docker-compose.dev.yml up

# Production
docker-compose -f docker/docker-compose.yml up -d
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check (no auth required) |
| `POST` | `/api/v1/messages` | Coordinator triage + routing |
| `GET` | `/api/v1/sessions/:id` | PTY session status |
| `WS` | `/api/v1/ws` | WebSocket streaming |
| `POST` | `/api/v1/{coding,research,writing,planning,review,testing}` | Direct agent access |
| `POST` | `/api/v1/workflow` | Multi-agent workflows |

## Agents

- **Coding** -- PTY-MCP sessions with Claude CLI, persistent terminals, y/N handling
- **Research** -- Web search, query decomposition, source synthesis
- **Writing** -- Docs, email, reports with tone adaptation
- **Planning** -- Goal breakdown, dependency tracking, progress monitoring
- **Review** -- Security scanning, style checking, improvement suggestions
- **Testing** -- Test suite execution, case generation, coverage analysis

All agents use the LLM client for content generation with template fallback when no LLM is configured.

## Memory

Dual-layer architecture with fastembed local embeddings:

- **Working memory** -- Recent messages, active task context, session state
- **Persistent memory (LanceDB)** -- Vector embeddings for semantic search, full-text search, hybrid retrieval (70% vector / 30% keyword)

Three memory types: Episodic (events), Semantic (facts), Procedural (patterns).

## Security

| Feature | Description |
|---------|-------------|
| API key auth | `PANOPTES_API_KEY` env var, bearer token on all endpoints except `/health` |
| Rate limiting | 100 req/min, 50 concurrent connections, 10 WebSocket connections per IP |
| CORS | Localhost-only default; configurable via `PANOPTES_CORS_ORIGINS` |
| Network binding | `127.0.0.1` default; configurable via `PANOPTES_BIND_ADDR` |
| Working dir validation | `allowed_base_dirs` config restricts agent filesystem access |
| Permission modes | Plan (confirm changes) or Act (proceed directly, requires auth) |
| PTY command whitelist | Only allowed commands; dangerous flags blocked |
| Docker hardening | Read-only FS, dropped caps, no-new-privileges |
| Session limits | 32 concurrent PTY sessions, 1-hour TTL |

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `PANOPTES_API_KEY` | None (unauthenticated) | API authentication |
| `PANOPTES_BIND_ADDR` | `127.0.0.1` | Server bind address |
| `PANOPTES_CORS_ORIGINS` | localhost only | CORS allowed origins |
| `OPENAI_API_KEY` | None | OpenAI API key for ZeroClaw triage |

## Related Projects

- [Argus](https://github.com/jtmcg3/argus) -- Original PTY-based Claude CLI wrapper
- [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) -- Ultra-lightweight AI agent framework
- [swarms-rs](https://github.com/The-Swarm-Corporation/swarms-rs) -- Rust multi-agent orchestration

## License

MIT
