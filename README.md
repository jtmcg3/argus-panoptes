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

## Related Projects

- [Argus](https://github.com/jtmcg3/argus) — Original PTY-based Claude CLI wrapper
- [ZeroClaw](https://github.com/jtmcg3/zeroclaw) — Ultra-lightweight AI agent framework
- [swarms-rs](https://github.com/The-Swarm-Corporation/swarms-rs) — Rust multi-agent orchestration

## License

MIT
