# Simaris Architecture

Knowledge unit storage and retrieval system backed by SQLite with FTS5 full-text search and LLM-powered classification and synthesis.

## Module Overview

### `main.rs`

CLI entry point. Defines the `Cli` struct (clap `Parser`) with global `--json` and `--debug` flags, and the `Command` enum covering all subcommands: `Add`, `Show`, `Edit`, `Link`, `Drop`, `Promote`, `Inbox`, `List`, `Search`, `Backup`, `Restore`, `Digest`, `Mark`, `Ask`, `Scan`. Also defines the `UnitType` enum (fact, procedure, principle, preference, lesson, idea), the `Relationship` enum (related_to, part_of, depends_on, contradicts, supersedes, sourced_from), and the `MarkKind` enum (used, wrong, outdated, helpful) with associated confidence deltas. The `main` function handles `Restore` without a database connection, then opens a connection for all other commands and dispatches to `db::*` and `display::*` functions.

`main.rs:142-163` -- `UnitType` variants and `as_str()` mapping.
`main.rs:165-186` -- `Relationship` variants and `as_str()` mapping.
`main.rs:188-214` -- `MarkKind` variants, `as_str()`, and `delta()` (used=+0.05, wrong=-0.2, outdated=-0.1, helpful=+0.1).

### `db.rs`

Database layer. Owns connection setup, schema initialization, migration, all CRUD operations, FTS5 search, backup/restore, and the `scan` health-check. All IDs are UUIDv7 strings. Tables: `units`, `links`, `inbox`, `marks`, and the `units_fts` virtual table. The module is entirely synchronous using `rusqlite::Connection`.

### `digest.rs`

LLM classification pipeline. Takes raw inbox content and shells out to the `claude` CLI to break it into typed knowledge units with tags. Validates the LLM response (correct unit types, non-empty result set) before returning. Single responsibility: turn unstructured text into structured `DigestUnit` values.

### `ask.rs`

Three-phase retrieval pipeline: FTS5 search with 1-hop graph expansion, Haiku-based relevance filtering, and optional Sonnet-based synthesis. Returns an `AskResult` containing matched units, their IDs, an optional synthesized response, and an optional debug trace. All LLM calls shell out to the `claude` CLI.

### `display.rs`

Output formatting. Every user-facing print operation goes through this module. Each function takes a `json: bool` flag and either prints structured JSON (`serde_json::to_string_pretty`) or human-readable text. Handles units, inbox items, links, backups, marks, and scan results. Uses `short_id()` (first 8 chars of UUID) for compact display in text mode.

## Data Flow

### Capture: Drop to Typed Units

```
User input
    |
    v
simaris drop "content" --source cli
    |
    v
db::drop_item()                      -- Insert into `inbox` table, return UUIDv7
    |
    v
simaris digest
    |
    v
db::list_inbox()                     -- Fetch all pending inbox items
    |
    v
digest::check_claude()               -- Verify `claude` CLI is on PATH
    |
    v
digest::classify(content)            -- Shell out: claude -p --model sonnet "<prompt>"
    |                                   Parse JSON response into DigestResult
    |                                   Validate unit types against allowed set
    v
db::digest_inbox_item_multi()        -- Transaction:
    |                                     1. Insert each DigestUnit as a new unit (UUIDv7)
    |                                     2. Link non-overview units -> overview via part_of
    |                                     3. Delete inbox item
    v
Typed units in `units` table
(FTS5 index updated automatically via AFTER INSERT trigger)
```

### Retrieval: Ask the Knowledge Store

