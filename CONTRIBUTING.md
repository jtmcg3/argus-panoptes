# Contributing to Argus-Panoptes

## Prerequisites

- **Rust** (stable, edition 2024) — install via [rustup](https://rustup.rs/)
- **pkg-config** and **libssl-dev** (Linux) or equivalent OpenSSL headers
- **protobuf-compiler** (`protoc`)
- **Ollama** (optional) — for local LLM testing without an API key. Install from [ollama.com](https://ollama.com/), then `ollama pull lfm2:24b`

## Build Commands

```bash
cargo build              # Build all crates
cargo test --workspace   # Run all tests
cargo clippy --all-targets -- -D warnings  # Lint (must pass CI)
cargo fmt --all          # Format code
```

## Code Style

- Run `cargo fmt` before committing — CI enforces `cargo fmt --all -- --check`.
- All clippy warnings are treated as errors in CI (`-D warnings`).
- Follow existing patterns: async-first (tokio), trait-based agents, serde for serialization.
- Edition 2024 — do not downgrade.

## Testing

- Every new agent or module should include unit tests in a `#[cfg(test)] mod tests` block.
- Use mock traits (e.g., `MockCommandRunner`) for external dependencies.
- Run `cargo test --workspace` to verify nothing is broken.

## Commit Conventions

- Use conventional commit prefixes: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`.
- Keep commits focused — one logical change per commit.

## Project Structure

```
crates/
├── common/        # Shared types, traits (Agent, Task, AgentMessage)
├── coordinator/   # ZeroClaw triage + routing
├── agents/        # Specialist agents (coding, research, writing, etc.)
├── memory/        # LanceDB vector memory
├── llm/           # LLM client abstraction (OpenAI/Ollama, Anthropic)
├── pty-mcp/       # PTY session management via MCP
└── api/           # HTTP API gateway (binary: panoptes-api)
```

## Adding a New Agent

1. Create `crates/agents/src/<name>.rs` following the `ResearchAgent` pattern.
2. Implement the `Agent` trait from `panoptes_common::traits`.
3. Add `pub mod <name>;` and re-exports in `crates/agents/src/lib.rs`.
4. Register the agent in `crates/api/src/state.rs` (`AgentRegistry`).
5. Wire routing in `crates/coordinator/src/triage.rs` (`Coordinator::execute`).
