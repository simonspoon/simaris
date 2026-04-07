## Approach
Rust CLI scaffold following ecosystem conventions (clap 4.x derive, rusqlite bundled, serde, dirs). Project at /Users/simonspoon/claudehub/simaris. Structure: main.rs (CLI entry + clap), db.rs (connection + schema + CRUD), display.rs (human + JSON output). Schema: units table (id INTEGER PK AUTOINCREMENT, content TEXT NOT NULL, type TEXT CHECK enum, source TEXT DEFAULT inbox, confidence REAL DEFAULT 1.0, verified INTEGER DEFAULT 0, tags TEXT DEFAULT '[]', conditions TEXT DEFAULT '{}', created/updated TEXT datetime). Links table (from_id, to_id, relationship with composite PK, CASCADE, idx on to_id). Three commands: add, show, link. Data dir: ~/.simaris/ with SIMARIS_HOME override. WAL mode, foreign_keys=ON. Error handling with anyhow (matching helios pattern). JSON validation on tags/conditions before INSERT.

## Verify
Run: simaris add 'test knowledge' --type fact --source inbox && simaris show 1 — must display the unit with all born-with fields populated. Run: simaris add 'related thing' --type procedure --source inbox && simaris link 1 2 --rel related_to && simaris show 1 — must display the unit with its link.

## Result
Working atom: a single knowledge unit can be created, stored, linked, and read back from SQLite.

## AcceptanceCriteria
1. simaris add 'test knowledge' --type fact --source inbox exits 0, prints unit ID. 2. simaris show 1 displays: content, type=fact, source=inbox, non-empty created, confidence=1.0, verified=0. 3. simaris add 'related thing' --type procedure --source inbox creates unit id 2. 4. simaris link 1 2 --rel related_to exits 0. 5. simaris show 1 after linking displays outgoing link to unit 2 with rel=related_to. 6. Unit test: invalid type 'bogus' returns CHECK constraint error. 7. Unit test: duplicate link returns PK violation. 8. Unit test: deleting unit cascades to remove its links. 9. Integration test: SIMARIS_HOME override creates db at that path.

## ScopeOut
No FTS5, no edit/update command, no search command, no schema_version table, no confidence range enforcement

## AffectedAreas
New project — all files are new. db.rs (schema + CRUD), main.rs (CLI), display.rs (output formatting), tests/integration.rs

## TestStrategy
Unit tests in db.rs: test_schema_creation, test_add_unit, test_constraint_violations (invalid type CHECK), test_duplicate_link (PK violation), test_cascade_delete. Integration tests in tests/integration.rs using TestEnv pattern: test_add_command, test_show_command, test_link_command, test_show_with_links, test_env_override. Verification: cargo fmt && cargo clippy -- -D warnings && cargo test. Smoke test: cargo run -- add 'hello' --type fact --source inbox && cargo run -- show 1.

## Risks
Medium: JSON fields (tags/conditions) have no DB-level validation — mitigate with Rust-side validation before INSERT. Low: CASCADE deletes could remove many links — acceptable for v1, no delete command planned. Low: rigid type enum requires migration to add types — acceptable, known tradeoff. Note: rusqlite version should match ecosystem (check current). Note: updated column not auto-maintained — no edit command in v1, so no issue yet.

## Report
TL built simaris CLI. 5 files: Cargo.toml, main.rs (clap derive with add/show/link + --json global), db.rs (connect + schema + CRUD + 5 unit tests), display.rs (human + JSON output), tests/integration.rs (6 tests with TestEnv). 11/11 tests pass. cargo fmt clean. cargo clippy zero warnings. Smoke test verified: add → show → add → link → show-with-links all exit 0.

## Notes
### 2026-04-07T13:39:03-04:00
Investigation: Researcher validated schema against suda/nyx/helios/ivara patterns. Key findings: (1) Follow scaffold pattern exactly, no deviations. (2) tags as JSON TEXT with json_each() for future queries. (3) conditions as JSON TEXT — opaque for now. (4) links table needs composite PK (from_id, to_id, relationship), idx on to_id, ON DELETE CASCADE. (5) Defer FTS5 to v2. (6) No schema_version table needed yet. (7) Use CHECK constraints on type and relationship enums. (8) verified as INTEGER (0/1), confidence as REAL DEFAULT 1.0. (9) Timestamps as TEXT with datetime('now') default.
