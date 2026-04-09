## Approach
1. Update tests/integration.rs: all ID assertions expect String/UUID format instead of i64. 2. Update any test helpers that create units and capture IDs. 3. Add a test that verifies UUID format (regex: [0-9a-f]{8}-...). 4. Ensure all existing test scenarios still pass with UUID IDs.

## Verify
cargo test -- --test-threads=1 passes with all tests green. No i64 ID assumptions remain in test code.

## Result
Integration tests validate UUID-based ID system end-to-end.
