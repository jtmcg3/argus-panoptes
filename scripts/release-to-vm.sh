#!/usr/bin/env bash
set -euo pipefail

# Build, deploy, optionally restart, and smoke test on a VM.
#
# Usage:
#   ./scripts/release-to-vm.sh <user@host> [remote_dir]
#
# Env vars:
#   SSH_OPTS="-i ~/.ssh/id_ed25519 -p 22"
#   RESTART_CMD="<your restart command>"
#   RESTART_STRICT=1            # fail if restart command fails (default: 0)
#   SKIP_BUILD=1                # skip Docker build stage
#   RUN_SMOKE=0                 # skip remote smoke test
#   SMOKE_WAIT_SECONDS=5        # wait before smoke (useful after restart)
#   COORD_PORT=18080            # optional override for remote smoke
#
# Build-related env vars are forwarded to build-linux.sh:
#   PLATFORM, RUST_IMAGE, CARGO_BUILD_JOBS, RELEASE_LTO,
#   RELEASE_CODEGEN_UNITS, EXTRA_CARGO_ARGS

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <user@host> [remote_dir]" >&2
  exit 1
fi

TARGET_HOST="$1"
REMOTE_DIR="${2:-/home/jim/projects/argus-panoptes}"
SSH_OPTS="${SSH_OPTS:-}"
RESTART_CMD="${RESTART_CMD:-}"
RESTART_STRICT="${RESTART_STRICT:-0}"
SKIP_BUILD="${SKIP_BUILD:-0}"
RUN_SMOKE="${RUN_SMOKE:-1}"
SMOKE_WAIT_SECONDS="${SMOKE_WAIT_SECONDS:-5}"
COORD_PORT="${COORD_PORT:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_SCRIPT="${SCRIPT_DIR}/build-linux.sh"
DEPLOY_SCRIPT="${SCRIPT_DIR}/deploy-vm.sh"

if [[ ! -x "${BUILD_SCRIPT}" ]]; then
  echo "Missing executable build script: ${BUILD_SCRIPT}" >&2
  exit 1
fi

if [[ ! -x "${DEPLOY_SCRIPT}" ]]; then
  echo "Missing executable deploy script: ${DEPLOY_SCRIPT}" >&2
  exit 1
fi

ssh_cmd() {
  if [[ -n "${SSH_OPTS}" ]]; then
    # Intentionally allow SSH_OPTS word-splitting into ssh flags.
    # shellcheck disable=SC2086
    ssh ${SSH_OPTS} "$@"
  else
    ssh "$@"
  fi
}

echo "Release pipeline target=${TARGET_HOST} remote_dir=${REMOTE_DIR}"

if [[ "${SKIP_BUILD}" != "1" ]]; then
  echo
  echo "==> Building linux release artifacts"
  (
    cd "${REPO_ROOT}"
    PLATFORM="${PLATFORM:-}" \
    RUST_IMAGE="${RUST_IMAGE:-}" \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-}" \
    RELEASE_LTO="${RELEASE_LTO:-}" \
    RELEASE_CODEGEN_UNITS="${RELEASE_CODEGEN_UNITS:-}" \
    EXTRA_CARGO_ARGS="${EXTRA_CARGO_ARGS:-}" \
      "${BUILD_SCRIPT}"
  )
else
  echo
  echo "==> Skipping build stage (SKIP_BUILD=1)"
fi

echo
echo "==> Deploying artifacts"
(
  cd "${REPO_ROOT}"
  SSH_OPTS="${SSH_OPTS}" RESTART_CMD="${RESTART_CMD}" RESTART_STRICT="${RESTART_STRICT}" "${DEPLOY_SCRIPT}" "${TARGET_HOST}" "${REMOTE_DIR}"
)

if [[ "${RUN_SMOKE}" == "1" ]]; then
  if [[ -n "${RESTART_CMD}" && "${SMOKE_WAIT_SECONDS}" -gt 0 ]]; then
    echo
    echo "==> Waiting ${SMOKE_WAIT_SECONDS}s before smoke test"
    sleep "${SMOKE_WAIT_SECONDS}"
  fi

  echo
  echo "==> Running remote smoke test"
  if [[ -n "${COORD_PORT}" ]]; then
    ssh_cmd "${TARGET_HOST}" "cd '${REMOTE_DIR}' && COORD_PORT='${COORD_PORT}' ./scripts/smoke.sh"
  else
    ssh_cmd "${TARGET_HOST}" "cd '${REMOTE_DIR}' && ./scripts/smoke.sh"
  fi
else
  echo
  echo "==> Skipping smoke stage (RUN_SMOKE=0)"
fi

echo
echo "Release pipeline complete."
