# Simaris M5 migration runbook

Upgrade from pre-M5 (FTS5-only) to M5 (hybrid: FTS5 + lance KNN + tantivy + RRF).

## Audience

End user with an existing `~/.simaris/sanctuary.db`. ≤ 5 minutes hands-on, ≤ 30 minutes wall time for backfill on Apple Silicon.

## Steps

### 1. Backup

```bash
cp ~/.simaris/sanctuary.db ~/.simaris/backups/sanctuary.db.pre-m5.$(date +%Y%m%d).db
```

### 2. Install M5

```bash
brew upgrade simonspoon/tap/simaris   # or `brew install` if not yet tapped
simaris search --help | grep -q -- '--no-vec' && echo "M5 wired" || echo "STILL PRE-M5"
```

If `STILL PRE-M5`: `brew uninstall simaris && brew install simonspoon/tap/simaris`. The installed binary date must be ≥ the M5 release tag date. (Lesson `019df607-0ed1`.)

### 3. Backfill vec dataset

```bash
ollama serve &              # bge-m3 backend (pull once: `ollama pull bge-m3`)
simaris vec backfill        # defaults: --model bge-m3, --backend lance
```

Idempotent — re-runs are safe. ~15-30 min on Apple Silicon for ~3500 units. Dataset lands at `$SIMARIS_VEC_DIR` (default: `~/.simaris/vec/bge-m3/`).

### 4. Verify hybrid

```bash
simaris search "memory architect" --debug 2>&1 | grep -E '(hybrid|lance|fts5)'
```

Expect log lines indicating both legs ran. If only FTS5: check `SIMARIS_VEC_DIR` matches the directory created in step 3 (lesson `019df607-4ef5`).

### 5. Hook integration (optional)

Already-installed UserPromptSubmit hooks resolve `simaris` via `which`. Nothing to update — the new binary is picked up automatically. To verify:

```bash
SIMARIS_HOOK_FALLBACK=fts5 echo '{"prompt":"test"}' | bash ~/.claude/hook-scripts/simaris-procedures.sh
```

`SIMARIS_HOOK_FALLBACK=fts5` forces the FTS5 path. Unset it (default) to use hybrid.

## Rollback

### Per-invocation rollback

```bash
simaris search QUERY --no-vec   # FTS5-only, identical to pre-M5
```

### Hook-time rollback

```bash
export SIMARIS_HOOK_FALLBACK=fts5   # in shell rc; hook uses --no-vec
```

### Full revert (binary-level)

```bash
brew uninstall simaris
# install pre-M5 release (e.g., v0.5.2):
brew install simonspoon/tap/simaris@0.5.2   # if pinned tap exists
# OR cargo install --git https://github.com/simonspoon/simaris --tag v0.5.2
cp ~/.simaris/backups/sanctuary.db.pre-m5.* ~/.simaris/sanctuary.db
```

The lance dataset under `~/.simaris/vec/` is additive — leaving it in place does NOT corrupt FTS5 reads. Pre-M5 binary ignores it entirely.

## Verifier checklist

| step  | command                                                                | expect                |
|-------|------------------------------------------------------------------------|-----------------------|
| 1     | `ls ~/.simaris/backups/`                                               | backup file present   |
| 2     | `simaris search --help \| grep -- --no-vec`                            | non-empty             |
| 3     | `ls ~/.simaris/vec/bge-m3/units.lance/`                                | lance fragments       |
| 4     | `simaris search Q --debug 2>&1 \| grep hybrid`                         | hybrid path used      |
| 5     | `simaris search Q --no-vec --json` vs pre-M5 `simaris search Q --json` | identical id list     |

Step 5 is the rollback-parity guarantee: M5 `--no-vec` returns the same unit IDs in the same order as pre-M5 default. Verified across 44 queries in M5.5 (atom `simaris-m5-5-rollback-parity-2026-05-04`).

## Known caveats

- `SIMARIS_VEC_DIR` mismatch silently falls back to FTS5 (lesson `019df607-4ef5`). Default path is `~/.simaris/vec/bge-m3/`.
- Backfill on Intel CPU is the throughput bottleneck (~6h). Apple Silicon ~30 min.
- Disk: ~21 MB lance + ~2 MB tantivy per ~3500 units. Linear in corpus.
