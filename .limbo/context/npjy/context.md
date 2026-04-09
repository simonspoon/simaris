## Approach
Write a Python migration script (scripts/populate-production.py) that: (1) Backs up production via simaris backup. (2) Opens production sanctuary.db directly via sqlite3. (3) DELETEs the test unit. (4) ATTACHes dev2/sanctuary.db. (5) Within a single BEGIN IMMEDIATE transaction: INSERTs all 196 units (no explicit id — autoincrement), builds old→new ID map via content matching, INSERTs 167 links with remapped IDs. COMMIT. (6) Runs FTS rebuild: INSERT INTO units_fts(units_fts) VALUES("rebuild"). (7) Loops suda recall --json, calls simaris drop via subprocess (list args, not shell string) for each with --source suda:<name>. (8) Calls simaris digest to classify inbox items. Script is disposable — one-time use, not committed to repo.

## Verify
Production store has skill knowledge + suda memories. simaris search finds content from both sources.

## Result
Simaris is populated and ready for real use.

## Outcome
Production store populated: 270 units (196 from dev2 skill knowledge + 74 from 20 suda memories via digest), 221 links, 0 inbox items. FTS search works, scan clean, backup preserved. Simaris is ready for real use as canonical knowledge source.

## AcceptanceCriteria
1. simaris list returns >= 196 units (all dev2 units migrated). 2. simaris list --json shows sources starting with skill: (from dev2) and suda: (from suda memories). 3. simaris search "xaku" returns results (FTS index works). 4. simaris scan shows no orphaned links. 5. All 20 suda memories dropped and digested (inbox empty after digest). 6. Production DB is ~/.simaris/sanctuary.db. 7. Backup exists before migration.

## ScopeOut
Migrating marks (dev2 has 0). Migrating backups. Simaris-observer hook changes (already done). Source code changes to simaris. Re-importing overlapping content (confirmed no overlap). Auto-digest automation.

## AffectedAreas
~/Library/Application Support/simaris/sanctuary.db (production DB). No source code changes.

## TestStrategy
Pre-migration: verify dev2 has 196 units and 167 links via sqlite3. Post-migration: (1) cargo run -- list --json | jq length >= 196. (2) cargo run -- list --json | jq sources include skill: and suda:. (3) cargo run -- search xaku returns results. (4) cargo run -- scan shows no orphans. (5) Inbox is empty after digest. (6) simaris backup exists.

## Risks
CRITICAL: DB inconsistency if script fails mid-way — mitigated by single transaction + backup. HIGH: Shell injection from suda content — mitigated by subprocess list args. MEDIUM: FTS index staleness after bulk INSERT — mitigated by explicit rebuild. LOW: Concurrent access during migration — acceptable for one-time script on dev machine.

## Report
Script created: scripts/populate-production.py. Migrated 196 dev2 units + 167 links via SQL ATTACH. Imported 20 suda memories via drop+digest → 74 new units + 54 links. Total: 270 units, 221 links, 0 inbox items. All verification passed: search xaku returns 6 results, scan shows no issues, inbox empty. Deviation: production DB at ~/.simaris/ not ~/Library/Application Support/simaris/.

## Notes
### 2026-04-08T19:29:20-04:00
Investigation: dev2 has 196 units + 167 links (procedure:80, fact:58, principle:56, preference:1, lesson:1) from 29 skill sources. Schemas identical. No simaris import/export command exists. SQLite ATTACH+INSERT is best path for dev2 (preserves timestamps, links need ID remapping). Suda has 20 memories — drop+digest pipeline is correct (LLM reclassifies into atomic units). No dedup in simaris — idempotency must be enforced by script. Production has 1 test unit to clean up. SIMARIS_HOME env var can point at dev2.

### 2026-04-08T19:32:58-04:00
TEST STRATEGY RATIONALE: Three-phase approach ensures data integrity at each step. Phase 1 validates source data and current functionality before migration. Phase 2 tests the actual migration results against all 7 acceptance criteria with specific commands and expected outputs. Phase 3 performs comprehensive health checks using simaris built-in scan command plus manual verification. Strategy uses cargo test for unit tests, direct simaris CLI commands for integration verification, and jq for JSON parsing/validation. Each test item specifies exact commands and expected outcomes (unit counts, grep patterns, file existence). No prose directives - every verification step has a concrete tool/command. Migration is one-time so strategy focuses on post-migration verification rather than repeatability.

### 2026-04-08T19:42:54-04:00
REVIEW FINDINGS:

### 0001-01-01T00:00:00Z
- [scripts/populate-production.py:11] Path mismatch: Script assumes production DB at ~/.simaris/sanctuary.db but task report indicates production DB is actually at ~/.simaris/ (not ~/Library/Application Support/simaris/ as stated in acceptance criteria #6). Script will fail if actual production location differs from hardcoded path.

### 0001-01-01T00:00:00Z
- [scripts/populate-production.py:55-56] Dev2 unit count check only warns but continues processing. If dev2 doesn't have exactly 196 units, migration continues with potentially incomplete data. Consider making this a hard error.
- [scripts/populate-production.py:89-92] Incomplete cleanup on ID mapping failure. ROLLBACK is called but ATTACH DATABASE dev2 is not detached, potentially leaving connection in inconsistent state.
- [scripts/populate-production.py:79-88] ID mapping assumes content+source uniqueness. If dev2 has duplicate content+source combinations, mapping could fail silently or create incorrect associations.

### 0001-01-01T00:00:00Z
- [scripts/populate-production.py:95-110] Link insertion uses individual INSERT statements instead of bulk operations. For 167 links this is acceptable but could be optimized with INSERT...SELECT.
- [scripts/populate-production.py:134-139] suda recall subprocess error handling could provide more detailed failure context beyond stderr output.
- [scripts/populate-production.py:1-181] Overall excellent code quality: proper use of parameterized SQL queries prevents injection, subprocess list arguments prevent shell injection, good error handling patterns, clear structure.

### 2026-04-08T19:42:57-04:00
VERDICT:review:REQUEST_CHANGES
