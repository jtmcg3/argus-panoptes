# PRD: Argus-Panoptes A2A Infrastructure

**Author:** Jim McG / Claude
**Date:** 2026-03-03
**Status:** Draft
**Version:** 1.0

---

## 1. Problem Statement

Argus-Panoptes is a multi-agent orchestration system that currently suffers from
three architectural problems:

### 1.1 Synchronous Timeout Bottleneck

All agent work is executed synchronously within an HTTP request-response cycle
bounded by a 300-second timeout. The research agent (search -> fetch 3 URLs ->
synthesize -> LLM enhance -> memory store) routinely approaches this limit. When
it exceeds the timeout, the caller gets an error and all in-flight work is lost.
There is no progress reporting -- the caller sees nothing until success or
timeout.

### 1.2 Monolithic API Server

`crates/api/` is a monolithic Axum server that owns every agent's lifecycle,
routes all requests, and is a single point of failure. Adding, updating, or
scaling a single agent requires redeploying the entire system.

### 1.3 Framework Duplication and Dead Weight

The codebase declares dependencies on ZeroClaw and references swarms-rs, but:

- **ZeroClaw** is used for exactly one `agent.turn()` call to get a JSON triage
  decision. The 95%+ of ZeroClaw's capabilities (gateway, channels, memory,
  tools, coordination, team orchestration) are unused, while argus-panoptes
  reimplements most of them independently.
- **swarms-rs** is mentioned in documentation but has zero imports, zero Cargo.toml
  dependencies, and zero code usage anywhere in the workspace. Custom
  `SequentialWorkflow` and `ConcurrentWorkflow` types were built from scratch.
- The custom `panoptes_llm::LlmClient` trait duplicates functionality available
  in both ZeroClaw and Rig.

---

## 2. Goals

### 2.1 Primary Goals

1. **Eliminate timeouts for long-running agent work.** The research agent (and
   any agent) must be able to run for minutes without the caller losing
   connection or state. Progress must be visible in real-time.

2. **Make each agent independently deployable and testable.** An agent should be
   a standalone process with a well-known endpoint. It should be discoverable,
   invocable, and observable without the rest of the system running.

3. **Adopt A2A as the inter-agent communication protocol.** Use an open standard
   (Google A2A, Linux Foundation governed, v0.3 stable) instead of proprietary
   in-process dispatch. This enables interop with the broader ecosystem
   (LangGraph, CrewAI, Spring AI, etc.).

4. **Preserve all existing agent domain logic.** The actual research/writing/
   planning/review/testing/coding logic is sound. Only the orchestration and
   communication layers change.

5. **Remove dead-weight dependencies.** Eliminate the ZeroClaw and swarms-rs
   dependencies. Replace the single `turn()` triage call with a direct LLM
   request using the existing `panoptes_llm` crate.

### 2.2 Stretch Goals

- Adopt `rig-core` as the LLM foundation layer, replacing `panoptes_llm` with
  Rig's richer provider abstraction (18+ providers, streaming, typed tools, RAG).
- Enable external A2A agents (in any language/framework) to participate in the
  system.
- Publish argus-panoptes agents as reusable A2A-compatible crates.

### 2.3 Non-Goals

