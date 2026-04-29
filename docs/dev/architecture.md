# Simaris Architecture

Knowledge unit storage and retrieval system backed by SQLite with FTS5 full-text search and graph-based linking.

## Module Overview

### `main.rs`

CLI entry point. Defines the `Cli` struct (clap `Parser`) with global `--json` and `--debug` flags, and the `Command` enum covering all subcommands: `Add`, `Show`, `Edit`, `Link`, `Drop`, `Promote`, `Inbox`, `List`, `Search`, `Ask`, `Prime`, `Stats`, `Archive`, `Unarchive`, `Clone`, `Mark`, `Slug`, `Emit`, `Scan`, `Rewrite`, `Backup`, `Restore`, `Delete`. Also defines the `UnitType` enum (fact, procedure, principle, preference, lesson, idea, aspect), the `Relationship` enum (related_to, part_of, depends_on, contradicts, supersedes, sourced_from), and the `MarkKind` enum (used, wrong, outdated, helpful) with associated confidence deltas. The `main` function handles `Restore` without a database connection, then opens a connection for all other commands and dispatches to `db::*` and `display::*` functions.

### `db.rs`

Database layer. Owns connection setup, schema initialization, migration, all CRUD operations, FTS5 search, backup/restore, and the `scan` health-check. All IDs are UUIDv7 strings. Tables: `units`, `links`, `inbox`, `marks`, `slugs`, and the `units_fts` virtual table. The module is entirely synchronous using `rusqlite::Connection`.

### `ask.rs`

Two-phase retrieval pipeline: FTS5 search (up to 15 direct matches), then 1-hop graph expansion fetching all linked neighbours of each match. Returns an `AskResult` containing matched units, their IDs, and an optional debug trace. No LLM call — `ask` is a pure SQL + graph walk.

### `display.rs`

Output formatting. Every user-facing print operation goes through this module. Each function takes a `json: bool` flag and either prints structured JSON (`serde_json::to_string_pretty`) or human-readable text. Handles units, inbox items, links, backups, marks, and scan results. Uses `short_id()` (first 8 chars of UUID) for compact display in text mode.

### `rewrite.rs`

Editor-driven rewrite flow. Opens `$EDITOR` (override: `SIMARIS_EDITOR`; fallback: `vi`) with a type-aware skeleton pre-filled with the unit's existing content. Parses frontmatter (if any) on save and writes the new content back, preserving identity (id, tags, links, marks, slugs, created).

### `frontmatter.rs`

YAML frontmatter parse/write + `refs:` graph materialization. When a unit body opens with a `---` block, fields are parsed and `refs:` entries are turned into `related_to` edges from the unit.

### `size_guard.rs`

Write-time body-size thresholds. At `add` and `edit` time, body size is measured against `SIMARIS_WARN_BYTES` and `SIMARIS_HARD_BYTES`. Warn → stderr message citing the `split-ruleset` slug, write proceeds. Hard → non-zero exit unless `--force` or tag/flag `flow`.

## Data Flow

### Capture: Drop to Typed Units

```
User input
    |
    v
simaris drop "content" --source cli
    |
    v
db::drop_item()              -- Insert into `inbox` table, return UUIDv7
    |
    v
simaris promote <id> --type <type>
    |
    v
db::promote_item()           -- Transaction:
    |                              1. Insert new unit (UUIDv7) with content/type/source
    |                              2. Delete inbox item
    v
Typed unit in `units` table
(FTS5 index updated automatically via AFTER INSERT trigger)
```

`simaris add` skips the inbox entirely and inserts directly into `units`. `clone` copies a unit into a new UUIDv7. `add` and `clone` auto-link the new unit to existing units sharing 2+ tags via `related_to`.

### Retrieval: Ask the Knowledge Store

```
simaris ask "query" [--type fact]
    |
    v
Phase 1: FTS5 Search + Graph Expansion
    |
    +-- sanitize_fts_query(query)     -- Strip operators, remove stop words,
    |                                    quote each term, join with OR
    +-- search_and_expand()
            |
            +-- db::search_units()    -- FTS5 MATCH on units_fts, JOIN to units
            |                            Capped at 15 direct matches
            +-- db::get_linked_unit_ids()  -- For each match, fetch 1-hop neighbours
            |                                 (both outgoing and incoming links)
            +-- db::get_unit()        -- Fetch full unit data for each linked unit
            v
        GatherResult { units, matches_per_query, direct_count, expansion_count }
    |
    v
AskResult { query, units, units_used, debug }
```

Each unit in the result carries `is_direct_match: bool` (FTS5 hit vs 1-hop linked) and `links` to units outside the result set so the caller can walk further on demand.

## Database Lifecycle

### Connection setup

```rust
pub fn connect() -> Result<Connection>
```

