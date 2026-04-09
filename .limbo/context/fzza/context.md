## Approach
1. Add uuid = { version = 1, features = [v7] } to Cargo.toml. 2. In db.rs, write migrate_to_uuid(conn): a) call create_backup() first, b) begin unchecked_transaction, c) drop FTS5 table and triggers, d) rename units→units_old, inbox→inbox_old, links→links_old, marks→marks_old, e) create new tables with TEXT PRIMARY KEY, f) INSERT INTO units SELECT with Uuid::now_v7() generated in Rust (iterate rows), g) INSERT INTO links/marks mapping old integer FKs to new UUIDs via units_old.id→units.id join, h) create standalone FTS5 (no content_rowid) with uuid column, i) create new triggers referencing uuid, j) drop _old tables, k) set PRAGMA user_version=1, l) commit, m) verify record counts match. 3. Update connect(): check PRAGMA user_version — if 0, run migrate_to_uuid(); then run initialize(). 4. Update initialize(): new DDL uses TEXT PRIMARY KEY, standalone FTS5, set user_version=1 for fresh installs. 5. Update all db.rs structs (Unit, InboxItem, Link, Mark) to String IDs. 6. Update all db.rs function signatures and SQL bindings from i64 to String. 7. Generate Uuid::now_v7().to_string() in add_unit/add_inbox_item instead of last_insert_rowid().

## Verify
cargo build succeeds. Fresh simaris init creates TEXT PK tables. Existing sanctuary.db with integer IDs gets migrated — all rows preserved with UUID PKs. Verify with: sqlite3 ~/.simaris/sanctuary.db '.schema units'

## Result
Schema uses UUIDv7 TEXT PRIMARY KEY everywhere. Migration function handles existing data. Fresh installs use new schema directly.

## Outcome
Schema migrated to UUIDv7 TEXT PRIMARY KEY. uuid crate added. In-place migration preserves all existing data with backup, FK remapping, and record count verification. Standalone FTS5 with uuid column. PRAGMA user_version for schema versioning. All db.rs types and functions updated. 24 expected compile errors remain in sibling-task files (ask.rs, main.rs, display.rs).

## AcceptanceCriteria
1. Cargo.toml has uuid = { version = 1, features = [v7] } dependency. 2. units/inbox use TEXT PRIMARY KEY, links.from_id/to_id and marks.unit_id are TEXT NOT NULL with ON DELETE CASCADE. 3. Existing integer-ID rows get valid UUIDv7 strings assigned; FK relationships preserved. 4. units_fts rebuilt as standalone FTS5 (no content_rowid coupling) with uuid column; triggers updated. search_units joins on uuid. 5. PRAGMA user_version = 1 set after migration; connect() guards against re-running. 6. All db.rs struct types (Unit, InboxItem, Link, Mark) use String for IDs. 7. All db.rs function signatures use String IDs. 8. cargo build succeeds.

## ScopeOut
display.rs formatting, CLI arg types in main.rs, ask.rs structs/LLM prompt, integration tests — those are sibling tasks ocyf and tbwj.

## AffectedAreas
src/db.rs (schema DDL, migration function, all struct types, all function signatures), Cargo.toml (uuid dependency)

## TestStrategy
1. cargo build — confirms uuid crate compiles and all type changes are consistent. 2. cargo test — runs db.rs unit tests (will need updating in same subtask if they exist). 3. sqlite3 verification: sqlite3 ~/.simaris/sanctuary.db '.schema units' — confirm TEXT PRIMARY KEY, no INTEGER AUTOINCREMENT. 4. sqlite3 ~/.simaris/sanctuary.db 'SELECT count(*) FROM units' — confirm record count matches pre-migration. 5. sqlite3 ~/.simaris/sanctuary.db 'SELECT id FROM units LIMIT 3' — confirm UUID format. 6. sqlite3 ~/.simaris/sanctuary.db 'PRAGMA user_version' — confirm returns 1. 7. Re-run simaris binary to confirm connect() doesn't re-migrate (idempotent).

## Risks
1. CRITICAL: FTS5 content_rowid requires integer — mitigated by switching to standalone FTS5 with uuid column. 2. HIGH: Multi-DDL migration atomicity — mitigated by unchecked_transaction + pre-migration backup. 3. HIGH: FK integrity during data copy — mitigated by joining on units_old.id for UUID mapping. 4. MEDIUM: Partial migration failure — mitigated by backup + post-migration record count verification. 5. LOW: compile errors from i64→String type changes propagating to out-of-scope files (ask.rs, main.rs, display.rs) — expected and acceptable, sibling tasks fix those.

