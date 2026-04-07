## Approach
Add inbox table in db.rs initialize(): (id INTEGER PK AUTOINCREMENT, content TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'cli', created TEXT NOT NULL DEFAULT datetime('now')). Add Rust functions: drop_item(conn, content, source) with explicit empty-content validation (anyhow::bail), list_inbox(conn) returning Vec<InboxItem> ordered by created ASC. Add InboxItem struct (id, content, source, created). Add Drop and Inbox subcommands to main.rs clap. Add display functions in display.rs (human + JSON). Update test_schema_creation to expect 3 tables. Source is required param in Rust function, optional on CLI with default 'cli'.

## Verify
Run: simaris drop 'raw idea about caching' && simaris drop 'another thought' && simaris inbox — must show 2 pending items with timestamps. Run: simaris inbox --json — must return JSON array. Run: simaris drop 'third thing' --source phone && simaris inbox — source field should show 'phone'.

## Result
Working inbox: raw knowledge can be captured with zero friction and listed for later processing.

## AcceptanceCriteria
1. simaris drop 'some raw text' exits 0, prints Dropped item N. 2. simaris inbox lists the item with ID, timestamp, content. 3. simaris inbox on empty inbox prints 'Inbox is empty.' exit 0. 4. simaris drop '' (empty) exits non-zero with error. 5. Drop 3 items then simaris inbox shows all 3 in chronological order. 6. simaris inbox --json returns JSON array with id, content, source, created per item. 7. simaris drop 'thing' --source phone shows source=phone in inbox listing. 8. cargo test passes with new inbox tests + no regression on existing 11 tests. 9. simaris show and simaris link still work (no regression).

## ScopeOut
No promote command, no --limit/pagination on inbox, no file drops (text only for now)

## AffectedAreas
src/db.rs (new table + functions + struct + tests), src/main.rs (new subcommands), src/display.rs (new display functions), tests/integration.rs (new tests)

## TestStrategy
Unit tests in db.rs: test_inbox_schema_created, test_add_inbox_item_defaults, test_add_inbox_item_custom_source, test_get_inbox_items_ordering, test_add_inbox_item_empty_content_rejected. Integration tests: test_drop_command, test_drop_command_custom_source, test_inbox_empty, test_drop_empty_content_rejected, test_inbox_json_output. Verify: cargo fmt --check && cargo clippy -- -D warnings && cargo test (all old + new tests pass).

## Risks
Low: 'drop' name may confuse (SQL DROP) but user chose it deliberately. Low: source field DB default could mask missing args — mitigated by making it required in Rust function. Low: existing test_schema_creation needs count update 2→3. Note: empty string validation must be explicit in Rust, SQLite TEXT NOT NULL allows empty strings.

## Report
TL added inbox capture. Modified 4 files: db.rs (inbox table, InboxItem struct, drop_item with empty validation, list_inbox, 5 new unit tests, updated schema count), main.rs (Drop + Inbox subcommands), display.rs (print_dropped + print_inbox with truncation), integration.rs (5 new tests). 21/21 tests pass. cargo fmt clean. clippy zero warnings.

## Notes
### 2026-04-07T14:34:08-04:00
Investigation: Separate inbox table confirmed — units CHECK constraint prevents type='raw', and inbox items are pre-knowledge (may become 0, 1, or many units). Schema: id, content, source (default 'cli'), created. Minimal — no type/confidence/tags. Migration safe via CREATE TABLE IF NOT EXISTS. Risks: ID confusion between inbox and units (different sequences); no promote command yet (scope out). Researcher recommends dropping source field entirely but user explicitly wanted source as auto-captured envelope metadata, so keeping it with default 'cli'.
