## Approach
1. Add uuid crate (v7 feature) to Cargo.toml. 2. Rewrite schema: units.id becomes TEXT PRIMARY KEY (UUIDv7), inbox.id same, links.from_id/to_id become TEXT, marks.unit_id becomes TEXT. 3. Write migration function: create new tables, INSERT INTO new SELECT with generated UUIDs, rebuild FTS triggers, drop old tables, rename. 4. Update all Rust code: i64 → String for IDs, update add_unit/add_inbox_item to generate UUIDv7, update all queries. 5. Update CLI display to show UUIDs (short form in tables, full in --json). 6. Update integration tests.

## Verify
cargo build succeeds, cargo test passes, manual test: simaris add + simaris show + simaris search all work with UUID IDs

## Result
All simaris IDs are UUIDv7 strings. Existing data preserved via migration. Ready for future export/import work.

## Outcome
All 3 subtasks complete. simaris IDs migrated from auto-increment integers to UUIDv7. Schema, code, and tests all updated. 84/84 tests pass. In-place migration preserves existing data.

## Description
Replace INTEGER PRIMARY KEY AUTOINCREMENT with UUIDv7 TEXT PRIMARY KEY across the entire schema (units, inbox, links, marks, units_fts). In-place migration preserves all existing data by generating UUIDv7s for current rows. Enables future cross-instance export/import without ID collisions.
