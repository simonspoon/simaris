## Approach
1. Add MarkKind ValueEnum (Used/Wrong/Outdated/Helpful) with as_str() — follows UnitType/Relationship pattern
2. Add Mark { id, kind } variant to Command enum in main.rs
3. Append marks table DDL + index to initialize() in db.rs (IF NOT EXISTS, idempotent)
4. Add delta constants in db.rs: WRONG=-0.2, HELPFUL=0.1, USED=0.05, OUTDATED=-0.1
5. Add add_mark(conn, unit_id, kind_str, delta) -> Result<f64> in db.rs — single transaction: verify unit exists, INSERT mark, UPDATE confidence with MAX(0.0, MIN(1.0, confidence+delta)), return new confidence
6. Add print_marked(id, kind, confidence, json) in display.rs — follows existing two-path pattern
7. Add match arm in main.rs dispatch — map MarkKind to delta, call add_mark, call print_marked
8. Unit tests in db.rs: test_mark_unit, test_mark_confidence_clamping, test_mark_nonexistent_unit, test_mark_cascade_delete
9. Integration tests: test_mark_command, test_mark_json_output, test_mark_nonexistent, test_mark_confidence_accumulation, test_mark_clamping, test_mark_no_kind

## Verify
Mark a unit as used, verify outcome logged. Mark as wrong, verify confidence drops.

## Result
Knowledge units carry feedback from real usage. Foundation for immune system scoring.

## Outcome
mark command implemented: simaris mark <id> --kind <used|wrong|outdated|helpful>. Appends to marks event log table, updates confidence on units with clamping [0.0, 1.0]. 4 unit tests + 6 integration tests, all 61 tests pass. Commit 0ec8a67.

## AcceptanceCriteria
1. simaris mark 1 --kind wrong exits 0, prints confirmation with updated confidence, appends row to marks table with unit_id/kind/timestamp
2. simaris mark 1 --kind helpful after --kind wrong shows cumulative confidence change (0.80 -> 0.90)
3. simaris mark 999 --kind used exits non-zero with 'not found' error
4. simaris --json mark 1 --kind outdated emits valid JSON with id, mark, confidence fields
5. simaris show 1 reflects updated confidence after marking
6. Confidence clamped to [0.0, 1.0] — 10x --wrong never goes negative, 20x --helpful never exceeds 1.0
7. simaris mark with no --kind exits non-zero with clap usage error
8. marks table exists in freshly initialized database

## ScopeOut
get_marks read path, show command displaying mark history, archive/delete side effects, confidence affecting search/ask ranking

## AffectedAreas
src/main.rs (Command enum, MarkKind enum, match dispatch), src/db.rs (initialize DDL, add_mark fn, delta constants, unit tests), src/display.rs (print_marked fn), tests/integration.rs (6 new tests)

## TestStrategy
Unit tests: cargo test test_mark — db.rs tests for add_mark correctness, clamping, nonexistent unit error, cascade delete. Integration tests: cargo test test_mark — CLI exit codes, stdout format, JSON output, cumulative confidence, clap errors. Build verification: cargo build && cargo test && cargo fmt --check

## Risks
Low: confidence float precision (use {:.2} formatting). Low: concurrent access (single-user CLI, acceptable). None: migration risk (IF NOT EXISTS is idempotent, confidence column pre-exists). Design: duplicate marks allowed intentionally (append-only event log for immune system).

## Report
4 files modified: main.rs (MarkKind enum + Command variant + dispatch), db.rs (marks table DDL + add_mark fn + 4 unit tests), display.rs (print_marked fn), integration.rs (6 tests). TL adapted plan for actual API signatures (add_unit params, TestEnv::new(name), positional content arg). cargo build clean, cargo test 61/61 pass, cargo fmt clean.

## Notes
### 2026-04-08T14:01:59-04:00
Investigation: confidence column already exists on units table (REAL NOT NULL DEFAULT 1.0). No migration needed. marks table is new — DDL goes in initialize() with IF NOT EXISTS pattern. Codebase uses ValueEnum pattern for enums (UnitType, Relationship). Confidence must be clamped to [0.0, 1.0].

### 2026-04-08T14:17:40-04:00
TEST STRATEGY RATIONALE: Strategy covers all 8 acceptance criteria with concrete tools and commands. Unit tests verify mark_unit() database function behavior in isolation using memory_db() helper. Integration tests verify CLI parsing, error handling, JSON output, and end-to-end workflow using TestEnv pattern. Each test specifies exact commands (cargo test test_name) and expected outcomes. Confidence clamping tested with boundary conditions. No test pollution - each test uses fresh TestEnv or memory_db. Strategy follows existing patterns: #[test] functions, TestEnv::run/run_ok assertions, memory_db() for unit tests.

### 2026-04-08T15:37:53-04:00
REVIEW FINDINGS (re-review — scan SQL injection fix):

### 0001-01-01T00:00:00Z
Primary concern (SQL injection in stale query) — RESOLVED

src/db.rs:472-480: The fix is correctly in place. stale_modifier is built via format!("-{stale_days} days") where stale_days is a u32 (compiler-enforced, cannot be attacker-supplied string data). This value is then passed as a bound parameter (?1) to datetime('now', ?1) — NOT interpolated into the SQL string literal. The SQL string is a compile-time constant. Pattern is correct.

No other format!-based SQL construction exists in the scan function.

All five scan sub-queries examined (low_confidence, negative_marks, contradictions, orphans, stale) use only compile-time SQL string literals in prepare() and params![...] bound parameters. No format! calls inside any SQL string.

All other format! usages in db.rs (lines 202, 292, 517, 556) are error message formatting and path construction — none appear in SQL strings.

FTS query path in ask.rs: sanitize_fts_query() wraps tokens in double-quotes via format!, but the resulting string is passed as a bound parameter to search_units (db.rs:237 params![query]) — not interpolated into SQL. Correct.

No new issues introduced by the mark command implementation. All SQL in add_mark (src/db.rs:366-419) uses params![] binding. No string interpolation into SQL.

Positive findings confirmed: confidence clamping in SQL is atomic, ON DELETE CASCADE verified by test, MarkKind enum follows conventions, all 8 acceptance criteria covered by tests.

### 2026-04-08T15:37:56-04:00
VERDICT:review:APPROVE
