## Approach
Add get_inbox_item(conn, id) -> Result<InboxItem>. Add promote_item(conn, inbox_id, unit_type) -> Result<i64> using conn.transaction(): SELECT inbox row, INSERT unit (content + source from inbox item), DELETE inbox row, commit, return unit id. Add Promote { id, type } to Command enum reusing UnitType ValueEnum. Call db::promote_item then display::print_added (reuse existing). No new display function needed.

## Verify
Run: simaris drop 'caching is important for perf' && simaris promote 1 --type fact && simaris show 1 — unit should exist with content from inbox, type=fact, source=inbox. Run: simaris inbox — should be empty. Run: simaris promote 999 --type fact — should fail (not found). cargo fmt && cargo clippy -- -D warnings && cargo test.

## Result
Complete capture pipeline: drop → inbox → promote → unit. Raw knowledge flows from inbox into the typed store.

## Outcome
Promote command working. Full capture pipeline: drop → inbox → promote → unit. Transaction-safe (atomic read+insert+delete). Source preserved from inbox item. 27 total tests passing. Committed as 4793063.

## AcceptanceCriteria
1. promote on valid inbox id returns new unit id with correct content/type/source. 2. Inbox item deleted after promote — inbox list empty. 3. Promote nonexistent id returns error. 4. Promote with invalid type returns CHECK constraint error. 5. Full pipeline: drop → promote → show unit → inbox empty. 6. All existing tests pass (no regression). 7. cargo fmt && cargo clippy -- -D warnings && cargo test.

## ScopeOut
No batch promote, no --source override on promote

## AffectedAreas
src/db.rs (get_inbox_item + promote_item + tests), src/main.rs (Promote subcommand), tests/integration.rs (new tests)

## TestStrategy
Unit tests in db.rs: test_promote_item (valid promote, verify unit fields), test_promote_deletes_inbox_item, test_promote_nonexistent_fails, test_promote_preserves_source. Integration tests: test_promote_command (full drop→promote→show→inbox-empty pipeline), test_promote_nonexistent_id. Verify: cargo fmt --check && cargo clippy -- -D warnings && cargo test.

## Risks
Low: transaction rollback on failure — conn.transaction() auto-rollbacks on drop. Low: ID sequences are independent (inbox vs units) — promote creates new unit ID, not reusing inbox ID.

## Report
TL built promote. db.rs: get_inbox_item + promote_item (with transaction via unchecked_transaction), 4 new unit tests. main.rs: Promote subcommand. Reused print_added. 2 new integration tests. 27/27 tests pass. Clean fmt/clippy.

## Notes
### 2026-04-07T14:47:07-04:00
Investigation: Use conn.transaction() for atomic promote (INSERT unit + DELETE inbox). Source field: copy from inbox item's source, not hardcode 'inbox'. Reuse print_added display function. get_inbox_item needed. Clean error on nonexistent ID (QueryReturnedNoRows). No --source flag needed — data is already captured.
