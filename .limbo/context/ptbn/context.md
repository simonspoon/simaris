## Approach
Add 6 integration tests to tests/integration.rs using TestEnv: empty store, low-confidence, contradictions, orphans, --json output, --stale-days flag

## Verify
cargo test --test integration test_scan — all new tests pass, no existing tests broken

## Result
Full CLI-level test coverage for scan command

## Notes
### 2026-04-08T15:54:14-04:00
REVIEW FINDINGS:

### 0001-01-01T00:00:00Z
- [Cargo.toml:15] rusqlite is already a [dependencies] entry (line 10) with features=["bundled"]. The [dev-dependencies] duplicate (line 15) is harmless at build time — Cargo deduplicates them — but it is redundant and misleads readers into thinking the crate is only for tests. The dev-dependency entry can be deleted without losing any test capability.

### 0001-01-01T00:00:00Z
- [tests/integration.rs:563-565] test_scan_contradictions asserts both [1] and [2] appear in the full output string. If a future scan section above Contradictions ever prints those IDs, the assertion would pass vacuously. The orphans test already uses the better pattern (split on section header and check only the relevant slice). Applying the same to contradictions would make it more defensive.

- [tests/integration.rs:629-634] test_scan_stale_days SQL backdating is safe: no user input, parameterized correctly, connection dropped before process spawn, isolated env directory by name+PID. Pattern is identical to what the unit tests in db.rs already do.

- [tests/integration.rs:545-554] Confidence math comment (1.0->0.8->0.6->0.4 after 3 wrongs) is accurate. wrong delta=-0.2, threshold is strict < 0.6, so 0.4 triggers it correctly.

- [tests/integration.rs:595-621] test_scan_json correctly verifies all five JSON array keys and that unit 1 appears in both low_confidence and negative_marks (3x wrong degrades confidence AND inserts wrong-kind marks). No fragility.

- All six new tests follow the existing TestEnv pattern exactly: unique name+PID temp directory, SIMARIS_HOME override, Drop cleanup. No shared state, no hard-coded paths, no test pollution.

### 2026-04-08T15:54:16-04:00
VERDICT:review:APPROVE
