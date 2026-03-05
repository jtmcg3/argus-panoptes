# Argus-Panoptes Multi-Agent System
# Multi-stage build for minimal image size

# Stage 1: Build
FROM rust:1.87-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binaries
RUN cargo build --release

# Stage 2: Runtime
FROM ubuntu:24.04 AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user and data directories
RUN useradd -m -u 1000 panoptes \
    && mkdir -p /app/data/memory /app/config \
    && chown -R panoptes:panoptes /app

# Copy binaries from builder
COPY --from=builder /app/target/release/panoptes-coordinator /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-research /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-writing /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-planning /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-review /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-testing /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-coding /usr/local/bin/
COPY --from=builder /app/target/release/pty-mcp-server /usr/local/bin/

USER panoptes

ENV RUST_LOG=info,panoptes=debug
ENV PANOPTES_CONFIG=/app/config/default.toml

EXPOSE 18080 9001 9002 9003 9004 9005 9006

# Default entrypoint is the coordinator.
CMD ["panoptes-coordinator"]