```
simaris ask "query" [--synthesize] [--type fact]
    |
    v
Phase 1: FTS5 Search + Graph Expansion
    |
    +-- sanitize_fts_query(query)     -- Strip operators, remove stop words,
    |                                    quote each term, join with OR
    +-- search_and_expand()
    |       |
    |       +-- db::search_units()    -- FTS5 MATCH on units_fts, JOIN to units
    |       |                            Capped at 15 direct matches
    |       +-- db::get_linked_unit_ids()  -- For each match, fetch 1-hop neighbors
    |       |                                 (both outgoing and incoming links)
    |       +-- db::get_unit()        -- Fetch full unit data for each linked unit
    |       v
    |   GatherResult { units, matches_per_query, direct_count, expansion_count }
    |
    v
Phase 2: Relevance Filter (Haiku)
    |
    +-- filter_relevance(query, gathered)
    |       |
    |       +-- Build summary: id, type, tags, first 150 chars of content
    |       +-- Shell out: claude -p --model haiku "<prompt>"
    |       |   Prompt asks for JSON: {"relevant_ids": [...]}
    |       +-- Parse response, filter gathered units
    |       +-- On ANY failure (CLI error, parse error, empty result): fall back
    |       |   to returning all gathered units unfiltered
    |       v
    |   (filtered_units, fallback_used)
    |
    v
Phase 3: Optional Synthesis (Sonnet)
    |
    +-- synthesize_response(query, filtered_units)  -- Only if --synthesize flag
    |       |
    |       +-- Build prompt with full unit content, types, sources, tags
    |       +-- Shell out: claude -p --model sonnet "<prompt>"
    |       |   (model overridable via SIMARIS_MODEL env var)
    |       +-- Return synthesized response text
    |       v
    |   String
    |
    v
AskResult { query, units, units_used, response, debug }
```

## LLM Integration

Both `digest.rs` and `ask.rs` invoke the `claude` CLI as a subprocess via `std::process::Command`. There is no HTTP client, no SDK -- the `claude` binary is the sole interface to LLM capabilities.

### Invocation pattern

All calls use the same form:

```
claude -p --model <model> "<prompt>"
```

The `-p` flag enables pipe/non-interactive mode. Model selection:

| Call site | Model | Override |
|-----------|-------|---------|
| `digest::classify` | sonnet | `SIMARIS_MODEL` env var |
| `ask::filter_relevance` | haiku | hardcoded |
| `ask::synthesize_response` | sonnet | `SIMARIS_MODEL` env var |

Model resolution (`digest.rs:22`, `ask.rs:450`):
```rust
fn model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "sonnet".to_string())
}
```

### Error handling

- `digest::check_claude()` (`digest.rs:26`) -- runs `which claude` before any LLM operation. Bails with a descriptive error if not found.
- `digest::classify` -- bails on non-zero exit status (includes stderr in error message), bails on JSON parse failure, bails on invalid unit types, bails on empty unit list.
- `ask::filter_relevance` -- gracefully degrades on any failure. If the `claude` command fails to spawn, exits non-zero, returns unparseable JSON, or filters everything out, the function returns all gathered units unfiltered with `fallback_used = true`. This is intentional: retrieval should never fail just because the relevance filter is unavailable.
- `ask::synthesize_response` -- bails on non-zero exit status (includes stderr). No fallback; synthesis failure is a hard error.

### Response parsing

