# Argus-Panoptes Deployment with OrbStack

This document covers deployment options for the Argus-Panoptes multi-agent orchestration system using [OrbStack](https://orbstack.dev/) on macOS.

## Table of Contents

1. [OrbStack Overview](#orbstack-overview)
2. [Option 1: Linux VM](#option-1-linux-vm)
3. [Option 2: Kubernetes](#option-2-kubernetes-orbstack-k8s)
4. [Option 3: Docker Containers](#option-3-docker-containers)
5. [Recommendation](#recommendation)
6. [Quick Start Guides](#quick-start-guides)

---

## OrbStack Overview

### What is OrbStack?

OrbStack is a fast, lightweight Docker Desktop alternative designed specifically for macOS. It provides:

- **Docker containers** with full Docker CLI compatibility
- **Linux virtual machines** supporting 15+ distributions
- **Kubernetes** single-node cluster for development
- **Native macOS integration** with Finder, SSH, and DNS

### Key Advantages Over Docker Desktop

| Feature | OrbStack | Docker Desktop |
|---------|----------|----------------|
| Startup time | ~2 seconds | 20-30 seconds |
| Idle CPU usage | ~0.1% | Higher baseline |
| Memory management | Dynamic (grows/shrinks) | Static allocation |
| File sharing | VirtioFS + caching (2-5x faster) | Standard VirtioFS |
| Power efficiency | 1.7x more efficient | Higher consumption |
| Image builds | 1.2x faster | Baseline |

### Deployment Modes Available

1. **Linux VM** - Full Linux distribution with systemd support
2. **Kubernetes** - Single-node k8s cluster with deep integration
3. **Docker Containers** - Standard containers with Compose support

### Pricing

- **Personal use**: Free
- **Commercial use**: $8/month ($96/year)

### Limitations

- macOS-only (no Windows/Linux support)
- Younger ecosystem than Docker Desktop
- Some edge cases may lack community documentation

---

## Option 1: Linux VM

### Overview

Run Argus-Panoptes in a full Linux virtual machine with direct access to the host filesystem and native systemd support.

### Distro Comparison for Rust/AI Workloads

| Distribution | Rust Support | AI/Ollama Support | Learning Curve | Recommendation |
|--------------|--------------|-------------------|----------------|----------------|
| **Ubuntu 24.04 LTS** | Excellent | First-party NVIDIA/CUDA | Low | Best for production |
| **Fedora 40+** | Excellent | Good | Medium | Best for development |
| **Alpine** | Good | Limited | Medium | Best for containers only |
| **NixOS** | Excellent | Growing | High | Best for reproducibility |

### Recommended: Ubuntu 24.04 LTS

**Why Ubuntu for Argus-Panoptes:**

1. **Ollama support** - First-party installation, prebuilt binaries
2. **CUDA/GPU** - Best driver support if using GPU acceleration
3. **LanceDB** - Tested compatibility with Arrow ecosystem
4. **Community** - Most documentation and troubleshooting resources
5. **fastembed** - ONNX runtime has excellent Ubuntu support

### Resource Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU cores | 4 | 8+ |
| RAM | 8 GB | 16 GB |
| Disk | 20 GB | 50 GB |
| Ollama models | +4 GB per model | +8 GB for larger models |

**Note:** OrbStack uses dynamic memory allocation - unused RAM is returned to macOS automatically.

### Setup Guide: Linux VM with Ollama

```bash
# Create Ubuntu VM
orb create ubuntu:24.04 argus-vm

# Enter the VM
orb shell argus-vm

# Inside VM: Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# Start Ollama service
sudo systemctl enable ollama
sudo systemctl start ollama

# Pull the default model
ollama pull lfm2:24b

# Install build dependencies
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev

# Clone and build Argus-Panoptes
git clone https://github.com/jtmcg3/argus-panoptes.git
cd argus-panoptes
cargo build --release

# Configure
cp config/config.example.toml config/config.toml
# If adding API keys, restrict permissions:
chmod 600 config/config.toml

# Run the API server
./target/release/panoptes-api --config config/config.toml --memory
```

> **Ollama from VM**: When running Ollama on the Mac host and the server inside an
> OrbStack VM, use `host.orb.internal:11434` as the Ollama URL in your config
> (instead of `localhost:11434`).

### Pros

- Full Linux environment with systemd
- Direct Ollama installation (native performance)
- Simple to understand and debug
- SSH access from macOS: `ssh argus-vm@orb`
- Filesystem access: `~/OrbStack/machines/argus-vm`

### Cons

- Manual service management
- No built-in scaling
- Single point of failure
- Updates require manual intervention

---

## Option 2: Kubernetes (OrbStack K8s)

### Overview

OrbStack includes a lightweight single-node Kubernetes cluster optimized for development, comparable to Minikube but with better macOS integration.

### Key Features

- **Zero configuration** - Kubernetes included out of the box
- **Direct service access** - No port forwarding needed; services accessible at `*.k8s.orb.local`
- **Shared images** - Built images immediately available in pods
- **LoadBalancer support** - Works out of the box
- **cluster.local DNS** - Accessible directly from Mac

### When Kubernetes Makes Sense

**Good fit if:**
- You plan to deploy to production Kubernetes later
- You want to learn/practice Kubernetes concepts
- You need to test Helm charts or operators
- Multiple services need service discovery

**Overkill if:**
- Single-user development tool
- Simple service dependencies
- No production k8s in roadmap
- Team unfamiliar with k8s

### Resource Overhead

| Metric | Impact |
|--------|--------|
| Base memory | +200-400 MB for k8s components |
| Startup time | Additional 5-10 seconds |
| Complexity | Significant learning curve |
| Debugging | Requires k8s-specific tooling |

### Helm Chart Considerations

For Argus-Panoptes, a Helm chart would need:

```yaml
# Example structure
argus-panoptes/
  Chart.yaml
  values.yaml
  templates/
    coordinator-deployment.yaml
    pty-mcp-deployment.yaml
    ollama-statefulset.yaml
    lancedb-pvc.yaml
    configmap.yaml
    service.yaml
```

**Complexity factors:**
- StatefulSet for Ollama (model persistence)
- PersistentVolumeClaim for LanceDB data
- ConfigMaps for routing configuration
- Service mesh for inter-agent communication
- Secrets for API keys

### Sample Kubernetes Manifest

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: argus-coordinator
spec:
  replicas: 1
  selector:
    matchLabels:
      app: argus-coordinator
  template:
    metadata:
      labels:
        app: argus-coordinator
    spec:
      containers:
      - name: coordinator
        image: argus-panoptes/coordinator:latest
        imagePullPolicy: IfNotPresent
        ports:
        - containerPort: 8080
        env:
        - name: OLLAMA_HOST
          value: "http://ollama:11434"
        - name: LANCEDB_PATH
          value: "/data/lancedb"
        volumeMounts:
        - name: lancedb-data
          mountPath: /data/lancedb
      volumes:
      - name: lancedb-data
        persistentVolumeClaim:
          claimName: lancedb-pvc
---
apiVersion: v1
kind: Service
metadata:
  name: argus-coordinator
spec:
  type: LoadBalancer
  ports:
  - port: 8080
  selector:
    app: argus-coordinator
```

### Pros

- Production-like environment
- Service discovery built-in
- Declarative configuration
- Easy rollbacks
- Prepare for production k8s

### Cons

- Significant complexity overhead
- Overkill for single-user tool
- Longer feedback loops
- Debugging requires k8s knowledge
- Resource overhead

---

## Option 3: Docker Containers

### Overview

Run Argus-Panoptes as Docker containers using Docker Compose for orchestration. This balances simplicity with container benefits.

### Multi-Container vs Single Container

| Approach | Use Case | Complexity |
|----------|----------|------------|
| **Multi-container** | Production-ready, separate concerns | Medium |
| **Single container** | Simple deployment, all-in-one | Low |

**Recommendation:** Multi-container for Argus-Panoptes due to:
- Separate Ollama lifecycle (model loading)
- Independent scaling of components
- Easier debugging and logging
- Standard microservices patterns

### Docker Compose Configuration

```yaml
# docker-compose.yml
version: '3.8'

services:
  coordinator:
    build:
      context: .
      dockerfile: docker/Dockerfile.coordinator
    container_name: argus-coordinator
    ports:
      - "8080:8080"
    environment:
      - RUST_LOG=info
      - OLLAMA_HOST=http://ollama:11434
      - LANCEDB_PATH=/data/lancedb
    volumes:
      - lancedb-data:/data/lancedb
      - ./config:/app/config:ro
    depends_on:
      ollama:
        condition: service_healthy
    networks:
      - argus-network
    restart: unless-stopped

  pty-mcp-server:
    build:
      context: .
      dockerfile: docker/Dockerfile.pty-mcp
    container_name: argus-pty-mcp
    ports:
      - "8081:8081"
    environment:
      - RUST_LOG=info
    volumes:
      # Mount host directories for PTY access to repos
      - ${HOME}/projects:/workspace:rw
    networks:
      - argus-network
    restart: unless-stopped

  ollama:
    image: ollama/ollama:latest
    container_name: argus-ollama
    ports:
      - "11434:11434"
    volumes:
      - ollama-data:/root/.ollama
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:11434/"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s
    networks:
      - argus-network
    restart: unless-stopped

volumes:
  lancedb-data:
    driver: local
  ollama-data:
    driver: local

networks:
  argus-network:
    driver: bridge
```

### Dockerfile Examples

**Coordinator (Multi-stage build):**

```dockerfile
# docker/Dockerfile.coordinator
FROM rust:1.85 AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release -p panoptes-api

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false appuser

COPY --from=builder /app/target/release/panoptes-api /usr/local/bin/

USER appuser

EXPOSE 8080

CMD ["panoptes-api", "--config", "/app/config/config.toml", "--memory"]
```

**PTY-MCP Server:**

```dockerfile
# docker/Dockerfile.pty-mcp
FROM rust:1.85 AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p panoptes-pty-mcp --bin pty-mcp-server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Claude CLI would need to be installed here
# RUN npm install -g @anthropic-ai/claude-cli

COPY --from=builder /app/target/release/pty-mcp-server /usr/local/bin/

EXPOSE 8081

CMD ["pty-mcp-server"]
```

### Volume Mounts for Persistence

| Volume | Purpose | Mount Type |
|--------|---------|------------|
| `lancedb-data` | Vector database storage | Named volume (fast) |
| `ollama-data` | Downloaded models | Named volume (fast) |
| `./config` | Configuration files | Bind mount (convenient) |
| `~/projects` | Host repos for PTY | Bind mount (required) |

**Performance Note:** Named volumes are stored on the Linux side and are much faster than bind mounts. Use bind mounts only when host access is required.

### Build and Run

```bash
# Build all images
docker compose build

# Start services
docker compose up -d

# Pull Ollama models
docker exec argus-ollama ollama pull lfm2:24b

# View logs
docker compose logs -f coordinator

# Stop services
docker compose down

# Stop and remove volumes (clean slate)
docker compose down -v
```

### Pros

- Balance of simplicity and structure
- Industry-standard tooling
- Easy to understand and debug
- Works identically on any Docker host
- Simple scaling (multiple instances)
- Compose file is self-documenting

### Cons

- Still container overhead vs native
- Bind mount performance for PTY workspaces
- Networking complexity for PTY sessions
- Claude CLI installation in container

---

## Recommendation

### For Development: Docker Compose

**Why Docker Compose for development:**

1. **Fast iteration** - `docker compose up` is instant with OrbStack
2. **Isolated environment** - No pollution of host system
3. **Reproducible** - Same environment for all developers
4. **Simple debugging** - `docker logs`, `docker exec`
5. **Portable** - Works on any Docker host

**Setup time:** ~30 minutes

### For "Production" (Single User): Linux VM

**Why Linux VM for single-user production:**

1. **Native Ollama performance** - No container overhead
2. **Simple operations** - Standard Linux administration
3. **Direct debugging** - SSH in and inspect
4. **Persistent state** - Natural file system
5. **Systemd integration** - Service management

**Setup time:** ~1 hour

### Migration Path

```
Development (Docker Compose)
        ↓
    Local Testing
        ↓
Single-User Production (Linux VM)
        ↓
    [Future: Team/Scale]
        ↓
Production Kubernetes (external cluster)
```

### Decision Matrix

| Factor | Docker Compose | Linux VM | Kubernetes |
|--------|---------------|----------|------------|
| Setup complexity | Low | Low | High |
| Learning curve | Low | Low | High |
| Resource overhead | Low | Lowest | Medium |
| Debugging ease | High | Highest | Medium |
| Production-ready | Medium | High | Highest |
| Team scalability | Low | Low | High |
| Single-user fit | Good | Best | Overkill |

### Kubernetes: When to Consider

Consider Kubernetes only if:
- Deploying to a team of 5+ users
- Running on actual server infrastructure
- Need auto-scaling capabilities
- Production k8s is in the roadmap
- Team has k8s expertise

---

## Quick Start Guides

### Quick Start: Docker Compose (Recommended for Dev)

```bash
# 1. Install OrbStack
brew install orbstack

# 2. Clone repository
cd ~/github
git clone https://github.com/jtmcg3/argus-panoptes.git
cd argus-panoptes

# 3. Create docker directory
mkdir -p docker

# 4. Create Dockerfiles (see examples above)
# ... create docker/Dockerfile.coordinator
# ... create docker/Dockerfile.pty-mcp

# 5. Create docker-compose.yml (see example above)

# 6. Start services
docker compose up -d

# 7. Pull Ollama model
docker exec argus-ollama ollama pull lfm2:24b

# 8. Verify
curl http://localhost:8080/health
```

### Quick Start: Linux VM

```bash
# 1. Install OrbStack
brew install orbstack

# 2. Create and configure VM
orb create ubuntu:24.04 argus-vm
orb shell argus-vm

# Inside VM:
# 3. Install dependencies
sudo apt update && sudo apt install -y \
    build-essential pkg-config libssl-dev curl git

# 4. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# 5. Install Ollama
curl -fsSL https://ollama.com/install.sh | sh
sudo systemctl enable --now ollama
ollama pull lfm2:24b

# 6. Clone and build
git clone https://github.com/jtmcg3/argus-panoptes.git
cd argus-panoptes
cargo build --release

# 7. Create systemd service (optional)
# ... see systemd service example below
```

### Systemd Service Example (Linux VM)

```ini
# /etc/systemd/system/argus-coordinator.service
[Unit]
Description=Argus-Panoptes Coordinator
After=network.target ollama.service
Requires=ollama.service

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/argus-panoptes
Environment=RUST_LOG=info
Environment=OLLAMA_HOST=http://localhost:11434
ExecStart=/home/ubuntu/argus-panoptes/target/release/panoptes-api --config /home/ubuntu/argus-panoptes/config/config.toml --memory
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

---

## OrbStack Configuration

### Recommended Settings

```bash
# Set memory limit (adjust based on your Mac's RAM)
orb config set memory_mib 8192

# Set CPU limit (cores)
orb config set cpu 4

# View current config
orb config show
```

### Accessing Volumes from Mac

```bash
# Docker volumes
ls ~/OrbStack/docker/volumes/

# Linux VM files
ls ~/OrbStack/machines/argus-vm/

# Or use Finder: OrbStack tab in sidebar
```

### Network Access

| Service Type | Access Method |
|--------------|---------------|
| Docker ports | `localhost:PORT` |
| Linux VM | `vm-name.orb.local` or `ssh vm-name@orb` |
| Kubernetes | `*.k8s.orb.local` |

---

## References

- [OrbStack Documentation](https://docs.orbstack.dev/)
- [OrbStack Kubernetes Docs](https://docs.orbstack.dev/kubernetes/)
- [OrbStack Linux Machines](https://docs.orbstack.dev/machines/)
- [OrbStack File Sharing](https://docs.orbstack.dev/docker/file-sharing)
- [Ollama Documentation](https://docs.ollama.com/linux)
- [LanceDB Rust SDK](https://docs.rs/lancedb/latest/lancedb/)
- [Docker Compose Best Practices](https://docs.docker.com/guides/rust/develop/)

---

## Changelog

- **2026-02-24**: Updated binary name to panoptes-api, added config/security notes, Ollama host.orb.internal note
- **2026-02-21**: Initial document created
