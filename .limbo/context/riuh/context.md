## Approach
1. Add ScanResult struct to db.rs grouping five vecs (low_confidence, negative_marks, contradictions, orphans, stale)
2. Add ContradictionPair struct for contradiction pairs (from_id < to_id dedup)
3. Add pub fn scan(conn, stale_days) -> Result<ScanResult> with five SQL queries:
   - Low confidence: SELECT where confidence < 0.6 (const LOW_CONFIDENCE_THRESHOLD)
   - Negative marks: NOT EXISTS on marks table where kind IN ('wrong','outdated') — query marks rows, not confidence
   - Contradictions: JOIN links where relationship='contradicts', dedup via from_id < to_id
   - Orphans: NOT EXISTS on both links.from_id AND links.to_id
   - Stale: created < datetime('now', '-N days') AND NOT EXISTS in marks
4. Add Scan variant to Command enum with --stale-days optional arg (default 90)
5. Add print_scan(result, json) to display.rs — single entry point, JSON via serde, plain text with section headers
6. Reuse existing patterns: --json flag, content truncation, row_to_unit

## Verify
Seed units with varying ages/confidence. Run scan. Verify stale items flagged, contradictions surfaced.

## Result
Self-maintaining knowledge store. No human in the maintenance loop.

## Outcome
scan command implemented: simaris scan runs five LLM-free health checks (low confidence, negative marks, contradictions, orphans, stale). Supports --json and --stale-days. 11 unit tests, all 72 tests pass. SQL injection fix applied after review. Integration tests deferred. Commit 4ca7fb8.

## AcceptanceCriteria
1. scan on empty store exits 0, prints 'No issues found'
2. Reports units with confidence < 0.6 (low-confidence section)
3. Reports units with any wrong/outdated marks (negative-marks section)
4. Reports both unit IDs when a contradicts link exists (contradictions section)
5. Reports units with zero links as orphans
6. Reports units with created > 90 days ago and zero marks as stale
7. --json outputs valid JSON with keys: low_confidence, negative_marks, contradictions, orphans, stale
8. --stale-days flag overrides default 90-day threshold
9. Per-check sections — a unit can appear in multiple sections
10. No LLM calls — pure SQL, completes in under 2s on 1000 units

## ScopeOut
Semantic contradiction detection (LLM), verified field, type-based gap inference, auto-fix suggestions

## AffectedAreas
src/db.rs (new queries + ScanResult struct), src/main.rs (Command::Scan variant), src/display.rs (print_scan), tests/integration.rs (scan tests)

## TestStrategy
Unit tests in db.rs (memory_db): empty store, low-confidence + boundary, negative marks (wrong + outdated), contradictions, orphans, stale + stale-with-mark, stale-days override, multi-section overlap.
Integration tests in integration.rs (TestEnv): empty store output, each check type via CLI, --json shape validation, --stale-days flag, timing guard (<2s).
Run: cargo fmt --check && cargo build && cargo test

## Risks
1. Contradictions are directional — dedup with from_id < to_id to avoid duplicate pairs in output
2. Negative marks vs confidence: query marks table rows, not confidence value (a unit can recover confidence but still have wrong marks)
3. Orphans need both link directions checked (from_id AND to_id)
4. Stale check: created column is TEXT but ISO-8601 lexicographic comparison is safe for SQLite datetime
5. Five queries not wrapped in transaction — acceptable for read-only personal CLI
6. ContradictionPair needs different struct from Vec<Unit> — JSON schema: {from: {id, content}, to: {id, content}}

## Report
TL implemented scan command across db.rs, main.rs, display.rs. 11 unit tests added, all pass. Integration tests not written (skill enforcement hook blocked — follow-up needed). Design note: stale and negative_marks are mutually exclusive (stale requires zero marks, negative_marks requires marks to exist). cargo fmt clean, cargo build no warnings, 72 tests pass.

## Notes
### 2026-04-08T15:22:25-04:00
Investigation findings:
- confidence field (0.0-1.0) and marks table exist from mark command
- contradicts is already a link relationship — free structural detection
- verified field is dead (never set to true) — exclude from v1
- updated column never written post-insert — use created + mark absence for staleness
- Five LLM-free checks: low-confidence, negative marks, contradictions, orphans, stale
- Out of scope: semantic contradiction (LLM/O(n²)), verified field, type-based gaps, auto-fix

### 2026-04-08T15:27:05-04:00
TEST STRATEGY RATIONALE: Strategy is structured around the two existing test layers: memory_db() unit tests in db.rs for pure SQL logic, and TestEnv integration tests in tests/integration.rs for CLI behavior. Unit tests verify each SQL query in isolation — boundary conditions (confidence exactly at 0.60), stale threshold override, mark-presence disqualifying staleness, and a multi-section membership test that confirms a single unit can appear in all applicable sections simultaneously. Integration tests verify the CLI surface: exit codes, human-readable section headers with unit IDs, JSON shape (all five required keys present as arrays), and the --stale-days flag. The timing assertion in test_scan_no_llm_calls (under 2s on seeded data) acts as a regression guard against accidental LLM calls — no mocking required because the check is structural. Stale-days testing via SQL UPDATE inside the TestEnv SQLite file is the simplest approach; no time-mocking or special helpers needed. The contradictions test verifies both IDs appear (not just the source side of the link), matching AC #4 exactly. The multi-section unit test is the single most important coverage item — it proves sections are independent and a unit is not deduped out of any section it qualifies for.

### 2026-04-08T15:35:15-04:00
REVIEW FINDINGS:

### 0001-01-01T00:00:00Z
- [src/db.rs:472-478] SQL injection risk via format! in stale query. While stale_days is u32, format! is unnecessary and violates security best practices. Use parameterized query: WHERE created < datetime('now', '-?1 days') with params![stale_days].

### 0001-01-01T00:00:00Z
- [tests/integration.rs] Missing integration tests noted in TL report. AC #1,7,8,10 require CLI validation: empty store output format, --json schema, --stale-days flag, timing assertion.

### 0001-01-01T00:00:00Z
- [src/db.rs:36] LOW_CONFIDENCE_THRESHOLD constant is well-placed and matches AC #2 requirement.
- [src/db.rs:441-447] Contradictions deduplication logic (from_id < to_id) correctly handles bidirectional links.
- [src/db.rs:464-465] Orphan detection correctly checks both link directions (from_id AND to_id).
- [src/db.rs:1066-1099] Multi-section test correctly validates mutual exclusivity of stale/negative_marks and section independence.
- [src/display.rs:171-181] Content truncation at 80 chars with Unicode-aware char_indices() is robust.
- All 11 unit tests pass, covering boundary conditions and edge cases comprehensively.

### 2026-04-08T15:35:17-04:00
VERDICT:review:REQUEST_CHANGES
