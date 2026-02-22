# Argus-Panoptes

> The hundred-eyed guardian — A multi-agent orchestration system built on ZeroClaw and swarms-rs.

Argus-Panoptes is a Rust-native AI agent swarm platform that coordinates specialized agents for coding, research, writing, planning, and more. It uses ZeroClaw for intelligent triage and routing, swarms-rs for agent orchestration, and LanceDB for long-term memory.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         ARGUS-PANOPTES                                  │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  User Request                                                           │
│       │                                                                 │
│       ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                  ZEROCLAW COORDINATOR                           │   │
│  │  - Triage & intent classification                               │   │
│  │  - Agent routing decisions                                       │   │
│  │  - Workflow orchestration                                        │   │
│  │  - Multi-turn memory context                                     │   │
│  └───────────────────────────┬─────────────────────────────────────┘   │
│                              │                                          │
│              MCP (Model Context Protocol)                               │
│                              │                                          │
│  ┌───────────┬───────────┬───┴───┬───────────┬───────────┐            │
│  ▼           ▼           ▼       ▼           ▼           ▼            │
│ ┌─────┐   ┌─────┐   ┌─────┐   ┌─────┐   ┌─────┐   ┌─────┐           │
│ │Code │   │Rsrch│   │Write│   │Plan │   │Revw │   │Test │           │
│ │Agent│   │Agent│   │Agent│   │Agent│   │Agent│   │Agent│           │
│ └──┬──┘   └──┬──┘   └──┬──┘   └──┬──┘   └──┬──┘   └──┬──┘           │
│    │         │         │         │         │         │               │
│    │         │         │         │         │         │               │
│    └─────────┴─────────┴────┬────┴─────────┴─────────┘               │
│                             │                                         │
│                    ┌────────┴────────┐                                │
│                    │  SHARED MEMORY  │                                │
│                    │    (LanceDB)    │                                │
│                    │                 │                                │
│                    │  - Episodic     │                                │
│                    │  - Semantic     │                                │
│                    │  - Procedural   │                                │
│                    └─────────────────┘                                │
│                                                                        │
└────────────────────────────────────────────────────────────────────────┘
```

## Components

| Crate | Description |
|-------|-------------|
| `panoptes-coordinator` | ZeroClaw-based triage and orchestration |
| `panoptes-pty-mcp` | MCP server exposing PTY for Claude CLI |
| `panoptes-agents` | swarms-rs specialist agents |
| `panoptes-memory` | LanceDB-backed dual-layer memory |
| `panoptes-common` | Shared types and utilities |

## Quick Start

### Prerequisites

- Rust 1.87+
- Ollama (for local LLM inference)
- Docker (optional, for containerized deployment)

### Build

```bash
# Clone with ZeroClaw fork
git clone https://github.com/jtmcg3/argus-panoptes
cd argus-panoptes

# Build all crates
cargo build

# Run tests
cargo test
```

### Configuration

```bash
# Copy example config
cp config/config.example.toml config/config.toml

# Edit with your settings
vim config/config.toml
```

### Run

```bash
# Start Ollama (if not running)
ollama serve

# Pull required model
ollama pull llama3.2

# Run the coordinator
cargo run --bin panoptes-coordinator

# In another terminal, run PTY-MCP server
cargo run --bin pty-mcp-server
```

### Docker

```bash
# Development mode (with hot reload)
docker-compose -f docker/docker-compose.yml -f docker/docker-compose.dev.yml up

