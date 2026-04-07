## Approach
1. Add FTS5 virtual table in initialize() after existing execute_batch, with existence check guard (match suda pattern). DDL: CREATE VIRTUAL TABLE units_fts USING fts5(content, type, tags, source, content=units, content_rowid=id). Add 3 triggers: units_ai (after insert), units_ad (after delete), units_au (after update). 2. Extract row_to_unit() helper from get_unit to reuse in list/search. 3. Add list_units(conn, type_filter: Option<&str>) -> Result<Vec<Unit>> — SELECT * FROM units [WHERE type=?] ORDER BY created DESC. 4. Add search_units(conn, query: &str) -> Result<Vec<Unit>> — JOIN units_fts ON rowid WHERE MATCH ? ORDER BY rank. 5. Add List and Search subcommands. List: optional --type filter. Search: positional query arg. 6. Add display::print_units(items, json) — compact list format [id] type (source) content-truncated.

## Verify
Seed 5 units of mixed types. simaris list shows all 5. simaris list --type fact shows only facts. simaris search 'caching' returns matching units. simaris search 'nonexistent' returns empty (exit 0). simaris list --json returns JSON array. cargo fmt && cargo clippy -- -D warnings && cargo test.

## Result
Units are findable by type filter and full-text search. Unblocks assembly (ask) and immune system (scan).

## AcceptanceCriteria
1. simaris list shows all units ordered by created DESC. 2. simaris list --type fact shows only facts. 3. simaris list --type bogus returns empty (exit 0). 4. simaris list --json returns JSON array. 5. simaris search 'query' returns matching units by relevance. 6. simaris search 'nonexistent' returns empty (exit 0). 7. simaris search --json returns JSON array. 8. FTS5 stays in sync after add and promote. 9. All existing 27 tests pass. 10. cargo fmt && cargo clippy -- -D warnings && cargo test.

## ScopeOut
No pagination, no FTS5 for inbox, no prefix/column queries

## AffectedAreas
src/db.rs (FTS5 setup, row helper, list/search functions, tests), src/main.rs (List/Search subcommands), src/display.rs (print_units), tests/integration.rs

## TestStrategy
Unit tests: test_list_all_units, test_list_filter_by_type, test_list_empty, test_search_finds_match, test_search_no_match, test_fts_sync_after_add. Integration tests: test_list_command, test_list_filter, test_search_command, test_search_empty_result, test_list_json_output. Verify: cargo fmt --check && cargo clippy -- -D warnings && cargo test.

## Risks
Low: FTS5 content-table mode requires trigger sync — follow suda's proven pattern. Low: search query with special FTS5 syntax chars could error — acceptable for v1.

## Report
TL built list+search with FTS5. db.rs: FTS5 virtual table + 3 sync triggers, row_to_unit helper, list_units + search_units, 6 new unit tests. main.rs: List + Search subcommands. display.rs: print_units with UTF-8 safe truncation. integration.rs: 5 new tests. 38/38 pass.

## Notes
### 2026-04-07T15:18:16-04:00
Investigation: FTS5 included in rusqlite bundled — no Cargo.toml change. Follow suda's FTS5 pattern: content-table mode, 3 sync triggers (ai/ad/au), existence check guard. Index columns: content, type, tags, source. Search via JOIN units_fts ON rowid ORDER BY rank (BM25). List: simple SELECT with optional WHERE type filter. Extract row mapper to avoid duplication. No FTS5 for inbox (ephemeral).
