#!/usr/bin/env bash
set -euo pipefail

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

agent_port() {
  case "$1" in
    research) printf '%s\n' "9001" ;;
    writing) printf '%s\n' "9002" ;;
    planning) printf '%s\n' "9003" ;;
    review) printf '%s\n' "9004" ;;
    testing) printf '%s\n' "9005" ;;
    coding) printf '%s\n' "9006" ;;
    *) return 1 ;;
  esac
}

HOST="${HOST:-localhost}"
COORD_PORT="${COORD_PORT:-$(detect_coord_port)}"

FAILURES=0

log() { printf '%s\n' "$*"; }
ok() { printf '[OK] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*"; }
fail() { printf '[FAIL] %s\n' "$*"; FAILURES=$((FAILURES + 1)); }

check_health() {
  local name="$1"
  local port="$2"
  local url="http://${HOST}:${port}/health"

  if curl -fsS --max-time 3 "$url" >/dev/null 2>&1; then
    ok "${name} health (${url})"
  else
    fail "${name} health (${url})"
  fi
}

check_agent_card() {
  local expected_name="$1"
  local port="$2"
  local url="http://${HOST}:${port}/.well-known/agent.json"

  local body
  if ! body="$(curl -fsS --max-time 5 "$url" 2>/dev/null)"; then
    fail "agent card unreachable (${url})"
    return
  fi

  if grep -q "\"name\":\"${expected_name}\"" <<<"$body"; then
    ok "agent card ${expected_name} (${url})"
  else
    fail "agent card name mismatch at ${url}"
  fi
}

check_send() {
  local url="http://${HOST}:${COORD_PORT}/"
  local payload
  payload='{"jsonrpc":"2.0","id":"smoke-send","method":"message/send","params":{"message":{"role":"user","parts":[{"type":"text","text":"Plan my tasks for today"}]}}}'

  local body
  if ! body="$(curl -fsS --max-time 20 -H "content-type: application/json" -d "$payload" "$url" 2>/dev/null)"; then
    fail "coordinator message/send request (${url})"
    return
  fi

  if grep -q '"jsonrpc":"2.0"' <<<"$body" && grep -q '"state":"completed"' <<<"$body"; then
    ok "coordinator message/send"
  else
    fail "coordinator message/send response shape"
  fi
}

check_stream() {
  local url="http://${HOST}:${COORD_PORT}/"
  local payload
  payload='{"jsonrpc":"2.0","id":"smoke-stream","method":"message/stream","params":{"message":{"role":"user","parts":[{"type":"text","text":"Plan my tasks for today"}]}}}'

  local stream
  if ! stream="$(curl -fsS -N --max-time 25 -H "content-type: application/json" -d "$payload" "$url" 2>/dev/null)"; then
    fail "coordinator message/stream request (${url})"
    return
  fi

  local has_status=0
  local has_artifact=0
  local has_completed=0

  grep -q 'event: status' <<<"$stream" && has_status=1 || true
  grep -q 'event: artifact' <<<"$stream" && has_artifact=1 || true
  if grep -q '"state":"completed"' <<<"$stream" \
    || grep -q '\\"state\\":\\"completed\\"' <<<"$stream"; then
    has_completed=1
  fi

  if [[ "$has_status" -eq 1 && "$has_completed" -eq 1 ]]; then
    ok "coordinator message/stream"
    if [[ "$has_artifact" -eq 0 ]]; then
      warn "stream completed without explicit artifact event (result may be status-only)"
    fi
  else
    log "---- stream output begin ----"
    log "$stream"
    log "---- stream output end ----"
    fail "coordinator message/stream response shape"
  fi
}

main() {
  log "Running smoke checks against host=${HOST} coordinator_port=${COORD_PORT}"

  check_health "coordinator" "${COORD_PORT}"
  check_agent_card "panoptes-coordinator" "${COORD_PORT}"

  for name in research writing planning review testing coding; do
    local port
    if ! port="$(agent_port "$name")"; then
      fail "unknown agent '${name}'"
      continue
    fi
    check_health "$name" "$port"
    check_agent_card "panoptes-${name}" "$port"
  done

  check_send
  check_stream

  if [[ "$FAILURES" -gt 0 ]]; then
    log
    log "Smoke checks failed: ${FAILURES}"
    exit 1
  fi

  log
  log "Smoke checks passed."
}

main "$@"