## Report
TL modified Cargo.toml (added uuid v1 with v7 feature) and src/db.rs. All struct ID fields changed i64→String. Migration function: backup, rename old tables, create new TEXT PK tables, iterate rows generating UUIDv7, map FK references, rebuild standalone FTS5 with uuid column, drop old tables, set user_version=1. connect() checks user_version before migrating. initialize() uses new schema for fresh installs. All last_insert_rowid() removed. cargo check shows 24 errors — ALL in ask.rs, main.rs, display.rs, integration.rs (out of scope). Zero errors in db.rs.

## Notes
### 2026-04-08T22:55:26-04:00
INVESTIGATION: FTS5 content_rowid=id requires integer rowid. After migration to TEXT PK, units table still has an implicit SQLite rowid (since TEXT PK is not INTEGER PRIMARY KEY). Strategy: drop content_rowid from FTS5 definition, store uuid in an FTS5 column, and join on uuid instead of rowid. Alternatively, keep an explicit integer rowid column for FTS sync. Decision: use standalone FTS5 (no content= backing) with uuid stored as a column — simplest approach, avoids rowid coupling entirely. Rebuild triggers to INSERT uuid into FTS alongside content/type/tags/source. Search joins on uuid column in FTS.

### 2026-04-08T22:55:27-04:00
INVESTIGATION: No existing schema versioning. Will introduce PRAGMA user_version = 1 after migration. connect() checks user_version to decide whether to run migration. Fresh installs get new schema + user_version=1 directly.

### 2026-04-08T22:57:29-04:00
TEST STRATEGY RATIONALE: Three-phase approach validates UUID migration at every level. Phase 1 establishes baseline behavior before migration with existing integer ID system, focusing on schema structure and FTS coupling. Phase 2 tests the actual migration process with concrete SQLite commands to verify schema changes, data preservation, and version guards. Phase 3 ensures all functionality works post-migration with UUID IDs via both unit tests and CLI integration tests. Every test item specifies exact tools (cargo test, sqlite3, simaris CLI) and expected outcomes (schema patterns, row counts, UUID formats). Migration is one-time so strategy emphasizes verification of successful transformation rather than repeatability. Special attention to FTS5 transition from content_rowid coupling to standalone with uuid column, since this is the key technical risk identified in task notes.

### 2026-04-08T23:21:58-04:00
REVIEW FINDINGS:

### 0001-01-01T00:00:00Z
- [tests/integration.rs:46] Test hardcoded to expect 'Added unit 1' but UUIDv7 migration means IDs are now long strings. All numeric ID assertions in tests will fail post-migration.
- [tests/integration.rs:169] Same issue - test expects 'Added unit 1' with numeric ID.
- [tests/integration.rs:101,159,268,495] JSON tests expecting parsed['id'] == 1 (integer) will fail with UUID strings.
- [tests/integration.rs:60,71,72,80,81,82,86,168,182,484,485,493,503,571,576,577,584,587,594,600,601,612,634,640,647,655,660] All hardcoded references to numeric IDs ('1', '2', '999') in show/link/mark/scan commands will fail.

The migration from i64→String IDs requires all integration tests to be updated to handle UUID format. This is a blocking issue - tests cannot pass with current hardcoded expectations.

### 0001-01-01T00:00:00Z
- [src/display.rs:46-52] print_added() displays full UUID string instead of using short_id() helper for human readability. Should be: println!("Added unit {}", short_id(id)).
- [src/display.rs:54-67] print_linked() displays full UUID strings for both from_id and to_id instead of short IDs. Should use short_id() for console output.
- [src/display.rs:69-75] print_dropped() displays full UUID string instead of short ID.
- [src/display.rs:166-180] print_marked() displays full UUID string instead of short ID.

All user-facing display functions should use short_id() for console readability while keeping full UUIDs for JSON output.

### 0001-01-01T00:00:00Z
- [src/ask.rs:135-157] Borrow handling is correct - using HashSet<&String> to avoid cloning, then .clone() only when building final result. Efficient approach.
- [src/ask.rs:427] FilterResponse.relevant_ids: Vec<String> correctly matches UUID string format for LLM response parsing.
- [src/display.rs:5-7] short_id() function correctly handles edge case of IDs shorter than 8 chars by returning the full string.
- [src/main.rs:352] ask() function call correctly passes type filter by reference.

The core String ID type changes are correctly implemented throughout. LLM integration properly handles string IDs. Borrow patterns avoid unnecessary allocations.

### 2026-04-08T23:22:01-04:00
VERDICT:review:REQUEST_CHANGES
