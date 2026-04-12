#!/usr/bin/env bash
# Validate prime filter strategies against the real production simaris db.
# No ground truth — this is a qualitative inspection.
set -euo pipefail

unset SIMARIS_HOME  # use prod default
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/release/simaris"
OUT_DIR="$(dirname "$0")/results/real-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT_DIR"

declare -a TASKS=(
  "review a pull request"
  "debug a SolidJS UI freeze"
  "write a Rust integration test"
  "cut a new release of a Rust CLI tool"
)

declare -a FILTERS=("standard" "context" "tag-vote")

for task in "${TASKS[@]}"; do
  slug=$(echo "$task" | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
  echo "=== $task ==="
  for filter in "${FILTERS[@]}"; do
    start_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
    out=$("$BIN" prime "$task" --filter "$filter" --json 2>/dev/null || echo '{"sections":[],"unit_count":0}')
    end_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
    latency=$((end_ms - start_ms))
    echo "$out" > "$OUT_DIR/$slug-$filter.json"
    count=$(echo "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["unit_count"])')
    printf "  %-14s  units=%-3s  latency=%sms\n" "$filter" "$count" "$latency"
  done
  echo
done
echo "Results saved to: $OUT_DIR"
