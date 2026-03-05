#!/usr/bin/env bash
set -euo pipefail

PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM

cargo build --workspace --release

for agent in research writing planning review testing coding coordinator; do
  ./target/release/panoptes-${agent} &
  PIDS+=($!)
  echo "Started panoptes-${agent} (PID $!)"
done

echo "All agents started. Press Ctrl+C to stop."
wait
