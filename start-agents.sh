#!/usr/bin/env bash
set -euo pipefail

PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM

detect_coord_port() {
  local config_path="${PANOPTES_CONFIG:-config/default.toml}"

  if [[ -f "$config_path" ]]; then
    local parsed
    parsed="$(sed -nE 's/^[[:space:]]*port[[:space:]]*=[[:space:]]*([0-9]+).*/\1/p' "$config_path" | head -n 1)"
    if [[ -n "$parsed" ]]; then
      printf '%s\n' "$parsed"
      return 0
    fi
  fi

  printf '%s\n' "8080"
}

COORD_PORT="$(detect_coord_port)"

SKIP_BUILD="${SKIP_BUILD:-0}"
if [[ "${SKIP_BUILD}" != "1" ]]; then
  cargo build --workspace --release
else
  echo "Skipping build (SKIP_BUILD=1)"
fi

wait_for_health() {
  local name="$1"
  local port="$2"
  local tries=60

  for ((i=1; i<=tries; i++)); do
    if curl -fsS "http://localhost:${port}/health" >/dev/null 2>&1; then
      echo "panoptes-${name} is healthy on :${port}"
      return 0
    fi
    sleep 0.5
  done

  echo "panoptes-${name} failed health check on :${port}" >&2
  return 1
}

start_agent() {
  local name="$1"
  ./target/release/panoptes-"${name}" &
  PIDS+=($!)
  echo "Started panoptes-${name} (PID $!)"
}

# Start specialist agents first.
for agent in research writing planning review testing coding; do
  start_agent "${agent}"
done

# Wait for specialists to report healthy before starting coordinator.
wait_for_health research 9001
wait_for_health writing 9002
wait_for_health planning 9003
wait_for_health review 9004
wait_for_health testing 9005
wait_for_health coding 9006

# Start coordinator last so discovery succeeds reliably.
start_agent coordinator
wait_for_health coordinator "$COORD_PORT"

echo "All agents started. Press Ctrl+C to stop."
wait