1. Resolve data directory: `$SIMARIS_HOME` env var, or `~/.simaris`. If `$SIMARIS_ENV=dev`, appends `/dev`.
2. `create_dir_all` on the data directory.
3. Open `sanctuary.db` via `rusqlite::Connection::open`.
4. Set `PRAGMA journal_mode=WAL` -- write-ahead logging for concurrent read access.
5. Set `PRAGMA foreign_keys=ON` -- enforce referential integrity (links, marks).
6. Check `PRAGMA user_version`:
   - If 0 and `units` table exists: run `migrate_to_uuid()` (integer-to-UUIDv7 migration).
   - If 0 and no tables: fresh install path.
7. Call `initialize()` to ensure schema exists.

### Schema

Five tables and one virtual table (see `docs/dev/data-model.md` for full column-by-column detail):

```sql
units      (id TEXT PK, content, type, source, confidence, verified, archived,
            tags JSON, conditions JSON, created, updated)
links      (from_id, to_id, relationship)  -- composite PK, FK ON DELETE CASCADE
inbox      (id TEXT PK, content, source, created)
marks      (id TEXT PK, unit_id FK, kind, created)
slugs      (slug TEXT PK, unit_id FK, created)
units_fts  USING fts5(uuid, content, type, tags, source)  -- synced via triggers
```

Indexes: `idx_links_to ON links(to_id)`, `idx_marks_unit ON marks(unit_id)`, `idx_slugs_unit_id ON slugs(unit_id)`.

Three triggers keep `units_fts` in sync: `units_ai` (after insert), `units_ad` (after delete), `units_au` (after update).

### Migration system

Schema versioning uses `PRAGMA user_version`. `migrate_to_uuid()` handles the v0-to-v1 migration (integer auto-increment IDs to UUIDv7 TEXT IDs). The migration creates a backup, renames old tables, creates new TEXT-PK tables, maps old integer IDs to UUIDv7 strings, copies all data, verifies row counts, recreates FTS and triggers, drops old tables, sets `user_version = 1`. All inside a single transaction; rollback on failure.

### Backup and prune

```rust
pub fn create_backup(conn: &Connection) -> Result<PathBuf>
```

Uses `VACUUM INTO` to create a consistent snapshot at `~/.simaris/backups/sanctuary-YYYYMMDD-HHMMSS.db`. After each backup, `prune_backups()` keeps only the 10 most recent files.

```rust
pub fn restore_backup(filename: &str) -> Result<()>
```

Removes WAL/SHM files, then copies the backup over the active database file. Operates without an open connection (handled before `connect()` in `main.rs`).

## Key Public Functions

### `db.rs`

```rust
// Connection and schema
pub fn data_dir() -> PathBuf
pub fn db_path() -> PathBuf
pub fn backup_dir() -> PathBuf
pub fn connect() -> Result<Connection>

// Unit CRUD
pub fn add_unit(conn: &Connection, content: &str, unit_type: &str, source: &str) -> Result<String>
pub fn add_unit_full(conn: &Connection, content: &str, unit_type: &str, source: &str, tags: &[String]) -> Result<String>
pub fn update_unit(conn, id, content, unit_type, source, tags) -> Result<Unit>
pub fn get_unit(conn: &Connection, id: &str) -> Result<Unit>
pub fn list_units(conn: &Connection, type_filter: Option<&str>) -> Result<Vec<Unit>>
pub fn search_units(conn: &Connection, query: &str, type_filter: Option<&str>) -> Result<Vec<Unit>>
pub fn archive_unit(conn: &Connection, id: &str) -> Result<()>
pub fn unarchive_unit(conn: &Connection, id: &str) -> Result<()>
pub fn clone_unit(conn, id, type_override, source_override, tags_override) -> Result<String>

// Link operations
pub fn add_link(conn, from_id, to_id, relationship) -> Result<()>
pub fn get_links_from(conn: &Connection, id: &str) -> Result<Vec<Link>>
pub fn get_links_to(conn: &Connection, id: &str) -> Result<Vec<Link>>
pub fn get_linked_unit_ids(conn: &Connection, id: &str) -> Result<Vec<(String, String, String)>>

// Inbox operations
pub fn drop_item(conn: &Connection, content: &str, source: &str) -> Result<String>
pub fn get_inbox_item(conn: &Connection, id: &str) -> Result<InboxItem>
pub fn promote_item(conn: &Connection, inbox_id: &str, unit_type: &str) -> Result<String>
pub fn list_inbox(conn: &Connection) -> Result<Vec<InboxItem>>
pub fn delete_inbox_item(conn: &Connection, id: &str) -> Result<()>

// Marks and confidence
pub fn add_mark(conn: &Connection, unit_id: &str, kind: &str, delta: f64) -> Result<f64>

// Health check + stats
pub fn scan(conn: &Connection, stale_days: u32) -> Result<ScanResult>
pub fn stats(conn: &Connection, top: usize, include_archived: bool) -> Result<StatsResult>

// Backup/restore
pub fn create_backup(conn: &Connection) -> Result<PathBuf>
pub fn list_backups() -> Result<Vec<String>>
pub fn restore_backup(filename: &str) -> Result<()>
```

