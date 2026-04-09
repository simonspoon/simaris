# Simaris Data Model

Source of truth: `src/db.rs`

Database file: `~/.simaris/sanctuary.db` (or `$SIMARIS_HOME/sanctuary.db`; dev mode appends `/dev`).

Connection pragmas: `journal_mode=WAL`, `foreign_keys=ON`.

---

## SQLite Schema

### units

Primary knowledge store. Each row is a single atomic knowledge unit.

| Column | Type | Constraints | Default | Notes |
|---|---|---|---|---|
| id | TEXT | PRIMARY KEY | -- | UUIDv7 string |
| content | TEXT | NOT NULL | -- | The knowledge content |
| type | TEXT | NOT NULL, CHECK | -- | One of the `UnitType` enum values |
| source | TEXT | NOT NULL | `'inbox'` | Origin of the unit (e.g. `cli`, `phone`, `hook`) |
| confidence | REAL | NOT NULL | `1.0` | Score in [0.0, 1.0], adjusted by marks |
| verified | INTEGER | NOT NULL | `0` | Boolean (0/1) |
| tags | TEXT | NOT NULL | `'[]'` | JSON array of strings |
| conditions | TEXT | NOT NULL | `'{}'` | JSON object for conditional applicability |
| created | TEXT | NOT NULL | `datetime('now')` | ISO 8601 timestamp |
| updated | TEXT | NOT NULL | `datetime('now')` | ISO 8601 timestamp |

Type CHECK constraint (line 379):
```sql
CHECK(type IN ('fact','procedure','principle','preference','lesson','idea'))
```

Defined at line 374 (`initialize` function).

### links

Directed edges between units. Composite primary key prevents duplicate relationships.

| Column | Type | Constraints | Default | Notes |
|---|---|---|---|---|
| from_id | TEXT | NOT NULL, FK -> units(id) ON DELETE CASCADE | -- | Source unit |
| to_id | TEXT | NOT NULL, FK -> units(id) ON DELETE CASCADE | -- | Target unit |
| relationship | TEXT | NOT NULL, CHECK | -- | One of the `Relationship` enum values |

Primary key: `(from_id, to_id, relationship)`

Relationship CHECK constraint (line 392):
```sql
CHECK(relationship IN ('related_to','part_of','depends_on','contradicts','supersedes','sourced_from'))
```

Index: `idx_links_to ON links(to_id)` (line 398).

Defined at line 389.

### inbox

Staging area for raw thoughts before they become typed units.

| Column | Type | Constraints | Default | Notes |
|---|---|---|---|---|
| id | TEXT | PRIMARY KEY | -- | UUIDv7 string |
| content | TEXT | NOT NULL | -- | Raw content (empty/whitespace rejected at app layer) |
| source | TEXT | NOT NULL | `'cli'` | Where it came from |
| created | TEXT | NOT NULL | `datetime('now')` | ISO 8601 timestamp |

Defined at line 400.

### marks

Feedback signals on units. Each mark adjusts the unit's confidence score.

| Column | Type | Constraints | Default | Notes |
|---|---|---|---|---|
| id | TEXT | PRIMARY KEY | -- | UUIDv7 string |
| unit_id | TEXT | NOT NULL, FK -> units(id) ON DELETE CASCADE | -- | The unit being marked |
| kind | TEXT | NOT NULL, CHECK | -- | One of the `MarkKind` enum values |
| created | TEXT | NOT NULL | `datetime('now')` | ISO 8601 timestamp |

Kind CHECK constraint (line 410):
```sql
CHECK(kind IN ('used','wrong','outdated','helpful'))
```

Index: `idx_marks_unit ON marks(unit_id)` (line 414).

Defined at line 407.

### units_fts (FTS5 virtual table)

Full-text search index over units. Standalone (not a content table), kept in sync via triggers.

| FTS5 Column | Mirrors |
|---|---|
| uuid | units.id |
| content | units.content |
| type | units.type |
| tags | units.tags |
| source | units.source |

Defined at line 426. Search queries join back to `units` via `units_fts.uuid = units.id` (line 529).

#### Sync Triggers

Three triggers keep the FTS index consistent with the `units` table (lines 430-443):

| Trigger | Fires | Action |
|---|---|---|
| `units_ai` | AFTER INSERT ON units | INSERT into units_fts |
| `units_ad` | AFTER DELETE ON units | DELETE from units_fts WHERE uuid = old.id |
| `units_au` | AFTER UPDATE ON units | DELETE old row, INSERT new row |

The FTS table and triggers are created only if `units_fts` does not already exist (checked at line 417).

---

## Rust Structs

### Unit (line 8)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct Unit {
    pub id: String,
    pub content: String,
    #[serde(rename = "type")]
    pub unit_type: String,
    pub source: String,
    pub confidence: f64,
    pub verified: bool,
    pub tags: Vec<String>,
    pub conditions: serde_json::Value,
    pub created: String,
    pub updated: String,
}
```

The `verified` column is stored as INTEGER in SQLite but deserialized as `bool` via `row_to_unit` (line 457). Tags are stored as a JSON string in SQLite and deserialized to `Vec<String>`. Conditions are stored as a JSON string and deserialized to `serde_json::Value`.

### Link (line 22)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct Link {
    pub from_id: String,
    pub to_id: String,
    pub relationship: String,
}
```