- Building a personal assistant runtime (that's what the Claw ecosystem does).
- Supporting MCP as the inter-agent protocol (MCP is agent-to-tool; A2A is
  agent-to-agent; they are complementary, not alternatives).
- Multi-node distributed deployment in Phase 1. All agents run on a single
  machine initially, communicating over localhost.
- A web UI or dashboard.

---

## 3. Current Architecture Analysis

### 3.1 Workspace Crates

| Crate | Role | Verdict |
|-------|------|---------|
| `panoptes-common` | Shared types: `Task`, `AgentMessage`, `AgentCapability`, error types | **KEEP** as-is |
| `panoptes-memory` | LanceDB dual-layer memory + fastembed embeddings | **KEEP** as-is |
| `panoptes-llm` | LLM client abstraction (OpenAI/Ollama, Anthropic) | **KEEP** (Phase 1), evaluate Rig replacement (Phase 3) |
| `panoptes-agents` | 6 specialist agents + workflow orchestration | **KEEP** agent logic, **DELETE** `workflow.rs` |
| `panoptes-coordinator` | ZeroClaw triage + routing + PTY session management | **REWRITE** as A2A client + triage |
| `panoptes-api` | Monolithic Axum REST/WebSocket server | **DELETE** (each agent becomes its own server) |
| `panoptes-pty-mcp` | MCP server for PTY sessions | **KEEP** as-is (coding agent's tool) |

### 3.2 External Dependencies to Remove

| Dependency | Current Usage | Replacement |
|-----------|--------------|-------------|
| `zeroclaw` (git, main branch) | `Agent::turn()` for triage JSON | Direct LLM call via `panoptes_llm` with structured prompt |
| swarms-rs (documented, not used) | None | Remove from README |

### 3.3 External Dependencies to Add

| Dependency | Purpose | Crate |
|-----------|---------|-------|
| A2A protocol types | Agent Cards, Task lifecycle, Messages, Artifacts | `a2a-rs-core = "1.0"` |
| A2A server framework | JSON-RPC endpoint + Agent Card serving | Custom impl on axum 0.8 (see Risk 8.1 -- `a2a-rs-server` has dep conflicts) |
| A2A client | Calling remote A2A agents | `a2a-rs-client = "1.0"` |
| SSE streaming | Server-Sent Events for progress | `axum` (already in workspace) + `async-stream` |
| Broadcast channels | Agent progress events | `tokio::sync::broadcast` (already in workspace) |

### 3.4 What Maps to What

| Current (argus-panoptes) | A2A Equivalent |
|--------------------------|----------------|
| `AgentCapability` enum | Agent Card `skills[].tags` |
| `Agent::id()` + `Agent::name()` | Agent Card `name`, `description` |
| `Agent::process_task(&Task) -> AgentMessage` | `MessageHandler::handle_message(Message) -> Task` |
| `Task { status: TaskStatus }` | A2A `Task { status: TaskState }` |
| `TaskStatus::Pending/InProgress/Completed/Failed` | `TaskState::Submitted/Working/Completed/Failed` |
| `TaskStatus::AwaitingInput` | `TaskState::InputRequired` |
| `AgentMessage { content, source_agent, metadata }` | A2A `Message { parts: [TextPart], role }` |
| `AgentRoute` enum (coordinator routing) | Agent Card discovery + skill matching |
| `POST /api/v1/research` | `POST /` (JSON-RPC `message/send` to research agent) |
| WebSocket at `/api/v1/ws` | SSE via `message/stream` |

---

## 4. Target Architecture

### 4.1 High-Level Design

```
                    ┌──────────────────────────────────────────────────┐
                    │           Coordinator (port 8080)                │
  External       →  │  1. Triage: LLM classifies request              │
  Caller            │  2. Discovery: find agent by skill               │
  (ZeroClaw,        │  3. Delegate: A2A message/stream to agent        │
   curl, any        │  4. Forward: stream progress back to caller      │
   A2A client)      └──────┬────────┬────────┬────────┬───────┬───────┘
                           │        │        │        │       │
                    ┌──────▼──┐ ┌───▼────┐ ┌─▼──────┐│  ┌────▼─────┐
                    │Research │ │Writing │ │Planning││  │ Testing  │
                    │ :9001   │ │ :9002  │ │ :9003  ││  │  :9005   │
                    │         │ │        │ │        ││  │          │
                    │SearXNG  │ │LLM     │ │LLM     ││  │cargo test│
                    │HTTP     │ │Memory  │ │Memory  ││  │LLM       │
                    │LLM      │ │        │ │        ││  │          │
                    │Memory   │ │        │ │        ││  │          │
                    └─────────┘ └────────┘ └────────┘│  └──────────┘
                                               ┌─────▼────┐ ┌──────────┐
                                               │ Review   │ │ Coding   │
                                               │  :9004   │ │  :9006   │
                                               │          │ │          │
                                               │clippy    │ │PTY-MCP   │
                                               │fmt       │ │Claude CLI│
                                               │LLM       │ │          │
                                               └──────────┘ └──────────┘
```

Each box is an independent process serving:
- `GET /.well-known/agent.json` -- Agent Card (discovery)
- `POST /` -- JSON-RPC 2.0 endpoint (`message/send`, `message/stream`, `tasks/get`, `tasks/cancel`)

### 4.2 Request Flow (Research Example)

```
Caller                  Coordinator              Research Agent (:9001)
  │                         │                          │
  │── message/stream ──────>│                          │
  │                         │── triage (LLM) ─────────>│
  │                         │<─ route: research ───────│
  │                         │                          │
  │                         │── message/stream ───────>│
  │                         │                          │── search SearXNG
  │<── SSE: working ────────│<── SSE: working ─────────│
  │    "searching..."       │    "searching..."        │
  │                         │                          │── fetch URL 1/3
  │<── SSE: working ────────│<── SSE: working ─────────│
  │    "fetched 1/3"        │    "fetched 1/3"         │
  │                         │                          │── fetch URL 2/3
  │<── SSE: working ────────│<── SSE: working ─────────│
  │    "fetched 2/3"        │    "fetched 2/3"         │
  │                         │                          │── synthesize
  │<── SSE: working ────────│<── SSE: working ─────────│
  │    "synthesizing..."    │    "synthesizing..."     │
  │                         │                          │── LLM enhance
  │<── SSE: working ────────│<── SSE: working ─────────│
  │    "enhancing..."       │    "enhancing..."        │
  │                         │                          │── store memory
  │<── SSE: artifact ───────│<── SSE: artifact ────────│
  │    {text: "...result"}  │    {text: "...result"}   │
  │<── SSE: completed ──────│<── SSE: completed ───────│
  │                         │                          │
```

No timeouts. Connection stays open. Progress is visible at every phase.

### 4.3 Coordinator as "Smart Router"

The coordinator is NOT just a pass-through. It:

1. **Triages** via a direct LLM call (replacing ZeroClaw's `turn()`) with the
   same structured JSON prompt and security validation (SEC-009).
2. **Discovers** agents by fetching Agent Cards from configured URLs (initially
   static config, later dynamic registry).
3. **Delegates** via A2A `message/stream`, forwarding SSE events to the caller.
4. **Orchestrates** multi-step workflows by decomposing complex requests into
   sub-tasks, delegating each to the appropriate agent, and aggregating results.
5. **Is itself an A2A server** -- external callers talk to it via A2A, making
   the entire system accessible to any A2A-compatible client.

### 4.4 Agent Card Examples

**Research Agent:**
```json
{
  "name": "panoptes-research",
  "description": "Web search, document analysis, and knowledge synthesis with persistent memory",
  "url": "http://localhost:9001/",
  "version": "0.1.0",
  "capabilities": {
    "streaming": true,
    "pushNotifications": false
  },
  "skills": [
    {
      "id": "web-research",
      "name": "Web Research",
      "description": "Search the web, fetch and parse sources, synthesize findings",
      "tags": ["research", "search", "web", "analysis"],
      "examples": [
        "Research the latest advances in CRISPR gene therapy",
        "Find information about Rust async patterns"
      ]
    },
    {
      "id": "knowledge-recall",
      "name": "Knowledge Recall",
      "description": "Retrieve and synthesize prior research from persistent memory",
      "tags": ["memory", "recall", "knowledge"],
      "examples": ["What do we already know about quantum computing?"]
    }
  ],
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain", "text/markdown"]
}
```

**Coding Agent:**
```json
{
  "name": "panoptes-coding",
  "description": "Code generation and modification via Claude CLI in isolated PTY sessions",
  "url": "http://localhost:9006/",
  "version": "0.1.0",
  "capabilities": {
    "streaming": true,
    "pushNotifications": false
  },
  "skills": [
    {
      "id": "code-generation",
      "name": "Code Generation",
      "description": "Write new code, implement features, fix bugs",
      "tags": ["coding", "generation", "implementation"],
      "examples": ["Add a retry mechanism to the HTTP client"]
    },
    {
      "id": "code-execution",
      "name": "Code Execution",
      "description": "Run code in a sandboxed PTY environment via Claude CLI",
      "tags": ["coding", "execution", "pty", "cli"],
      "examples": ["Run the test suite and fix any failures"]
    }
  ],
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain", "application/json"]
}
```

---

## 5. Crate-Level Design

### 5.1 New Crate: `panoptes-a2a`

**Purpose:** Shared A2A infrastructure. Bridges existing agent logic to the A2A
protocol. Provides a generic server wrapper that any agent can use.

**Location:** `crates/a2a/`

**Dependencies:**
```toml
[dependencies]
# A2A types only (clean deps: serde, chrono, uuid)
a2a-rs-core = "1.0"
# NOTE: We do NOT use a2a-rs-server due to dep conflicts
# (it requires axum 0.7, thiserror 1, tower 0.4).
# Instead we build a thin JSON-RPC + Agent Card server layer
# directly on axum 0.8 (~200-300 lines).

axum = { version = "0.8", features = ["macros"] }
tokio = { workspace = true }
tokio-stream = "0.1"
async-stream = "0.3"
dashmap = "6"
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
panoptes-common = { workspace = true }
```

**Key Types:**

```rust
/// Progress events emitted by agents during execution.
#[derive(Debug, Clone, Serialize)]
pub enum ProgressEvent {
    /// Agent is working on a named phase.
    Phase { name: String, detail: String },
    /// Agent has a partial result to share.
    PartialResult { content: String },
    /// Agent encountered a non-fatal issue.
    Warning { message: String },
}

/// The bridge trait. Each specialist agent implements this to
/// expose itself as an A2A server.
#[async_trait]
pub trait A2aAgent: Send + Sync + 'static {
    /// Agent identity.
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn version(&self) -> &str { "0.1.0" }

    /// Skills (maps from AgentCapability).
    fn skills(&self) -> Vec<a2a_rs_core::Skill>;

    /// Whether streaming is supported.
    fn supports_streaming(&self) -> bool { false }

    /// Handle a task. Emit progress via the sender if streaming.
    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<tokio::sync::broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, AgentError>;
}

/// Generic A2A server that wraps any A2aAgent.
pub struct AgentServer<A: A2aAgent> { /* ... */ }

impl<A: A2aAgent> AgentServer<A> {
    pub fn new(agent: A, port: u16) -> Self;
    pub async fn run(self) -> anyhow::Result<()>;
}
```

**Responsibilities:**
- Serve `GET /.well-known/agent.json` from `A2aAgent::skills()` etc.
- Serve `POST /` JSON-RPC endpoint (`message/send`, `message/stream`,
  `tasks/get`, `tasks/cancel`).
- Manage task state in an in-memory `DashMap<TaskId, TaskState>`.
- Bridge `ProgressEvent` emissions from agents to A2A SSE
  `TaskStatusUpdateEvent` and `TaskArtifactUpdateEvent`.
- Handle concurrent requests via tokio tasks.

### 5.2 Modified Crate: `panoptes-agents`

**Changes:**
- Delete `workflow.rs` (orchestration moves to coordinator).
- Add `A2aAgent` impl for each specialist agent (thin bridge, ~30-50 lines
  each).
- Add a binary target for each agent (`bin/research.rs`, `bin/writing.rs`, etc.)
  that constructs the agent with config and runs `AgentServer::new(agent, port).run()`.
- Keep all existing domain logic untouched.

**New dependencies:**
```toml
panoptes-a2a = { workspace = true }
```

**Example binary (`crates/agents/src/bin/research.rs`):**
```rust
use panoptes_agents::ResearchAgent;
use panoptes_a2a::AgentServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = load_config()?;
    let agent = ResearchAgent::with_default_config()
        .with_search_url(config.search.url)
        .with_llm(build_llm_client(&config.llm).await?)
        .with_memory(build_memory_store(&config.memory).await?);

    tracing::info!("Research agent starting on port 9001");
    AgentServer::new(agent, 9001).run().await
}
```

### 5.3 Rewritten Crate: `panoptes-coordinator`

**Purpose:** A2A client that discovers agents, triages requests, delegates work,
and streams results. Also serves as an A2A server itself (the system's entry
point).

**Changes:**
- Remove `zeroclaw` dependency entirely.
- Replace ZeroClaw triage with a direct LLM call using the existing
  `panoptes_llm` crate. The structured prompt and security validation (SEC-009)
  are preserved.
- Add `AgentRegistry` for discovering agents via Agent Cards.
- Add `Orchestrator` for delegating tasks and streaming progress.
- Add a binary target that runs the coordinator as an A2A server.

**New structure:**
```
crates/coordinator/src/
  lib.rs
  triage.rs          # LLM-based classification (migrated from zeroclaw_triage.rs)
  discovery.rs       # Fetch and cache Agent Cards
  orchestrator.rs    # Delegate tasks, stream progress, aggregate results
  bin/
    coordinator.rs   # Binary: A2A server on port 8080
```

**Dependencies:**
```toml
[dependencies]
panoptes-common = { workspace = true }
panoptes-llm = { workspace = true }
panoptes-a2a = { workspace = true }

# A2A protocol types + client
a2a-rs-core = "1.0"
a2a-rs-client = "1.0"

# Async
tokio = { workspace = true }
async-trait = { workspace = true }
futures = { workspace = true }

# Serialization, logging, errors
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }

# No more zeroclaw!
```

### 5.4 Deleted Crate: `panoptes-api`

The monolithic API server is replaced by:
- Each agent running its own A2A server (via `panoptes-a2a::AgentServer`).
- The coordinator running as the A2A entry point.

All existing routes map to A2A equivalents:

| Current Route | A2A Equivalent |
|--------------|----------------|
| `POST /api/v1/messages` | `POST /` on coordinator (JSON-RPC `message/send`) |
| `POST /api/v1/research` | `POST /` on research agent directly |
| `POST /api/v1/coding` | `POST /` on coding agent directly |
| `POST /api/v1/workflow` | `POST /` on coordinator (decomposes into sub-tasks) |
| `GET /api/v1/ws` | SSE via `message/stream` on any agent or coordinator |
| `GET /health` | Can be added to `panoptes-a2a::AgentServer` |

### 5.5 Unchanged Crates

| Crate | Notes |
|-------|-------|
| `panoptes-common` | No changes. `Task`, `AgentMessage`, `AgentCapability` remain. |
| `panoptes-memory` | No changes. LanceDB + fastembed. |
| `panoptes-llm` | No changes in Phase 1. Used by triage and agents. |
| `panoptes-pty-mcp` | No changes. Still the coding agent's MCP tool server. |

---

## 6. Configuration

### 6.1 Updated `config/default.toml`

```toml
# Argus-Panoptes Configuration

# --- Coordinator ---
[coordinator]
port = 8080
# Agent URLs for discovery (coordinator fetches Agent Cards from these)
agent_urls = [
    "http://localhost:9001",  # research
    "http://localhost:9002",  # writing
    "http://localhost:9003",  # planning
    "http://localhost:9004",  # review
    "http://localhost:9005",  # testing
    "http://localhost:9006",  # coding
]

# --- Triage (replaces [provider] section) ---
[triage]
provider = "openai"           # "openai" (also works for Ollama) or "anthropic"
model = "lfm2:24b"
api_url = "http://host.orb.internal:11434"
temperature = 0.3
timeout_ms = 30000

# --- LLM (for agent content generation) ---
[llm]
provider = "openai"
model = "lfm2:24b"
api_url = "http://host.orb.internal:11434"
max_concurrent_requests = 2

[llm.retry]
max_retries = 3
initial_delay_ms = 500
max_delay_ms = 30000
backoff_multiplier = 2.0

# --- Agent Ports ---
[agents.research]
port = 9001

[agents.writing]
port = 9002

[agents.planning]
port = 9003

[agents.review]
port = 9004

[agents.testing]
port = 9005

[agents.coding]
port = 9006

# --- Search (research agent) ---
[search]
url = "http://100.85.147.105:8888"
max_results = 5

# --- Memory (shared across agents that use it) ---
[memory]
db_path = "./data/memory"
embedding_model = "all-MiniLM-L6-v2"
max_context_tokens = 4096

[memory.working]
size = 20
```

### 6.2 Process Management

In Phase 1, all agents run on localhost. A simple process manager (shell script
or `cargo-make` task) starts them:

```bash
#!/usr/bin/env bash
# start-agents.sh

cargo build --release

# Start agents in background
./target/release/panoptes-research &
./target/release/panoptes-writing &
./target/release/panoptes-planning &
./target/release/panoptes-review &
./target/release/panoptes-testing &
./target/release/panoptes-coding &

# Start coordinator (foreground)
./target/release/panoptes-coordinator
```

In future phases, this could be replaced by Docker Compose, systemd units, or
Kubernetes manifests.

---

## 7. Implementation Phases

### Phase 1: A2A Foundation (Weeks 1-2)

**Goal:** Build the shared A2A infrastructure and convert one agent as proof of
concept.

**Tasks:**
1. Create `crates/a2a/` with `A2aAgent` trait, `AgentServer`, and progress
   streaming infrastructure.
2. Add `a2a-rs-core` and `a2a-rs-client` to workspace dependencies. Build
   custom JSON-RPC server layer on axum 0.8 (do NOT use `a2a-rs-server` due
   to dependency conflicts with axum 0.7/thiserror 1/tower 0.4).
3. Implement `A2aAgent` for `ResearchAgent` (the primary timeout victim).
4. Add `bin/research.rs` binary target.
5. Verify: `curl localhost:9001/.well-known/agent.json` returns valid Agent Card.
6. Verify: JSON-RPC `message/send` executes a research task end-to-end.
7. Verify: `message/stream` delivers real-time progress via SSE.
8. Write integration tests for the A2A server.

**Exit Criteria:**
- Research agent runs as standalone A2A server.
- Progress events stream in real-time during web search/fetch/synthesize.
- Existing unit tests still pass (agent logic unchanged).

### Phase 2: All Agents + Coordinator (Weeks 3-4)

**Goal:** Convert remaining agents and build the coordinator as an A2A client.

**Tasks:**
1. Implement `A2aAgent` for Writing, Planning, Review, Testing, Coding agents.
2. Add binary targets for each agent.
3. Rewrite coordinator:
   - Replace ZeroClaw triage with direct LLM call.
   - Implement `AgentRegistry` (fetch Agent Cards on startup).
   - Implement `Orchestrator` (delegate via A2A, forward SSE).
   - Make coordinator itself an A2A server.
4. Remove `zeroclaw` from workspace dependencies.
5. Delete `crates/api/`.
6. Update `config/default.toml`.
7. Write integration tests for coordinator -> agent flow.

**Exit Criteria:**
- All 6 agents run as independent A2A servers.
- Coordinator discovers agents, triages requests, delegates via A2A.
- End-to-end flow works: caller -> coordinator -> agent -> response with
  streaming progress.
- `zeroclaw` dependency removed.
- `crates/api/` deleted.
- All existing tests pass or are migrated.

### Phase 3: Polish and Harden (Weeks 5-6)

**Goal:** Production-readiness, security, observability.

**Tasks:**
1. Add authentication to Agent Cards (API key or Bearer token).
2. Add rate limiting to `AgentServer` (per-IP, from existing SEC-006 logic).
3. Add input validation / injection detection (migrate SEC-009 logic).
4. Add health check endpoints to all agents.
5. Add Prometheus metrics / OpenTelemetry tracing.
6. Add graceful shutdown handling.
7. Create Docker Compose manifest for deployment.
8. Update all documentation (README, CLAUDE.md, architecture docs).
9. Evaluate replacing `panoptes-llm` with `rig-core` for richer provider
   support and streaming.
10. Benchmark: measure latency overhead of A2A (JSON-RPC + HTTP) vs previous
    in-process dispatch.

**Exit Criteria:**
- Security controls (auth, rate limiting, input validation) are in place.
- Observability (metrics, tracing, health checks) is operational.
- Documentation reflects the new architecture.
- Performance benchmarks show acceptable overhead.

### Phase 4: Advanced Orchestration (Future)

**Goal:** Multi-step workflows, agent composition, external interop.

**Tasks (scope TBD):**
1. Multi-agent workflows: coordinator decomposes complex requests into sub-tasks,
   delegates to multiple agents (sequential, parallel, DAG), aggregates results.
2. Agent-as-tool: an agent can invoke another agent as part of its work (e.g.,
   planning agent invokes research agent for context).
3. External agent integration: connect to third-party A2A agents.
4. Dynamic agent registration: agents register with coordinator on startup
   instead of static config.
5. Push notifications: for truly long-running tasks (hours), support webhook
   callbacks instead of SSE.

---

## 8. Risk Assessment

### 8.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `a2a-rs-server` crate is immature (v1.0.0, low downloads) and has **dependency conflicts** (axum 0.7, thiserror 1, tower 0.4 vs our axum 0.8, thiserror 2, tower 0.5) | High | High | **Recommended: use `a2a-rs-core` for types only and build our own thin server layer on axum 0.8.** The server is ~2k lines and the JSON-RPC protocol is straightforward. This avoids pulling in conflicting axum/tower/thiserror versions. Alternatively, fork and update deps. |
| JSON-RPC + HTTP overhead between localhost processes | Low | Low | Benchmark in Phase 3. Expected <1ms per call on localhost. Agent work (LLM calls, web fetches) dominates by orders of magnitude. |
| SSE streaming complexity (backpressure, disconnects) | Medium | Medium | Use tokio broadcast channels with bounded capacity. Drop old events if consumer is slow. Reconnect logic in coordinator's A2A client. |
| Breaking changes in A2A spec (v0.3 -> v1.0) | Low | Medium | v0.3 backward compat is promised. Our A2A layer is thin (~1 crate). Migration cost is bounded. |
| Memory store concurrency (multiple agent processes sharing LanceDB) | Medium | High | LanceDB supports concurrent readers. Phase 1: each agent gets its own DB path. Phase 2: evaluate shared access or memory-as-a-service. |

### 8.2 Dependency Risks

| Dependency | Risk | Notes |
|-----------|------|-------|
| `a2a-rs-core` v1.0.0 | New crate, may have bugs | Pin version, monitor for updates |
| `a2a-rs-server` v1.0.0 | Dep conflicts | **Decision: not using.** Building custom server layer on axum 0.8 instead. |
| Removing `zeroclaw` | Build breakage | The triage logic is well-understood; port the prompt and validation |
| Removing `crates/api/` | Losing integration tests | Migrate tests to new coordinator/agent tests |

### 8.3 Process Risks

| Risk | Mitigation |
|------|------------|
| Scope creep into Phase 4 features | Strict phase gating. Phase 1 ships before Phase 2 starts. |
| Agent port conflicts | Configurable ports in `default.toml`. Document defaults clearly. |
| Process management complexity (7 processes) | Simple shell script in Phase 1. Docker Compose in Phase 3. |

---

## 9. Success Criteria

### 9.1 Functional

- [ ] All 6 agents serve valid Agent Cards at `/.well-known/agent.json`.
- [ ] All 6 agents handle `message/send` JSON-RPC requests correctly.
- [ ] Research, Review, Testing, and Coding agents support `message/stream` with
      real-time progress events.
- [ ] Coordinator triages requests and routes to correct agent via A2A.
- [ ] Coordinator forwards SSE progress events from agents to callers.
- [ ] End-to-end latency for research tasks is no worse than current (minus the
      timeout failures, which are eliminated).
- [ ] `zeroclaw` dependency is removed from the workspace.
- [ ] `crates/api/` is deleted.
- [ ] All existing tests pass or are migrated.

### 9.2 Non-Functional

- [ ] Any A2A-compatible client can invoke agents directly (tested with `curl`
      and Python `a2a-sdk`).
- [ ] Agent Cards are discoverable from configured URLs.
- [ ] Each agent can be started/stopped independently without affecting others.
- [ ] Memory isolation: agent crash does not corrupt shared state.
- [ ] Progress events arrive within 100ms of emission (localhost).

### 9.3 Quality

- [ ] `cargo clippy` clean.
- [ ] `cargo fmt` clean.
- [ ] Integration test coverage for coordinator -> agent flow.
- [ ] Unit test coverage for A2A bridge layer.
- [ ] Documentation updated (README, CLAUDE.md, architecture diagram).

---

## 10. Open Questions

1. **Shared vs per-agent memory:** Should all agents share a single LanceDB
   instance, or should each agent have its own? Shared enables cross-agent
   knowledge but introduces concurrency concerns. Per-agent is simpler but
   fragments knowledge.

2. **Rig migration timing:** Should we replace `panoptes-llm` with `rig-core`
   during Phase 2 (when we're already touching agent code) or defer to Phase 3?
   Rig provides 18+ providers, native streaming, typed tools, and RAG -- all of
   which are currently hand-rolled.

3. **Agent Card versioning:** How do we handle Agent Card changes across
   deployments? Should the coordinator cache cards and refresh on a schedule, or
   fetch fresh on every request?

4. **Coding agent PTY lifecycle:** The coding agent currently manages PTY
   sessions via MCP. With the A2A model, how do we handle long-lived PTY
   sessions that span multiple A2A requests? The session state lives in the
   PTY-MCP server, so A2A's stateless request model may need a session ID
   extension.

5. **Authentication model:** Should agents authenticate to each other, or is
   localhost trust sufficient for Phase 1? If auth is needed, API keys (simple)
   or mTLS (robust)?

---

## Appendix A: Dependency Version Matrix

| Crate | Version | Purpose |
|-------|---------|---------|
| `a2a-rs-core` | 1.0.0 | A2A protocol types (clean deps: serde, chrono, uuid) |
| `a2a-rs-server` | ~~1.0.0~~ | **NOT USED** -- dep conflicts (axum 0.7, thiserror 1, tower 0.4). Custom server layer built on axum 0.8 instead. |
| `a2a-rs-client` | 1.0.0 | A2A HTTP client (clean deps: reqwest 0.12, no conflicts) |
| `axum` | 0.8 | HTTP server (already in workspace) |
| `tokio` | 1 | Async runtime (already in workspace) |
| `tokio-stream` | 0.1 | Stream utilities |
| `async-stream` | 0.3 | `stream!` macro for SSE |
| `dashmap` | 6 | Concurrent task state storage |
| `serde` | 1 | Serialization (already in workspace) |
| `serde_json` | 1 | JSON (already in workspace) |
| `uuid` | 1 | Task/message IDs (already in workspace) |
| `tracing` | 0.1 | Logging (already in workspace) |
| `reqwest` | 0.12 | HTTP client (already in workspace) |

## Appendix B: Binary Targets

| Binary | Crate | Default Port | Description |
|--------|-------|-------------|-------------|
| `panoptes-coordinator` | coordinator | 8080 | A2A entry point, triage, orchestration |
| `panoptes-research` | agents | 9001 | Research agent A2A server |
| `panoptes-writing` | agents | 9002 | Writing agent A2A server |
| `panoptes-planning` | agents | 9003 | Planning agent A2A server |
| `panoptes-review` | agents | 9004 | Review agent A2A server |
| `panoptes-testing` | agents | 9005 | Testing agent A2A server |
| `panoptes-coding` | agents | 9006 | Coding agent A2A server |
| `pty-mcp-server` | pty-mcp | stdio | PTY MCP tool server (unchanged) |

## Appendix C: Migration Checklist

### Before Starting
- [ ] Tag current main as `v0.1.0-pre-a2a` for rollback.
- [ ] Verify all 258 tests pass on current main.
- [ ] Document current ZeroClaw triage prompt for migration.

### Phase 1 Completion
- [ ] `crates/a2a/` exists and builds.
- [ ] Research agent runs as standalone binary.
- [ ] Agent Card serves correctly.
- [ ] `message/send` works end-to-end.
- [ ] `message/stream` delivers progress events.
- [ ] New integration tests pass.
- [ ] Existing tests still pass.

### Phase 2 Completion
- [ ] All 6 agents run as standalone binaries.
- [ ] Coordinator runs as A2A server.
- [ ] `zeroclaw` removed from Cargo.toml.
- [ ] `crates/api/` deleted.
- [ ] End-to-end flow: coordinator -> triage -> agent -> streaming response.
- [ ] All tests pass or migrated.

### Phase 3 Completion
- [ ] Authentication on all endpoints.
- [ ] Rate limiting active.
- [ ] Health checks on all agents.
- [ ] Metrics/tracing operational.
- [ ] Docker Compose deployment works.
- [ ] Documentation updated.
- [ ] Performance benchmarked.