### `ask.rs`

```rust
pub fn ask(
    conn: &Connection,
    query: &str,
    debug: bool,
    type_filter: Option<&str>,
    include_archived: bool,
) -> Result<AskResult>

pub fn prime(
    conn: &Connection,
    task: &str,
    filter: FilterStrategy,
    primary_ids: &[String],
    include_archived: bool,
) -> Result<PrimeResult>
```

### `display.rs`

```rust
pub fn print_unit(unit: &Unit, outgoing: &[Link], incoming: &[Link], json: bool)
pub fn print_added(id: &str, json: bool)
pub fn print_linked(from_id, to_id, relationship, json: bool)
pub fn print_dropped(id: &str, json: bool)
pub fn print_units(units: &[Unit], json: bool)
pub fn print_inbox(items: &[InboxItem], json: bool)
pub fn print_backup_created(path: &Path, json: bool)
pub fn print_backups(names: &[String], json: bool)
pub fn print_restored(filename: &str, json: bool)
pub fn print_marked(id, kind, confidence, json: bool)
pub fn print_scan(result: &ScanResult, json: bool)
```

## Key Data Types

### `db.rs`

```rust
pub struct Unit {
    pub id: String,                    // UUIDv7
    pub content: String,
    pub unit_type: String,             // serde: "type"
    pub source: String,
    pub confidence: f64,               // 0.0 to 1.0, clamped
    pub verified: bool,
    pub archived: bool,
    pub tags: Vec<String>,             // stored as JSON TEXT in SQLite
    pub conditions: serde_json::Value, // stored as JSON TEXT in SQLite
    pub created: String,
    pub updated: String,
}

pub struct Link {
    pub from_id: String,
    pub to_id: String,
    pub relationship: String,
}

pub struct InboxItem {
    pub id: String,
    pub content: String,
    pub source: String,
    pub created: String,
}

pub struct ScanResult {
    pub low_confidence: Vec<Unit>,     // confidence < 0.6
    pub negative_marks: Vec<Unit>,     // has wrong or outdated marks
    pub contradictions: Vec<ContradictionPair>,
    pub orphans: Vec<Unit>,            // no links in either direction
    pub stale: Vec<Unit>,              // older than threshold, never marked
}

pub struct ContradictionPair {
    pub from_id: String,
    pub from_content: String,
    pub to_id: String,
    pub to_content: String,
}
```

### `ask.rs`

```rust
pub struct AskResult {
    pub query: String,
    pub units: Vec<MatchedUnit>,
    pub units_used: Vec<String>,           // IDs of units included in result
    pub debug: Option<DebugTrace>,         // present only with --debug
}

pub struct MatchedUnit {
    pub id: String,
    pub content: String,
    pub unit_type: String,
    pub tags: Vec<String>,
    pub source: String,
    pub is_direct_match: bool,             // true = FTS5 hit, false = 1-hop linked
    pub links: Vec<LinkInfo>,              // links to units outside the result set
}

pub struct LinkInfo {
    pub unit_id: String,
    pub relationship: String,
    pub title: String,                     // first 80 chars of linked unit's content
}

pub struct DebugTrace {
    pub fts_query: String,
    pub matches_per_query: HashMap<String, usize>,
    pub total_gathered: usize,
    pub units_in_result: usize,
}
```

## Dependencies

From `Cargo.toml`:

| Crate | Version | Purpose |
|-------|---------|---------|
| `anyhow` | 1 | Error handling with context |
| `clap` | 4 (derive) | CLI argument parsing |
| `dirs` | 6 | Home directory resolution |
| `rusqlite` | 0.32 (bundled) | SQLite database access with bundled libsqlite3 |
| `serde` | 1 (derive) | Serialization/deserialization for JSON I/O |
| `serde_json` | 1 | JSON parsing and generation |
| `uuid` | 1 (v7) | UUIDv7 generation for all record IDs |

## Environment Variables

| Variable | Effect |
|----------|--------|
| `SIMARIS_HOME` | Override base data directory (default: `~/.simaris`) |
| `SIMARIS_ENV` | When set to `dev`, appends `/dev` to data directory |
| `SIMARIS_WARN_BYTES` | Body-size warn threshold for `add`/`edit` (default `2048`) |
| `SIMARIS_HARD_BYTES` | Body-size hard threshold for `add`/`edit` (default `8192`) |
| `SIMARIS_BIN` | Path to `simaris` binary used by `simaris-server` (default: `simaris` via `PATH`) |