### InboxItem (line 29)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub content: String,
    pub source: String,
    pub created: String,
}
```

### ScanResult (line 39)

Returned by the `scan` function. Aggregates all health signals from the store.

```rust
#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub low_confidence: Vec<Unit>,
    pub negative_marks: Vec<Unit>,
    pub contradictions: Vec<ContradictionPair>,
    pub orphans: Vec<Unit>,
    pub stale: Vec<Unit>,
}
```

- `low_confidence` -- units with `confidence < 0.6`
- `negative_marks` -- units that have at least one `wrong` or `outdated` mark
- `contradictions` -- pairs of units linked by `contradicts` (deduplicated via `from_id < to_id`)
- `orphans` -- units with no links in either direction
- `stale` -- units older than `stale_days` with zero marks of any kind

### ContradictionPair (line 48)

```rust
#[derive(Debug, Serialize)]
pub struct ContradictionPair {
    pub from_id: String,
    pub from_content: String,
    pub to_id: String,
    pub to_content: String,
}
```

---

## Enums

### UnitType

Constrained at the SQL layer via CHECK. Valid values:

| Value | Meaning |
|---|---|
| `fact` | A piece of factual knowledge |
| `procedure` | A how-to or process |
| `principle` | A guiding rule or heuristic |
| `preference` | A personal preference or opinion |
| `lesson` | Something learned from experience |
| `idea` | A speculative or unvalidated thought |

### Relationship

Constrained at the SQL layer via CHECK on `links.relationship`. Valid values:

| Value | Meaning |
|---|---|
| `related_to` | General association |
| `part_of` | Child-to-parent containment (used by digest to link sub-units to overview) |
| `depends_on` | Prerequisite relationship |
| `contradicts` | Conflicting information |
| `supersedes` | Replacement/update relationship |
| `sourced_from` | Provenance link |

### MarkKind

Constrained at the SQL layer via CHECK on `marks.kind`. Defined as a Rust enum `MarkKind` in `src/main.rs` (line 189) with `delta()` method (line 206).

| Value | Delta | Direction |
|---|---|---|
| `used` | +0.05 | Positive -- the unit was retrieved and applied |
| `helpful` | +0.1 | Positive -- the unit was explicitly helpful |
| `outdated` | -0.1 | Negative -- the unit is stale or no longer accurate |
| `wrong` | -0.2 | Negative -- the unit is factually incorrect |

---

## Confidence Scoring

Units start with `confidence = 1.0`. Each mark adjusts the score by its delta.

The adjustment is applied in SQL with clamping (line 718):
```sql
UPDATE units SET confidence = MAX(0.0, MIN(1.0, confidence + ?)) WHERE id = ?
```

This ensures the score never leaves the `[0.0, 1.0]` range regardless of how many marks accumulate.

**Low confidence threshold:** `0.6` (constant `LOW_CONFIDENCE_THRESHOLD`, line 37). Units below this threshold appear in `scan` results under `low_confidence`. A unit at exactly 0.6 does NOT appear (strict less-than comparison, verified by test at line 1297).

**Worked example:**
- New unit: confidence = 1.0
- Mark `helpful` (+0.1): MIN(1.0, 1.1) = 1.0 (clamped)
- Mark `wrong` (-0.2): MAX(0.0, 1.0 - 0.2) = 0.8
- Mark `wrong` (-0.2): 0.6
- Mark `wrong` (-0.2): 0.4 -- now below threshold, appears in scan

---

## ID Scheme

All IDs (units, inbox items, marks) use **UUIDv7** generated via `Uuid::now_v7().to_string()` (e.g. line 477).

UUIDv7 properties:
- Time-ordered: the first 48 bits encode a Unix timestamp in milliseconds, so lexicographic sort approximates chronological order.
- Globally unique without coordination: no auto-increment sequences, no central ID server.
- Stored as TEXT in SQLite (36-character hyphenated lowercase hex).

The scan deduplication for contradictions relies on string comparison of UUIDs (`from_id < to_id`, line 758).

---

## Migrations

### Version tracking

SQLite `PRAGMA user_version` tracks the schema version. Current version: **1**.

### v0 -> v1: Integer IDs to UUIDv7

Function: `migrate_to_uuid` (line 102). Triggered on `connect()` when `user_version == 0` and the `units` table already exists (line 86-96).

**Steps:**

1. Create a backup via `VACUUM INTO` (line 103).
2. Drop FTS table and triggers (lines 108-113).
3. Rename all four tables to `*_old` (lines 116-120).
4. Create new tables with `TEXT PRIMARY KEY` (lines 124-158).
5. Migrate `units_old` -- generate a new UUIDv7 for each row, build an `old_id -> new_uuid` HashMap (lines 161-217).
6. Migrate `inbox_old` -- new UUIDs, no ID mapping needed (lines 220-236).
7. Migrate `links_old` -- remap `from_id`/`to_id` via the HashMap; orphaned links (referencing missing units) are dropped with a warning to stderr (lines 238-258).
8. Migrate `marks_old` -- remap `unit_id` via the HashMap; orphaned marks dropped with a warning (lines 260-283).
9. Verify record counts -- units and inbox must match exactly; links and marks may have fewer (due to legitimately dropped orphans) but must not lose ALL records if any existed (lines 286-325).
10. Recreate FTS5 table, populate from migrated data, recreate triggers (lines 327-356).
11. Drop `*_old` tables (lines 358-364).
12. Set `PRAGMA user_version = 1` (line 367).

All steps run inside a single transaction (lines 105, 369). On any failure the transaction rolls back, leaving the original schema intact.

### Fresh install

The `initialize` function (line 374) uses `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` and conditionally creates the FTS table and triggers. After initialization it sets `user_version = 1` if still at 0 (lines 449-452).

---

## Backup System

- `create_backup` (line 862): uses `VACUUM INTO` to create a consistent snapshot at `~/.simaris/backups/sanctuary-{timestamp}.db`.
- `prune_backups` (line 874): retains only the 10 most recent backups (sorted by filename, which is timestamp-ordered).
- `list_backups` (line 893): lists backup files sorted by name.
- `restore_backup` (line 913): copies the backup file over the live database, removing WAL/SHM files first.
