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

# Stage 2: Runtime (Ubuntu 24.04 LTS)
FROM ubuntu:24.04 AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 panoptes
USER panoptes

# Copy binaries from builder
COPY --from=builder /app/target/release/pty-mcp-server /usr/local/bin/
COPY --from=builder /app/target/release/panoptes-api /usr/local/bin/

# Create data directories
RUN mkdir -p /app/data/memory /app/config

# Environment defaults
ENV RUST_LOG=info,panoptes=debug
ENV PANOPTES_PORT=8080

# Expose API port
EXPOSE 8080

# Default command (API server)
CMD ["panoptes-api", "--port", "8080"]

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1
