## Approach
1. Update test helpers to capture UUID from command output (parse 'Added unit <uuid>' messages). 2. Replace all hardcoded '1', '2' ID references with captured UUIDs. 3. Update JSON assertions from is_number() to is_string() for ID fields. 4. Add UUID format validation test. 5. Fix raw SQL in test_scan_stale_days to use UUID.

## Verify
cargo test -- --test-threads=1 passes with all tests green. No i64 ID assumptions remain in test code.

## Result
Integration tests validate UUID-based ID system end-to-end.

## Outcome
All 42 integration tests updated and passing. UUID extraction helpers, format validation, short_id display matching. No hardcoded integer IDs remain.

## AcceptanceCriteria
1. cargo test passes with ALL integration tests green. 2. No hardcoded integer IDs remain in test code. 3. Tests capture UUID from add/drop output and use it in subsequent commands. 4. UUID format validated (regex pattern). 5. JSON assertions check for string IDs, not numeric.

## AffectedAreas
tests/integration.rs

## TestStrategy
cargo test -- --test-threads=1 — all tests pass.

## Risks
LOW: test ordering dependency if UUIDs captured from prior commands.

## Report
Fixed all 18 failing integration tests. Added extract_id/short_id/assert_uuid_format helpers. All hardcoded integer IDs replaced with captured UUIDs. JSON assertions updated to string checks. Raw SQL fixed with parameterized UUID. Nonexistent ID tests use valid-format zero UUID. 84/84 tests pass. Clippy/fmt clean.

## Notes
### 2026-04-08T23:37:59-04:00
REVIEW FINDINGS:

### 2026-04-08T23:38:02-04:00
VERDICT:review:APPROVE
