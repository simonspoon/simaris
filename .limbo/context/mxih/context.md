## Approach
1. Update data_dir(): after SIMARIS_HOME check, check SIMARIS_ENV — if 'dev', append /dev/ to default ~/.simaris path. 2. Add backup_dir() -> PathBuf = data_dir().join('backups'). Add db_path() -> PathBuf = data_dir().join('sanctuary.db'). 3. Restructure main(): parse CLI first, handle Restore before connect() (restore needs no connection). All other commands call connect() as before. 4. simaris backup: uses VACUUM INTO ?1 with timestamp filename (sanctuary-YYYYMMDD-HHMMSS.db). After backup, prune: read dir, sort by name, keep last 10, delete rest. 5. simaris restore (no args): list backups from backup_dir sorted by name. simaris restore <filename>: fs::copy backup to sanctuary.db, delete any -wal/-shm sidecars. 6. Display functions for backup/restore output.

## Verify
DEV: SIMARIS_ENV=dev simaris add 'test' --type fact && ls ~/.simaris/dev/sanctuary.db (exists). Regular: simaris add 'test' --type fact && ls ~/.simaris/sanctuary.db (exists, separate from dev). BACKUP: seed data, simaris backup exits 0 and creates timestamped file in backups/. RESTORE: simaris restore lists backups. simaris restore <timestamp> replaces DB. After restore, data matches backup. PRUNE: create 12 backups, verify only 10 remain. cargo test passes.

## Result
Brain is protected: dev isolation prevents accidents during development, backup/restore protects against data loss in production use.

## Outcome
Backup/restore + dev env working. SIMARIS_ENV=dev for isolation. VACUUM INTO for safe backups. Auto-prune keeps 10. Restore with WAL/SHM cleanup. main() restructured for deferred connect. 43 total tests. Committed as a059e98.

## AcceptanceCriteria
1. SIMARIS_ENV=dev routes to ~/.simaris/dev/sanctuary.db. 2. SIMARIS_HOME still overrides everything. 3. simaris backup creates timestamped file in backups/. 4. simaris restore lists available backups when no arg given. 5. simaris restore <file> replaces DB with backup, deletes wal/shm sidecars. 6. After restore, data matches backup state. 7. Creating 12 backups leaves only 10 (oldest pruned). 8. All existing 38 tests pass. 9. cargo fmt && cargo clippy && cargo test.

## ScopeOut
No auto-backup on migration (no migrations yet), no concurrent lock checking

## AffectedAreas
src/db.rs (data_dir, backup_dir, db_path, backup/restore functions), src/main.rs (restructure for deferred connect, Backup/Restore subcommands), src/display.rs (backup/restore output), tests/integration.rs

## TestStrategy
Unit tests: test_data_dir_default, test_data_dir_env_dev, test_data_dir_simaris_home_overrides. Integration tests: test_backup_command (seed data, backup, verify file exists), test_restore_command (seed, backup, add more, restore, verify original data), test_backup_prune (create 12 backups, verify 10 remain), test_restore_list (backup twice, restore with no args lists both), test_env_dev_isolation (SIMARIS_ENV=dev creates separate db). Verify: cargo fmt --check && cargo clippy -- -D warnings && cargo test.

## Risks
Medium: restructuring main() to defer connect() — must not break existing command flow. Low: VACUUM INTO requires open connection — backup command gets one. Low: restore while another process has DB open — not guarded, acceptable for v1 single-user CLI.

## Report
TL built backup/restore + dev env. Restructured main() for deferred connect. VACUUM INTO for safe backups. Auto-prune keeps 10. WAL/SHM cleanup on restore. SIMARIS_ENV=dev always effective. 43/43 tests pass.

## Notes
### 2026-04-07T16:26:06-04:00
Investigation: VACUUM INTO for safe WAL-mode backup (atomic, no sidecars). Restore must skip connect() — restructure main() to handle restore before connecting. SIMARIS_ENV=dev adds /dev/ subdir to default path. backup_dir() derives from data_dir(). Prune by filename sort (timestamp is lexicographic). Delete -wal/-shm sidecars on restore.
