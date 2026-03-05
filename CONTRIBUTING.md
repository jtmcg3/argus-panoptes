# Contributing to Argus-Panoptes

## Prerequisites

- Rust stable (edition 2024)
- `pkg-config` and OpenSSL headers (`libssl-dev` on Debian/Ubuntu)
- `protoc` (`protobuf-compiler`)
- Optional: Ollama for local model inference

## Build and Validation

```bash
cargo build
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Project Structure

```text
crates/
├── a2a/           # Shared A2A server/protocol infrastructure
├── coordinator/   # Triage + delegation orchestrator
├── agents/        # Specialist agents + per-agent binaries
├── llm/           # LLM abstraction
├── memory/        # LanceDB memory
├── pty-mcp/       # PTY MCP server
└── common/        # Shared types/errors
```

## Adding or Updating an Agent

1. Implement or update logic in `crates/agents/src/<agent>.rs`.
2. Bridge A2A behavior in `crates/agents/src/a2a_bridge.rs`.
3. Ensure binary wiring exists in `crates/agents/src/bin/<agent>.rs`.
4. Verify triage routing in `crates/coordinator/src/triage.rs`.
5. Add/adjust tests (unit + integration as needed).

## Commit Guidance

- Use focused commits with conventional prefixes (`feat:`, `fix:`, `docs:`, `test:`, etc.).
- Keep behavior changes and documentation changes aligned in the same PR.
