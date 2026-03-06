#!/usr/bin/env bash
set -euo pipefail

# Deploy release artifacts to a Linux VM.
#
# Usage:
#   ./scripts/deploy-vm.sh <user@host> [remote_dir]
#
# Example:
#   ./scripts/deploy-vm.sh jim@argus-vm /home/jim/projects/argus-panoptes
#
# Optional env vars:
#   SSH_OPTS="-i ~/.ssh/id_ed25519"
#   RESTART_CMD="<your restart command>"
#   RESTART_STRICT=1   # fail deploy if RESTART_CMD fails (default: 0)

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <user@host> [remote_dir]" >&2
  exit 1
fi

TARGET_HOST="$1"
REMOTE_DIR="${2:-/home/jim/projects/argus-panoptes}"
SSH_OPTS="${SSH_OPTS:-}"
RESTART_CMD="${RESTART_CMD:-}"
RESTART_STRICT="${RESTART_STRICT:-0}"
RSYNC_RSH="ssh${SSH_OPTS:+ ${SSH_OPTS}}"

ssh_cmd() {
  if [[ -n "${SSH_OPTS}" ]]; then
    # Intentionally allow SSH_OPTS word-splitting into ssh flags.
    # shellcheck disable=SC2086
    ssh ${SSH_OPTS} "$@"
  else
    ssh "$@"
  fi
}

scp_cmd() {
  if [[ -n "${SSH_OPTS}" ]]; then
    # Intentionally allow SSH_OPTS word-splitting into scp flags.
    # shellcheck disable=SC2086
    scp ${SSH_OPTS} "$@"
  else
    scp "$@"
  fi
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
LOCAL_RELEASE_DIR="${REPO_ROOT}/target/release"

BINARIES=(
  panoptes-coordinator
  panoptes-telegram
  panoptes-research
  panoptes-writing
  panoptes-planning
  panoptes-review
  panoptes-testing
  panoptes-coding
  pty-mcp-server
)

for bin in "${BINARIES[@]}"; do
  if [[ ! -x "${LOCAL_RELEASE_DIR}/${bin}" ]]; then
    echo "Missing release binary: ${LOCAL_RELEASE_DIR}/${bin}" >&2
    echo "Run ./scripts/build-linux.sh (or cargo build --release) first." >&2
    exit 1
  fi
done

echo "Deploying to ${TARGET_HOST}:${REMOTE_DIR}"
ssh_cmd "${TARGET_HOST}" \
  "mkdir -p '${REMOTE_DIR}/target/release' '${REMOTE_DIR}/config' '${REMOTE_DIR}/scripts'"

if command -v rsync >/dev/null 2>&1; then
  echo "Copying binaries with rsync..."
  rsync -av -e "${RSYNC_RSH}" \
    "${LOCAL_RELEASE_DIR}/panoptes-coordinator" \
    "${LOCAL_RELEASE_DIR}/panoptes-telegram" \
    "${LOCAL_RELEASE_DIR}/panoptes-research" \
    "${LOCAL_RELEASE_DIR}/panoptes-writing" \
    "${LOCAL_RELEASE_DIR}/panoptes-planning" \
    "${LOCAL_RELEASE_DIR}/panoptes-review" \
    "${LOCAL_RELEASE_DIR}/panoptes-testing" \
    "${LOCAL_RELEASE_DIR}/panoptes-coding" \
    "${LOCAL_RELEASE_DIR}/pty-mcp-server" \
    "${TARGET_HOST}:${REMOTE_DIR}/target/release/"

  echo "Copying runtime config and helper scripts with rsync..."
  rsync -av -e "${RSYNC_RSH}" \
    "${REPO_ROOT}/config/default.toml" \
    "${TARGET_HOST}:${REMOTE_DIR}/config/default.toml"
  rsync -av -e "${RSYNC_RSH}" \
    "${REPO_ROOT}/start-agents.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/"
  rsync -av -e "${RSYNC_RSH}" \
    "${REPO_ROOT}/scripts/smoke.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/"
  rsync -av -e "${RSYNC_RSH}" \
    "${REPO_ROOT}/scripts/research-cron.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/"
  rsync -av -e "${RSYNC_RSH}" \
    "${REPO_ROOT}/scripts/research-topics.txt.example" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/"
else
  echo "rsync not found, falling back to scp..."
  scp_cmd \
    "${LOCAL_RELEASE_DIR}/panoptes-coordinator" \
    "${LOCAL_RELEASE_DIR}/panoptes-telegram" \
    "${LOCAL_RELEASE_DIR}/panoptes-research" \
    "${LOCAL_RELEASE_DIR}/panoptes-writing" \
    "${LOCAL_RELEASE_DIR}/panoptes-planning" \
    "${LOCAL_RELEASE_DIR}/panoptes-review" \
    "${LOCAL_RELEASE_DIR}/panoptes-testing" \
    "${LOCAL_RELEASE_DIR}/panoptes-coding" \
    "${LOCAL_RELEASE_DIR}/pty-mcp-server" \
    "${TARGET_HOST}:${REMOTE_DIR}/target/release/"
  scp_cmd "${REPO_ROOT}/config/default.toml" \
    "${TARGET_HOST}:${REMOTE_DIR}/config/default.toml"
  scp_cmd "${REPO_ROOT}/start-agents.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/"
  scp_cmd "${REPO_ROOT}/scripts/smoke.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/smoke.sh"
  scp_cmd "${REPO_ROOT}/scripts/research-cron.sh" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/research-cron.sh"
  scp_cmd "${REPO_ROOT}/scripts/research-topics.txt.example" \
    "${TARGET_HOST}:${REMOTE_DIR}/scripts/research-topics.txt.example"
fi

ssh_cmd "${TARGET_HOST}" \
  "chmod +x '${REMOTE_DIR}/start-agents.sh' '${REMOTE_DIR}/scripts/smoke.sh' '${REMOTE_DIR}/scripts/research-cron.sh' '${REMOTE_DIR}/target/release/'panoptes-* '${REMOTE_DIR}/target/release/pty-mcp-server'"

if [[ -n "${RESTART_CMD}" ]]; then
  echo "Running remote restart command..."
  if ssh_cmd "${TARGET_HOST}" "cd '${REMOTE_DIR}' && ${RESTART_CMD}"; then
    :
  else
    if [[ "${RESTART_STRICT}" == "1" ]]; then
      echo "Restart command failed and RESTART_STRICT=1; aborting deploy." >&2
      exit 1
    fi
    echo "Restart command failed; continuing because RESTART_STRICT=0." >&2
  fi
fi

echo
echo "Deploy complete."
echo "Remote run:"
echo "  ssh ${TARGET_HOST} 'cd ${REMOTE_DIR} && ./start-agents.sh'"
