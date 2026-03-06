#!/usr/bin/env bash
set -euo pipefail

# Build Linux release binaries from macOS (or any host) via Docker.
#
# Defaults are tuned for low-memory environments.
# Override via env vars when needed:
#   PLATFORM=linux/amd64|linux/arm64
#   RUST_IMAGE=rust:1.91-bookworm
#   CARGO_BUILD_JOBS=1
#   RELEASE_LTO=false
#   RELEASE_CODEGEN_UNITS=16
#   EXTRA_CARGO_ARGS="--locked"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required but not found in PATH" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

PLATFORM="${PLATFORM:-linux/arm64}"
RUST_IMAGE="${RUST_IMAGE:-rust:1.91-bookworm}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
RELEASE_LTO="${RELEASE_LTO:-false}"
RELEASE_CODEGEN_UNITS="${RELEASE_CODEGEN_UNITS:-16}"
EXTRA_CARGO_ARGS="${EXTRA_CARGO_ARGS:-}"

DOCKER_TTY_FLAGS=(-i)
if [[ -t 0 && -t 1 ]]; then
  DOCKER_TTY_FLAGS=(-it)
fi

HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

echo "Building in Docker"
echo "  platform: ${PLATFORM}"
echo "  image:    ${RUST_IMAGE}"
echo "  repo:     ${REPO_ROOT}"

docker run --rm "${DOCKER_TTY_FLAGS[@]}" \
  --platform "${PLATFORM}" \
  -e HOST_UID="${HOST_UID}" \
  -e HOST_GID="${HOST_GID}" \
  -e EXTRA_CARGO_ARGS="${EXTRA_CARGO_ARGS}" \
  -v "${REPO_ROOT}":/work \
  -w /work \
  "${RUST_IMAGE}" bash -c "
    set -euo pipefail
    apt-get update
    apt-get install -y pkg-config libssl-dev protobuf-compiler
    cargo --version
    rustc --version
    if [[ -n \"\${EXTRA_CARGO_ARGS}\" ]]; then
      # Intentionally split EXTRA_CARGO_ARGS into cargo flags.
      # shellcheck disable=SC2086
      CARGO_BUILD_JOBS='${CARGO_BUILD_JOBS}' CARGO_PROFILE_RELEASE_LTO='${RELEASE_LTO}' CARGO_PROFILE_RELEASE_CODEGEN_UNITS='${RELEASE_CODEGEN_UNITS}' cargo build --workspace --release \${EXTRA_CARGO_ARGS}
    else
      CARGO_BUILD_JOBS='${CARGO_BUILD_JOBS}' CARGO_PROFILE_RELEASE_LTO='${RELEASE_LTO}' CARGO_PROFILE_RELEASE_CODEGEN_UNITS='${RELEASE_CODEGEN_UNITS}' cargo build --workspace --release
    fi
    chown -R \"\${HOST_UID}:\${HOST_GID}\" /work/target
  "

echo
echo "Linux release build complete: ${REPO_ROOT}/target/release"
