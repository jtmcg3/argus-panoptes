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

# Build release binary
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 panoptes
USER panoptes

# Copy binaries from builder
COPY --from=builder /app/target/release/pty-mcp-server /usr/local/bin/

# Create data directories
RUN mkdir -p /app/data/memory /app/config

# Environment
ENV RUST_LOG=info,panoptes=debug

# Default command (can be overridden)
CMD ["pty-mcp-server"]

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1
