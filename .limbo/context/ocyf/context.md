## Approach
1. Change all i64 ID types to String throughout: Unit struct, InboxItem struct, Link struct, Mark struct, function signatures (add_unit, get_unit, delete_unit, add_inbox_item, promote, digest, link, unlink, mark, search, ask, scan, etc.). 2. Update add_unit/add_inbox_item to generate Uuid::now_v7().to_string() and bind as parameter instead of relying on last_insert_rowid(). 3. Update all SQL queries that reference id columns. 4. Update CLI argument parsing — id args become String. 5. Update display.rs for UUID formatting (show short 8-char prefix in table view, full UUID in --json).

## Verify
cargo build succeeds. cargo test passes. simaris add 'test fact' --type fact returns a UUID. simaris show <uuid> retrieves it. simaris list shows short UUIDs.

## Result
All code paths use String UUIDs. CLI accepts and displays UUIDs. No i64 ID references remain.
