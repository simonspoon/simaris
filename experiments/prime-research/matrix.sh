#!/usr/bin/env bash
# Run prime on every task × filter strategy, score against ground truth.
# Ground truth: each task has one expected domain tag; all 7 domain units
# should appear, 0 noise units.
set -euo pipefail

export SIMARIS_HOME="${SEED_HOME:-/tmp/simaris-prime-experiment}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/release/simaris"
OUT_DIR="$(dirname "$0")/results/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT_DIR"

# task|expected_domain_tag
declare -a TASKS=(
  "review a pull request|code-review"
  "cut a new release of a Rust CLI|rust-release"
  "debug a SolidJS freeze|solidjs"
  "add unit tests to a Rust function|rust-testing"
  "run a database schema migration|migration"
)

declare -a FILTERS=("none" "standard" "context")

printf "| Task | Filter | Recall | Noise | Latency |\n" | tee "$OUT_DIR/matrix.md"
printf "|------|--------|--------|-------|---------|\n" | tee -a "$OUT_DIR/matrix.md"

for entry in "${TASKS[@]}"; do
  task="${entry%%|*}"
  expected="${entry##*|}"
  for filter in "${FILTERS[@]}"; do
    start_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
    out=$("$BIN" prime "$task" --filter "$filter" --json 2>/dev/null || echo '{"sections":[],"unit_count":0}')
    end_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
    latency=$((end_ms - start_ms))
    echo "$out" > "$OUT_DIR/$(echo "$expected" | tr '/' '_')-$filter.json"
    scores=$(echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)
on, off = 0, 0
for s in r['sections']:
    for u in s['units']:
        if '$expected' in u['tags']:
            on += 1
        else:
            off += 1
print(f'{on}|{off}')
")
    IFS='|' read -r on off <<< "$scores"
    recall="$on/7"
    printf "| %-40s | %-8s | %-6s | %-5s | %5sms |\n" "$task" "$filter" "$recall" "$off" "$latency" | tee -a "$OUT_DIR/matrix.md"
  done
  printf "| | | | | |\n" | tee -a "$OUT_DIR/matrix.md"
done

echo
echo "Results saved to: $OUT_DIR"
