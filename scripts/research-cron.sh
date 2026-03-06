#!/usr/bin/env bash
set -euo pipefail

# Periodically run research topics via JSON-RPC message/send.
#
# Usage:
#   ./scripts/research-cron.sh [topics_file]
#
# Env vars:
#   HOST=localhost
#   PORT=9001
#   OUT_DIR=./logs/research
#   TOPICS_FILE=./scripts/research-topics.txt
#   MAX_TOPICS=0          # 0 means no limit
#   REQUEST_TIMEOUT=120   # seconds
#
# Notes:
# - This targets the research agent directly (port 9001) to avoid coordinator triage variance.
# - One JSON result file is written per topic.

HOST="${HOST:-localhost}"
PORT="${PORT:-9001}"
OUT_DIR="${OUT_DIR:-./logs/research}"
TOPICS_FILE="${1:-${TOPICS_FILE:-./scripts/research-topics.txt}}"
MAX_TOPICS="${MAX_TOPICS:-0}"
REQUEST_TIMEOUT="${REQUEST_TIMEOUT:-120}"

if [[ ! -f "${TOPICS_FILE}" ]]; then
  echo "Topics file not found: ${TOPICS_FILE}" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

timestamp_now() {
  date -u +"%Y%m%dT%H%M%SZ"
}

slugify() {
  local input="$1"
  echo "${input}" \
    | tr '[:upper:]' '[:lower:]' \
    | tr -cs 'a-z0-9' '-' \
    | sed -E 's/^-+//; s/-+$//' \
    | cut -c1-64
}

send_research() {
  local topic="$1"
  local url="http://${HOST}:${PORT}/"
  local id
  id="research-$(timestamp_now)"

  local payload
  payload="$(printf '{"jsonrpc":"2.0","id":"%s","method":"message/send","params":{"message":{"role":"user","parts":[{"type":"text","text":"%s"}]}}}' \
    "${id}" \
    "$(printf '%s' "${topic}" | sed 's/\\/\\\\/g; s/"/\\"/g')")"

  curl -fsS --max-time "${REQUEST_TIMEOUT}" \
    -H "content-type: application/json" \
    -d "${payload}" \
    "${url}"
}

echo "Running scheduled research against ${HOST}:${PORT}"
echo "Topics file: ${TOPICS_FILE}"
echo "Output dir:  ${OUT_DIR}"

count=0
while IFS= read -r topic || [[ -n "${topic}" ]]; do
  # Skip blank lines and comments.
  if [[ -z "${topic// }" || "${topic}" =~ ^[[:space:]]*# ]]; then
    continue
  fi

  count=$((count + 1))
  if [[ "${MAX_TOPICS}" -gt 0 && "${count}" -gt "${MAX_TOPICS}" ]]; then
    echo "Reached MAX_TOPICS=${MAX_TOPICS}, stopping."
    break
  fi

  ts="$(timestamp_now)"
  slug="$(slugify "${topic}")"
  [[ -z "${slug}" ]] && slug="topic-${count}"

  out_json="${OUT_DIR}/${ts}_${slug}.json"
  out_txt="${OUT_DIR}/${ts}_${slug}.txt"
  tmp_json="${out_json}.tmp"

  echo "[${count}] researching: ${topic}"
  if send_research "${topic}" >"${tmp_json}"; then
    mv "${tmp_json}" "${out_json}"
    if command -v jq >/dev/null 2>&1; then
      jq -r '.result.artifacts[0].parts[]? | select(.type=="text") | .text' "${out_json}" >"${out_txt}" || true
    fi
    echo "  -> wrote ${out_json}"
  else
    rm -f "${tmp_json}"
    echo "  -> request failed for topic: ${topic}" >&2
  fi
done <"${TOPICS_FILE}"

echo "Scheduled research run complete."
