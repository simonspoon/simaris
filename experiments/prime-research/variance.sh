#!/usr/bin/env bash
# Run matrix N times, aggregate recall/noise per cell to measure LLM variance.
set -euo pipefail

export SIMARIS_HOME="${SEED_HOME:-/tmp/simaris-prime-experiment}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/release/simaris"
RUNS="${RUNS:-3}"
OUT_DIR="$(dirname "$0")/results/variance-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT_DIR"

declare -a TASKS=(
  "review a pull request|code-review"
  "cut a new release of a Rust CLI|rust-release"
  "debug a SolidJS freeze|solidjs"
  "add unit tests to a Rust function|rust-testing"
  "run a database schema migration|migration"
)
declare -a FILTERS=("standard" "context" "context-strict" "tag-vote")

echo "Runs per cell: $RUNS"
echo

printf "| Task | Filter | Recall (avg/min/max) | Noise (avg/min/max) |\n"
printf "|------|--------|----------------------|---------------------|\n"

for entry in "${TASKS[@]}"; do
  task="${entry%%|*}"
  expected="${entry##*|}"
  for filter in "${FILTERS[@]}"; do
    recall_vals=()
    noise_vals=()
    for i in $(seq 1 $RUNS); do
      out=$("$BIN" prime "$task" --filter "$filter" --json 2>/dev/null || echo '{"sections":[]}')
      echo "$out" > "$OUT_DIR/$(echo "$expected" | tr '/' '_')-$filter-run$i.json"
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
print(f'{on} {off}')
")
      read -r on off <<< "$scores"
      recall_vals+=("$on")
      noise_vals+=("$off")
    done

    # Compute avg/min/max (comma-join the arrays)
    recall_csv=$(IFS=,; echo "${recall_vals[*]}")
    noise_csv=$(IFS=,; echo "${noise_vals[*]}")
    stats=$(python3 -c "
recall = [int(x) for x in '$recall_csv'.split(',')]
noise = [int(x) for x in '$noise_csv'.split(',')]
r_str = f'{sum(recall)/len(recall):.1f}/{min(recall)}/{max(recall)}'
n_str = f'{sum(noise)/len(noise):.1f}/{min(noise)}/{max(noise)}'
print(f'{r_str}|{n_str}')
")
    IFS='|' read -r r_str n_str <<< "$stats"
    printf "| %-40s | %-8s | %-20s | %-19s |\n" "$task" "$filter" "$r_str" "$n_str"
  done
done

echo
echo "Results saved to: $OUT_DIR"
