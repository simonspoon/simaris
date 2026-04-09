## Approach
Fix all 24 compile errors: 1) ask.rs: change LinkInfo.unit_id to String, AskResult.units_used to Vec<String>, FilterResponse.relevant_ids to Vec<String>, fix all call sites passing String where &str expected (add & borrows). Update LLM prompt format string from id={} to id={}(already string). 2) main.rs: change all CLI id: i64 args to id: String in clap Command enums. Update call sites to pass &id instead of id. 3) display.rs: update function signatures accepting i64 IDs to &str. Add short UUID display (first 8 chars) for table format.

## Verify
cargo build succeeds. cargo test passes. simaris add 'test fact' --type fact returns a UUID. simaris show <uuid> retrieves it. simaris list shows short UUIDs.

## Result
All code paths use String UUIDs. CLI accepts and displays UUIDs. No i64 ID references remain.

## AcceptanceCriteria
1. cargo build succeeds with zero errors. 2. cargo clippy passes (no warnings). 3. No i64 ID references remain in ask.rs, main.rs, display.rs (except legitimate integer uses like counts). 4. CLI args for ID parameters accept strings. 5. Display shows short 8-char UUID prefix in table view, full UUID in --json. 6. LLM filter in ask.rs sends/receives string IDs.

## ScopeOut
db.rs (done in fzza), tests/integration.rs (task tbwj)

## AffectedAreas
src/ask.rs, src/main.rs, src/display.rs

## TestStrategy
1. cargo build — zero errors. 2. cargo clippy --workspace --all-targets -- -D warnings — zero warnings. 3. cargo test — unit tests pass. 4. Manual: simaris add 'test' --type fact — returns UUID. simaris show <uuid> — works. simaris list — shows short UUIDs.

## Risks
1. LOW: LLM filter prompt format change could affect model parsing of IDs — mitigated by keeping same id=X format, just with UUID strings. 2. LOW: display truncation of UUIDs to 8 chars could cause ambiguity — acceptable for table view, full UUID in --json.

## Report
Fixed all 24 compile errors. ask.rs: all ID struct fields to String, FilterResponse.relevant_ids to Vec<String>, HashMap key types, borrow fixes. main.rs: all clap id args to String, call site borrows. display.rs: parameter types to &str, added short_id() helper (8-char prefix for table view, full UUID in JSON). One clippy annotation added to db.rs. cargo build/clippy/fmt all clean. 42 unit tests pass. 18 integration test failures remain (sibling task tbwj).