Both modules strip markdown code fences (````json ... ```) before parsing, handling LLMs that wrap JSON in fences despite being told not to:

```rust
let json_str = response
    .strip_prefix("```json")
    .or_else(|| response.strip_prefix("```"))
    .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
    .unwrap_or(response);
```

## Database Lifecycle

### Connection setup (`db.rs:78-100`)

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

### Schema (`db.rs:374-455`)

Four tables and one virtual table:

```sql
-- Primary knowledge storage
units (
    id          TEXT PRIMARY KEY,     -- UUIDv7
    content     TEXT NOT NULL,
    type        TEXT NOT NULL,        -- CHECK: fact|procedure|principle|preference|lesson|idea
    source      TEXT NOT NULL DEFAULT 'inbox',
    confidence  REAL NOT NULL DEFAULT 1.0,
    verified    INTEGER NOT NULL DEFAULT 0,
    tags        TEXT NOT NULL DEFAULT '[]',    -- JSON array stored as TEXT
    conditions  TEXT NOT NULL DEFAULT '{}',    -- JSON object stored as TEXT
    created     TEXT NOT NULL DEFAULT (datetime('now')),
    updated     TEXT NOT NULL DEFAULT (datetime('now'))
)

-- Directed graph edges between units
links (
    from_id      TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    to_id        TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    relationship TEXT NOT NULL,       -- CHECK: related_to|part_of|depends_on|contradicts|supersedes|sourced_from
    PRIMARY KEY (from_id, to_id, relationship)
)

-- Unprocessed raw captures
inbox (
    id      TEXT PRIMARY KEY,         -- UUIDv7
    content TEXT NOT NULL,
    source  TEXT NOT NULL DEFAULT 'cli',
    created TEXT NOT NULL DEFAULT (datetime('now'))
)

-- Feedback signals on units
marks (
    id       TEXT PRIMARY KEY,        -- UUIDv7
    unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL,           -- CHECK: used|wrong|outdated|helpful
    created  TEXT NOT NULL DEFAULT (datetime('now'))
)

-- Full-text search (standalone, synced via triggers)
units_fts USING fts5(uuid, content, type, tags, source)
```

Indexes: `idx_links_to` on `links(to_id)`, `idx_marks_unit` on `marks(unit_id)`.

Three triggers keep `units_fts` in sync: `units_ai` (after insert), `units_ad` (after delete), `units_au` (after update -- delete + reinsert).

### Migration system

Schema versioning uses `PRAGMA user_version`. Current version: 1.

`migrate_to_uuid()` (`db.rs:102-372`) handles the v0-to-v1 migration (integer auto-increment IDs to UUIDv7 TEXT IDs). The migration:
1. Creates a backup via `create_backup()`.
2. Runs in a single transaction.
3. Renames old tables, creates new tables with TEXT PKs, maps old integer IDs to UUIDv7 strings, copies all data, verifies row counts match, recreates FTS and triggers, drops old tables, sets `user_version = 1`.

### Backup and prune

```rust
pub fn create_backup(conn: &Connection) -> Result<PathBuf>   // db.rs:862
```

Uses `VACUUM INTO` to create a consistent snapshot at `~/.simaris/backups/sanctuary-YYYYMMDD-HHMMSS.db`. After each backup, `prune_backups()` keeps only the 10 most recent files (sorted by filename, which is timestamp-ordered).

```rust
pub fn restore_backup(filename: &str) -> Result<()>          // db.rs:913
```

Removes WAL/SHM files, then copies the backup over the active database file. Operates without an open connection (handled before `connect()` in `main.rs`).

```rust
pub fn list_backups() -> Result<Vec<String>>                  // db.rs:893
```

Lists `sanctuary-*.db` files from the backup directory, sorted alphabetically.

## Key Public Functions

### `db.rs`

```rust
// Connection and schema
pub fn data_dir() -> PathBuf                                              // :56
pub fn db_path() -> PathBuf                                               // :70
pub fn backup_dir() -> PathBuf                                            // :74
pub fn connect() -> Result<Connection>                                    // :78

// Unit CRUD
pub fn add_unit(conn: &Connection, content: &str, unit_type: &str, source: &str) -> Result<String>    // :476
pub fn add_unit_full(conn: &Connection, content: &str, unit_type: &str, source: &str, tags: &[String]) -> Result<String>  // :608
pub fn update_unit(conn: &Connection, id: &str, content: Option<&str>, unit_type: Option<&str>, source: Option<&str>, tags: Option<&[String]>) -> Result<Unit>  // :623
pub fn get_unit(conn: &Connection, id: &str) -> Result<Unit>              // :485
pub fn list_units(conn: &Connection, type_filter: Option<&str>) -> Result<Vec<Unit>>   // :497
pub fn search_units(conn: &Connection, query: &str, type_filter: Option<&str>) -> Result<Vec<Unit>>   // :519

// Link operations
pub fn add_link(conn: &Connection, from_id: &str, to_id: &str, relationship: &str) -> Result<()>     // :596
pub fn get_links_from(conn: &Connection, id: &str) -> Result<Vec<Link>>   // :551
pub fn get_links_to(conn: &Connection, id: &str) -> Result<Vec<Link>>     // :566
pub fn get_linked_unit_ids(conn: &Connection, id: &str) -> Result<Vec<(String, String, String)>>      // :582

// Inbox operations
pub fn drop_item(conn: &Connection, content: &str, source: &str) -> Result<String>    // :803
pub fn get_inbox_item(conn: &Connection, id: &str) -> Result<InboxItem>   // :815
pub fn promote_item(conn: &Connection, inbox_id: &str, unit_type: &str) -> Result<String>  // :833
pub fn list_inbox(conn: &Connection) -> Result<Vec<InboxItem>>            // :846
pub fn delete_inbox_item(conn: &Connection, id: &str) -> Result<()>       // :625

// Digest (transactional inbox-to-unit conversion)
pub fn digest_inbox_item(conn: &Connection, inbox_id: &str, content: &str, unit_type: &str, source: &str, tags: &[String]) -> Result<String>  // :636
pub fn digest_inbox_item_multi(conn: &Connection, inbox_id: &str, units: &[crate::digest::DigestUnit], source: &str) -> Result<Vec<String>>    // :657

// Marks and confidence
pub fn add_mark(conn: &Connection, unit_id: &str, kind: &str, delta: f64) -> Result<f64>   // :698

// Health check
pub fn scan(conn: &Connection, stale_days: u32) -> Result<ScanResult>     // :732

// Backup/restore
pub fn create_backup(conn: &Connection) -> Result<PathBuf>                // :862
pub fn list_backups() -> Result<Vec<String>>                              // :893
pub fn restore_backup(filename: &str) -> Result<()>                       // :913
```

### `digest.rs`

```rust
pub fn check_claude() -> Result<()>                                       // :26
pub fn classify(content: &str) -> Result<DigestResult>                    // :38
```

### `ask.rs`

```rust
pub fn ask(conn: &Connection, query: &str, synthesize: bool, debug: bool, type_filter: Option<&str>) -> Result<AskResult>  // :68
```

### `display.rs`

```rust
pub fn print_unit(unit: &Unit, outgoing: &[Link], incoming: &[Link], json: bool)    // :9
pub fn print_added(id: &str, json: bool)                                             // :46
pub fn print_linked(from_id: &str, to_id: &str, relationship: &str, json: bool)     // :54
pub fn print_dropped(id: &str, json: bool)                                           // :69
pub fn print_units(units: &[Unit], json: bool)                                       // :77
pub fn print_inbox(items: &[InboxItem], json: bool)                                  // :106
pub fn print_backup_created(path: &Path, json: bool)                                 // :135
pub fn print_backups(names: &[String], json: bool)                                   // :146
pub fn print_restored(filename: &str, json: bool)                                    // :158
pub fn print_marked(id: &str, kind: &str, confidence: f64, json: bool)               // :166
pub fn print_scan(result: &ScanResult, json: bool)                                   // :195
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
    pub tags: Vec<String>,             // stored as JSON TEXT in SQLite
    pub conditions: serde_json::Value, // stored as JSON TEXT in SQLite
    pub created: String,               // datetime string
    pub updated: String,               // datetime string
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

### `digest.rs`

```rust
pub struct DigestUnit {
    pub unit_type: String,   // serde: "type"
    pub content: String,
    pub tags: Vec<String>,
    pub is_overview: bool,   // serde default: false
}

pub struct DigestResult {
    pub units: Vec<DigestUnit>,
}
```

### `ask.rs`

```rust
pub struct AskResult {
    pub query: String,
    pub units: Vec<MatchedUnit>,
    pub units_used: Vec<String>,          // IDs of units included in result
    pub response: Option<String>,          // present only with --synthesize
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
    pub filter_kept: usize,
    pub filter_total: usize,
    pub filter_fallback: bool,
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
| `SIMARIS_MODEL` | Override LLM model for digest and synthesis (default: `sonnet`). Does not affect the relevance filter, which is hardcoded to `haiku`. |