# Production mode
docker-compose -f docker/docker-compose.yml up -d
```

## Agents

### Coding Agent
Uses PTY-MCP to run Claude CLI for code generation and modification.
- Spawns persistent PTY sessions
- Handles y/N confirmations
- Streams output in real-time

### Research Agent
Web search and knowledge gathering.
- Query decomposition
- Source synthesis
- Citation tracking

### Writing Agent
Document and content creation.
- Multiple formats (docs, email, reports)
- Tone adaptation
- Edit and refine

### Planning Agent
Task management and scheduling.
- Goal breakdown
- Dependency tracking
- Progress monitoring

### Review Agent
Code review and quality analysis.
- Security scanning
- Style checking
- Improvement suggestions

### Testing Agent
Test execution and coverage.
- Run test suites
- Generate test cases
- Coverage analysis

## Memory System

Dual-layer architecture:

**Hot Path (Working Memory)**
- Recent messages
- Active task context
- Session state

**Cold Path (LanceDB)**
- Vector embeddings for semantic search
- Full-text search
- Hybrid retrieval (70% vector, 30% keyword)

## Protocols

- **MCP (Model Context Protocol)**: Tool integration between coordinator and agents
- **A2A (Agent-to-Agent)**: Inter-agent communication (future)

## Security

Argus-Panoptes includes multiple layers of security hardening for safe deployment.

### Authentication

Set `PANOPTES_API_KEY` to enable bearer token authentication on all API endpoints (except `/health`):

```bash
export PANOPTES_API_KEY="your-secret-key-here"
```

Requests must include the header: `Authorization: Bearer <key>`. Without `PANOPTES_API_KEY`, the server runs unauthenticated (suitable for local development only).

### Network Binding

The server binds to `127.0.0.1` (localhost only) by default. To expose externally:

```bash
# Via environment variable
export PANOPTES_BIND_ADDR=0.0.0.0

# Via CLI flag (overrides env var)
panoptes-api --bind 0.0.0.0
```

A warning is logged when binding to `0.0.0.0`. Always use authentication and a firewall when exposing to the network.

### CORS

CORS is restricted to localhost origins by default. Configure with:

```bash
# Specific origins (comma-separated)
export PANOPTES_CORS_ORIGINS="https://app.example.com,https://admin.example.com"

# Wildcard (development only)
export PANOPTES_CORS_ORIGINS="*"
```

### Docker

Production Docker containers are hardened with:
- `read_only: true` filesystem
- `cap_drop: ALL` (only `SYS_PTRACE` added for PTY service)
- `security_opt: no-new-privileges:true`
- No Docker socket mount
- No privileged mode
- `tmpfs` for `/tmp` and `/run`

### Working Directory Restrictions

The `allowed_base_dirs` configuration restricts which directories agents can operate in:

```toml
# In config.toml
allowed_base_dirs = ["/home/user/projects", "/opt/workspace"]
```

An empty list (default) allows all directories. Paths containing `..` are always rejected.

### Permission Modes

- **Plan** (default): Agents ask for confirmation before making changes
- **Act**: Agents proceed without confirmation (only available via authenticated API requests)

Keyword-based triage always uses Plan mode. Act mode cannot be triggered by user input content — it must be explicitly requested via the `permission_mode` API field.

### Rate Limiting

All endpoints enforce per-IP rate limiting:
- 100 requests per minute (configurable)
- 50 concurrent connections per IP
- 10 WebSocket connections per IP
- 1 MB max request body size
- 64 KB max WebSocket message size

### Command Whitelist & Argument Validation

PTY sessions only allow whitelisted commands (`claude`, `cargo`, `git`, `python`, etc.). Commands with dangerous flags are blocked:

| Command | Blocked Flags |
|---------|--------------|
| `python`/`python3` | `-c`, `-m` |
| `node` | `-e`, `--eval` |
| `deno` | `eval`, `-e` |
| `bun` | `-e`, `--eval` |
| `cargo` | `--config` |
| `git` | `-c` |

### Session Limits

PTY sessions are capped at 32 concurrent sessions with a 1-hour TTL. Expired sessions are cleaned up automatically.

### Trust Model

| Boundary | Trust Level | Protection |
|----------|-------------|------------|
| External clients | Untrusted | Auth + rate limit + input validation |
| LLM responses | Semi-trusted | Route whitelist + instruction sanitization |
| Agent-to-agent | Semi-trusted | Output sanitization + size limits |
| Internal code | Trusted | Type system + Rust safety |

### Environment Variables

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `PANOPTES_API_KEY` | Recommended | None (unauthenticated) | API authentication key |
| `PANOPTES_BIND_ADDR` | No | `127.0.0.1` | Server bind address |
| `PANOPTES_CORS_ORIGINS` | No | localhost only | CORS allowed origins (comma-separated) |
| `OPENAI_API_KEY` | For ZeroClaw | None | OpenAI API key for LLM triage |

## Related Projects

- [Argus](https://github.com/jtmcg3/argus) — Original PTY-based Claude CLI wrapper
- [ZeroClaw](https://github.com/jtmcg3/zeroclaw) — Ultra-lightweight AI agent framework
- [swarms-rs](https://github.com/The-Swarm-Corporation/swarms-rs) — Rust multi-agent orchestration

## License

MIT
