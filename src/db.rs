use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct Link {
    pub from_id: String,
    pub to_id: String,
    pub relationship: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub content: String,
    pub source: String,
    pub created: String,
}

const LOW_CONFIDENCE_THRESHOLD: f64 = 0.6;

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub low_confidence: Vec<Unit>,
    pub negative_marks: Vec<Unit>,
    pub contradictions: Vec<ContradictionPair>,
    pub orphans: Vec<Unit>,
    pub stale: Vec<Unit>,
}

#[derive(Debug, Serialize)]
pub struct ContradictionPair {
    pub from_id: String,
    pub from_content: String,
    pub to_id: String,
    pub to_content: String,
}

/// One row of `scan --unstructured` output — a unit that lacks a frontmatter
/// block and is large enough to be worth rewriting. Ordered for rewrite
/// priority: aspect first, then mark_count DESC, then confidence DESC.
#[derive(Debug, Serialize)]
pub struct UnstructuredRow {
    pub id: String,
    #[serde(rename = "type")]
    pub unit_type: String,
    pub slugs: Vec<String>,
    pub marks: u32,
    pub confidence: f64,
    pub first_line: String,
}

/// Minimum body length (bytes) before a unit becomes eligible for rewrite.
/// Short prose — one sentence, a URL, a single fact — carries no schema
/// payload, so skip. Matches the spec in frontmatter-p2.
pub const UNSTRUCTURED_MIN_BYTES: usize = 200;

pub fn data_dir() -> PathBuf {
    let base = if let Ok(dir) = std::env::var("SIMARIS_HOME") {
        PathBuf::from(dir)
    } else {
        dirs::home_dir()
            .expect("Could not determine home directory")
            .join(".simaris")
    };
    if std::env::var("SIMARIS_ENV").as_deref() == Ok("dev") {
        return base.join("dev");
    }
    base
}

pub fn db_path() -> PathBuf {
    data_dir().join("sanctuary.db")
}

pub fn backup_dir() -> PathBuf {
    data_dir().join("backups")
}

pub fn connect() -> Result<Connection> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let conn = Connection::open(dir.join("sanctuary.db"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    let mut user_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version == 0 {
        // Check if this is an existing database with old INTEGER schema
        let has_units: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='units'",
            [],
            |row| row.get(0),
        )?;
        if has_units {
            migrate_to_uuid(&conn)?;
            user_version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        }
    }
    if user_version == 1 {
        migrate_add_aspect_type(&conn)?;
        user_version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    }
    if user_version == 2 {
        migrate_add_slugs(&conn)?;
    }

    initialize(&conn)?;
    Ok(conn)
}

fn migrate_to_uuid(conn: &Connection) -> Result<()> {
    create_backup(conn)?;

    let tx = conn.unchecked_transaction()?;

    // Drop FTS virtual table and triggers
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS units_ai;
         DROP TRIGGER IF EXISTS units_ad;
         DROP TRIGGER IF EXISTS units_au;
         DROP TABLE IF EXISTS units_fts;",
    )?;

    // Rename old tables
    tx.execute_batch(
        "ALTER TABLE units RENAME TO units_old;
         ALTER TABLE inbox RENAME TO inbox_old;
         ALTER TABLE links RENAME TO links_old;
         ALTER TABLE marks RENAME TO marks_old;",
    )?;

    // Create new tables with TEXT PRIMARY KEY
    tx.execute_batch(
        "CREATE TABLE units (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            type TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea')),
            source TEXT NOT NULL DEFAULT 'inbox',
            confidence REAL NOT NULL DEFAULT 1.0,
            verified INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]',
            conditions TEXT NOT NULL DEFAULT '{}',
            created TEXT NOT NULL DEFAULT (datetime('now')),
            updated TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE links (
            from_id TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            to_id TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            relationship TEXT NOT NULL CHECK(relationship IN (
                'related_to','part_of','depends_on','contradicts','supersedes','sourced_from')),
            PRIMARY KEY (from_id, to_id, relationship)
        );

        CREATE TABLE inbox (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'cli',
            created TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE marks (
            id TEXT PRIMARY KEY,
            unit_id TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            kind TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
            created TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    // Migrate units: generate UUIDs, build old_id -> new_uuid mapping
    let mut id_map: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
             FROM units_old",
        )?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            i64,
            String,
            String,
            String,
            f64,
            i32,
            String,
            String,
            String,
            String,
        )> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (
            old_id,
            content,
            unit_type,
            source,
            confidence,
            verified,
            tags,
            conditions,
            created,
            updated,
        ) in &rows
        {
            let new_uuid = Uuid::now_v7().to_string();
            tx.execute(
                "INSERT INTO units (id, content, type, source, confidence, verified, tags, conditions, created, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![new_uuid, content, unit_type, source, confidence, verified, tags, conditions, created, updated],
            )?;
            id_map.insert(*old_id, new_uuid);
        }
    }

    // Migrate inbox
    {
        let mut stmt = tx.prepare("SELECT id, content, source, created FROM inbox_old")?;
        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (_old_id, content, source, created) in &rows {
            let new_uuid = Uuid::now_v7().to_string();
            tx.execute(
                "INSERT INTO inbox (id, content, source, created) VALUES (?1, ?2, ?3, ?4)",
                params![new_uuid, content, source, created],
            )?;
        }
    }

    // Migrate links: map old integer IDs to new UUIDs
    {
        let mut stmt = tx.prepare("SELECT from_id, to_id, relationship FROM links_old")?;
        let rows: Vec<(i64, i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        for (old_from, old_to, relationship) in &rows {
            if let (Some(new_from), Some(new_to)) = (id_map.get(old_from), id_map.get(old_to)) {
                tx.execute(
                    "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
                    params![new_from, new_to, relationship],
                )?;
            } else {
                eprintln!(
                    "Warning: dropping orphaned link ({} -> {}, {})",
                    old_from, old_to, relationship
                );
            }
        }
    }

    // Migrate marks
    {
        let mut stmt = tx.prepare("SELECT id, unit_id, kind, created FROM marks_old")?;
        let rows: Vec<(i64, i64, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (_old_id, old_unit_id, kind, created) in &rows {
            if let Some(new_unit_uuid) = id_map.get(old_unit_id) {
                let new_mark_uuid = Uuid::now_v7().to_string();
                tx.execute(
                    "INSERT INTO marks (id, unit_id, kind, created) VALUES (?1, ?2, ?3, ?4)",
                    params![new_mark_uuid, new_unit_uuid, kind, created],
                )?;
            } else {
                eprintln!(
                    "Warning: dropping orphaned mark (unit_id={}, kind={})",
                    old_unit_id, kind
                );
            }
        }
    }

    // Verify record counts before dropping old tables
    {
        let old_units: i64 = tx.query_row("SELECT COUNT(*) FROM units_old", [], |r| r.get(0))?;
        let new_units: i64 = tx.query_row("SELECT COUNT(*) FROM units", [], |r| r.get(0))?;
        if old_units != new_units {
            anyhow::bail!(
                "Migration verification failed: units count mismatch ({} vs {})",
                old_units,
                new_units
            );
        }

        let old_inbox: i64 = tx.query_row("SELECT COUNT(*) FROM inbox_old", [], |r| r.get(0))?;
        let new_inbox: i64 = tx.query_row("SELECT COUNT(*) FROM inbox", [], |r| r.get(0))?;
        if old_inbox != new_inbox {
            anyhow::bail!(
                "Migration verification failed: inbox count mismatch ({} vs {})",
                old_inbox,
                new_inbox
            );
        }

        // Links and marks may have fewer records due to legitimately dropped orphans
        let old_links: i64 = tx.query_row("SELECT COUNT(*) FROM links_old", [], |r| r.get(0))?;
        let new_links: i64 = tx.query_row("SELECT COUNT(*) FROM links", [], |r| r.get(0))?;
        if old_links > 0 && new_links == 0 {
            anyhow::bail!(
                "Migration verification failed: all {} links were lost",
                old_links
            );
        }

        let old_marks: i64 = tx.query_row("SELECT COUNT(*) FROM marks_old", [], |r| r.get(0))?;
        let new_marks: i64 = tx.query_row("SELECT COUNT(*) FROM marks", [], |r| r.get(0))?;
        if old_marks > 0 && new_marks == 0 {
            anyhow::bail!(
                "Migration verification failed: all {} marks were lost",
                old_marks
            );
        }
    }

    // Create standalone FTS5 table
    tx.execute_batch(
        "CREATE VIRTUAL TABLE units_fts USING fts5(
            uuid, content, type, tags, source
        );",
    )?;

    // Populate FTS from migrated units
    tx.execute_batch(
        "INSERT INTO units_fts(uuid, content, type, tags, source)
         SELECT id, content, type, tags, source FROM units;",
    )?;

    // Create triggers
    tx.execute_batch(
        "CREATE TRIGGER units_ai AFTER INSERT ON units BEGIN
            INSERT INTO units_fts(uuid, content, type, tags, source)
            VALUES (new.id, new.content, new.type, new.tags, new.source);
        END;

        CREATE TRIGGER units_ad AFTER DELETE ON units BEGIN
            DELETE FROM units_fts WHERE uuid = old.id;
        END;

        CREATE TRIGGER units_au AFTER UPDATE ON units BEGIN
            DELETE FROM units_fts WHERE uuid = old.id;
            INSERT INTO units_fts(uuid, content, type, tags, source)
            VALUES (new.id, new.content, new.type, new.tags, new.source);
        END;",
    )?;

    // Drop old tables
    tx.execute_batch(
        "DROP TABLE IF EXISTS marks_old;
         DROP TABLE IF EXISTS links_old;
         DROP TABLE IF EXISTS inbox_old;
         DROP TABLE IF EXISTS units_old;",
    )?;

    // Set user_version
    tx.execute_batch("PRAGMA user_version = 1;")?;

    tx.commit()?;

    Ok(())
}

/// Migration v1→v2: Add 'aspect' to the units type CHECK constraint.
/// SQLite cannot ALTER CHECK constraints, so we rebuild the table.
/// Must also rebuild links/marks since their FK references get silently
/// repointed by ALTER TABLE RENAME.
fn migrate_add_aspect_type(conn: &Connection) -> Result<()> {
    create_backup(conn)?;

    // Must disable FK checks outside transaction (PRAGMA is no-op inside transactions)
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;

    let tx = conn.unchecked_transaction()?;

    // Drop FTS triggers and table
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS units_ai;
         DROP TRIGGER IF EXISTS units_ad;
         DROP TRIGGER IF EXISTS units_au;
         DROP TABLE IF EXISTS units_fts;",
    )?;

    // Rebuild all tables that reference units
    tx.execute_batch(
        "ALTER TABLE links RENAME TO links_v1;
         ALTER TABLE marks RENAME TO marks_v1;
         ALTER TABLE units RENAME TO units_v1;

         CREATE TABLE units (
             id          TEXT PRIMARY KEY,
             content     TEXT NOT NULL,
             type        TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea','aspect')),
             source      TEXT NOT NULL DEFAULT 'inbox',
             confidence  REAL NOT NULL DEFAULT 1.0,
             verified    INTEGER NOT NULL DEFAULT 0,
             tags        TEXT NOT NULL DEFAULT '[]',
             conditions  TEXT NOT NULL DEFAULT '{}',
             created     TEXT NOT NULL DEFAULT (datetime('now')),
             updated     TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO units (id, content, type, source, confidence, verified, tags, conditions, created, updated)
             SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated FROM units_v1;

         CREATE TABLE links (
             from_id      TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
             to_id        TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
             relationship TEXT NOT NULL CHECK(relationship IN (
                              'related_to','part_of','depends_on',
                              'contradicts','supersedes','sourced_from')),
             PRIMARY KEY (from_id, to_id, relationship)
         );
         CREATE INDEX idx_links_to ON links(to_id);
         INSERT INTO links (from_id, to_id, relationship)
             SELECT from_id, to_id, relationship FROM links_v1;

         CREATE TABLE marks (
             id       TEXT PRIMARY KEY,
             unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
             kind     TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
             created  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX idx_marks_unit ON marks(unit_id);
         INSERT INTO marks (id, unit_id, kind, created)
             SELECT id, unit_id, kind, created FROM marks_v1;

         DROP TABLE marks_v1;
         DROP TABLE links_v1;
         DROP TABLE units_v1;",
    )?;

    tx.execute_batch("PRAGMA user_version = 2;")?;

    tx.commit()?;

    // Re-enable FK checks after transaction completes
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    Ok(())
}

/// Migration v2→v3: Add slugs table for human-readable unit aliases.
/// Pure additive — no rebuild of existing tables, so foreign_keys stay ON.
fn migrate_add_slugs(conn: &Connection) -> Result<()> {
    create_backup(conn)?;

    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE slugs (
             slug     TEXT PRIMARY KEY,
             unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
             created  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX idx_slugs_unit ON slugs(unit_id);",
    )?;

    tx.execute_batch("PRAGMA user_version = 3;")?;

    tx.commit()?;

    Ok(())
}

fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS units (
            id          TEXT PRIMARY KEY,
            content     TEXT NOT NULL,
            type        TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea','aspect')),
            source      TEXT NOT NULL DEFAULT 'inbox',
            confidence  REAL NOT NULL DEFAULT 1.0,
            verified    INTEGER NOT NULL DEFAULT 0,
            tags        TEXT NOT NULL DEFAULT '[]',
            conditions  TEXT NOT NULL DEFAULT '{}',
            created     TEXT NOT NULL DEFAULT (datetime('now')),
            updated     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS links (
            from_id      TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            to_id        TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            relationship TEXT NOT NULL CHECK(relationship IN (
                             'related_to','part_of','depends_on',
                             'contradicts','supersedes','sourced_from')),
            PRIMARY KEY (from_id, to_id, relationship)
        );

        CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_id);

        CREATE TABLE IF NOT EXISTS inbox (
            id      TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            source  TEXT NOT NULL DEFAULT 'cli',
            created TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS marks (
            id       TEXT PRIMARY KEY,
            unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            kind     TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
            created  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_marks_unit ON marks(unit_id);

        CREATE TABLE IF NOT EXISTS slugs (
            slug     TEXT PRIMARY KEY,
            unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            created  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_slugs_unit ON slugs(unit_id);",
    )?;

    let fts_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='units_fts'",
        [],
        |row| row.get(0),
    )?;

    if !fts_exists {
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE units_fts USING fts5(
                uuid, content, type, tags, source
            );

            CREATE TRIGGER units_ai AFTER INSERT ON units BEGIN
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;

            CREATE TRIGGER units_ad AFTER DELETE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
            END;

            CREATE TRIGGER units_au AFTER UPDATE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            ",
        )?;
    }

    // Ensure user_version is set for fresh installs
    let user_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version < 3 {
        conn.execute_batch("PRAGMA user_version = 3;")?;
    }

    Ok(())
}

fn row_to_unit(row: &rusqlite::Row) -> rusqlite::Result<Unit> {
    let tags_str: String = row.get(6)?;
    let conditions_str: String = row.get(7)?;
    let verified_int: i32 = row.get(5)?;
    Ok(Unit {
        id: row.get(0)?,
        content: row.get(1)?,
        unit_type: row.get(2)?,
        source: row.get(3)?,
        confidence: row.get(4)?,
        verified: verified_int != 0,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        conditions: serde_json::from_str(&conditions_str)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        created: row.get(8)?,
        updated: row.get(9)?,
    })
}

pub fn add_unit(conn: &Connection, content: &str, unit_type: &str, source: &str) -> Result<String> {
    let id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO units (id, content, type, source) VALUES (?1, ?2, ?3, ?4)",
        params![id, content, unit_type, source],
    )?;
    // F15: materialize frontmatter `refs:` as related_to edges.
    sync_frontmatter_refs(conn, &id, content)?;
    Ok(id)
}

pub fn get_unit(conn: &Connection, id: &str) -> Result<Unit> {
    let unit = conn
        .query_row(
            "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
             FROM units WHERE id = ?1",
            params![id],
            row_to_unit,
        )
        .context(format!("Unit {id} not found"))?;
    Ok(unit)
}

pub fn list_units(conn: &Connection, type_filter: Option<&str>) -> Result<Vec<Unit>> {
    let units = match type_filter {
        Some(t) => {
            let mut stmt = conn.prepare(
                "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
                 FROM units WHERE type = ?1 ORDER BY created DESC",
            )?;
            stmt.query_map(params![t], row_to_unit)?
                .collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
                 FROM units ORDER BY created DESC",
            )?;
            stmt.query_map([], row_to_unit)?
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(units)
}

/// Sanitize a query string for FTS5 by quoting each word and joining with OR.
/// Strips hyphens (FTS5 interprets them as NOT) and other operator characters.
fn sanitize_fts_query(query: &str) -> String {
    // Replace hyphens with spaces so "pre-push" becomes "pre push" (two terms),
    // matching how FTS5's tokenizer splits hyphenated words.
    let query = query.replace('-', " ");
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            cleaned.to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"", w))
        .collect();

    if terms.is_empty() {
        return query.to_string();
    }
    terms.join(" OR ")
}

pub fn search_units(
    conn: &Connection,
    query: &str,
    type_filter: Option<&str>,
) -> Result<Vec<Unit>> {
    let sanitized = sanitize_fts_query(query);
    let units = match type_filter {
        Some(t) => {
            let mut stmt = conn.prepare(
                "SELECT u.id, u.content, u.type, u.source, u.confidence, u.verified, u.tags, u.conditions, u.created, u.updated
                 FROM units_fts f
                 JOIN units u ON u.id = f.uuid
                 WHERE units_fts MATCH ?1 AND u.type = ?2
                 ORDER BY rank",
            )?;
            stmt.query_map(params![sanitized, t], row_to_unit)?
                .collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT u.id, u.content, u.type, u.source, u.confidence, u.verified, u.tags, u.conditions, u.created, u.updated
                 FROM units_fts f
                 JOIN units u ON u.id = f.uuid
                 WHERE units_fts MATCH ?1
                 ORDER BY rank",
            )?;
            stmt.query_map(params![sanitized], row_to_unit)?
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(units)
}

pub fn get_links_from(conn: &Connection, id: &str) -> Result<Vec<Link>> {
    let mut stmt =
        conn.prepare("SELECT from_id, to_id, relationship FROM links WHERE from_id = ?1")?;
    let links = stmt
        .query_map(params![id], |row| {
            Ok(Link {
                from_id: row.get(0)?,
                to_id: row.get(1)?,
                relationship: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(links)
}

pub fn get_links_to(conn: &Connection, id: &str) -> Result<Vec<Link>> {
    let mut stmt =
        conn.prepare("SELECT from_id, to_id, relationship FROM links WHERE to_id = ?1")?;
    let links = stmt
        .query_map(params![id], |row| {
            Ok(Link {
                from_id: row.get(0)?,
                to_id: row.get(1)?,
                relationship: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(links)
}

/// Get all units linked from or to a given unit ID
pub fn get_linked_unit_ids(conn: &Connection, id: &str) -> Result<Vec<(String, String, String)>> {
    // Returns (linked_unit_id, relationship, direction)
    let mut ids = vec![];
    let outgoing = get_links_from(conn, id)?;
    for link in outgoing {
        ids.push((link.to_id, link.relationship, "outgoing".to_string()));
    }
    let incoming = get_links_to(conn, id)?;
    for link in incoming {
        ids.push((link.from_id, link.relationship, "incoming".to_string()));
    }
    Ok(ids)
}

pub fn add_link(conn: &Connection, from_id: &str, to_id: &str, relationship: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
        params![from_id, to_id, relationship],
    )
    .context(format!(
        "Failed to create link {from_id} -> {to_id} ({relationship})"
    ))?;
    Ok(())
}

/// Idempotent version of `add_link` — silently does nothing when the edge
/// already exists. Used by auto-edge paths (auto_link, sync_frontmatter_refs)
/// where double-write is expected and not an error. User-initiated
/// `simaris link` stays strict via `add_link`.
pub fn ensure_link(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    relationship: &str,
) -> Result<bool> {
    let changes = conn.execute(
        "INSERT OR IGNORE INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
        params![from_id, to_id, relationship],
    )?;
    Ok(changes > 0)
}

/// Materialize frontmatter `refs:` entries as `related_to` graph edges on
/// the given unit (F15).
///
/// Each ref is resolved as a UUID or slug. On resolve failure (unknown id
/// or slug, typo from LLM) we emit a stderr warning and skip — never fail
/// the enclosing write, to preserve unit integrity. Self-refs are skipped.
///
/// Returns `(created, skipped)` counts. `created` counts edges newly
/// inserted; idempotent re-runs produce zero.
pub fn sync_frontmatter_refs(
    conn: &Connection,
    unit_id: &str,
    content: &str,
) -> Result<(usize, usize)> {
    let refs = crate::frontmatter::extract_refs(content);
    if refs.is_empty() {
        return Ok((0, 0));
    }
    let mut created = 0;
    let mut skipped = 0;
    for entry in refs {
        // Strip optional `(uuid)` parenthetical hint so slugs with hints
        // like "verifier (019d93...)" resolve by slug.
        let token = entry
            .split_whitespace()
            .next()
            .unwrap_or(&entry)
            .to_string();
        match resolve_id(conn, &token) {
            Ok(target) if target == unit_id => {
                skipped += 1;
            }
            Ok(target) => {
                if ensure_link(conn, unit_id, &target, "related_to")? {
                    created += 1;
                } else {
                    skipped += 1;
                }
            }
            Err(_) => {
                eprintln!(
                    "simaris: warning — frontmatter ref `{entry}` does not resolve to a known unit; graph edge skipped"
                );
                skipped += 1;
            }
        }
    }
    Ok((created, skipped))
}

/// Create `related_to` links between a unit and existing units sharing 2+ tags.
/// Skips self-links and pairs with any existing link. Returns count of links created.
pub fn auto_link(conn: &Connection, unit_id: &str) -> Result<usize> {
    let unit = get_unit(conn, unit_id)?;
    if unit.tags.len() < 2 {
        return Ok(0);
    }

    let unit_tags: Vec<String> = unit.tags.iter().map(|t| t.to_lowercase()).collect();

    let mut stmt = conn.prepare("SELECT id, tags FROM units WHERE id != ?1 AND tags != '[]'")?;
    let candidates: Vec<(String, Vec<String>)> = stmt
        .query_map(params![unit_id], |row| {
            let id: String = row.get(0)?;
            let tags_str: String = row.get(1)?;
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
            Ok((id, tags))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut created = 0;
    for (cand_id, cand_tags) in &candidates {
        let shared = cand_tags
            .iter()
            .filter(|t| unit_tags.contains(&t.to_lowercase()))
            .count();
        if shared < 2 {
            continue;
        }

        // Skip if any link already exists between the pair
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM links WHERE (from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1))",
            params![unit_id, cand_id],
            |row| row.get(0),
        )?;
        if exists {
            continue;
        }

        conn.execute(
            "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, 'related_to')",
            params![unit_id, cand_id],
        )?;
        created += 1;
    }

    Ok(created)
}

pub fn add_unit_full(
    conn: &Connection,
    content: &str,
    unit_type: &str,
    source: &str,
    tags: &[String],
) -> Result<String> {
    let id = Uuid::now_v7().to_string();
    let tags_json = serde_json::to_string(tags)?;
    conn.execute(
        "INSERT INTO units (id, content, type, source, tags) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, content, unit_type, source, tags_json],
    )?;
    // F15: materialize frontmatter `refs:` as related_to edges.
    sync_frontmatter_refs(conn, &id, content)?;
    Ok(id)
}

pub fn update_unit(
    conn: &Connection,
    id: &str,
    content: Option<&str>,
    unit_type: Option<&str>,
    source: Option<&str>,
    tags: Option<&[String]>,
) -> Result<Unit> {
    let unit = get_unit(conn, id)?;

    let new_content = content.unwrap_or(&unit.content);
    let new_type = unit_type.unwrap_or(&unit.unit_type);
    let new_source = source.unwrap_or(&unit.source);
    let new_tags = match tags {
        Some(t) => serde_json::to_string(t)?,
        None => serde_json::to_string(&unit.tags)?,
    };

    conn.execute(
        "UPDATE units SET content = ?1, type = ?2, source = ?3, tags = ?4, updated = datetime('now') WHERE id = ?5",
        params![new_content, new_type, new_source, new_tags, id],
    )?;

    // F15: materialize frontmatter `refs:` as related_to edges. Runs on
    // every update (idempotent via INSERT OR IGNORE) so rewrites that only
    // tweak frontmatter still land fresh edges.
    sync_frontmatter_refs(conn, id, new_content)?;

    get_unit(conn, id)
}

pub fn delete_unit(conn: &Connection, id: &str) -> Result<()> {
    let changes = conn.execute("DELETE FROM units WHERE id = ?1", params![id])?;
    if changes == 0 {
        anyhow::bail!("Unit not found: {id}");
    }
    Ok(())
}

#[allow(dead_code)]
pub fn delete_inbox_item(conn: &Connection, id: &str) -> Result<()> {
    let affected = conn.execute("DELETE FROM inbox WHERE id = ?1", params![id])?;
    if affected == 0 {
        anyhow::bail!("Inbox item {id} not found");
    }
    Ok(())
}

/// Atomically create a unit from an inbox item and delete the inbox item.
/// Used by the digest command to ensure no duplicates on failure.
#[allow(dead_code)]
pub fn digest_inbox_item(
    conn: &Connection,
    inbox_id: &str,
    content: &str,
    unit_type: &str,
    source: &str,
    tags: &[String],
) -> Result<String> {
    let tx = conn.unchecked_transaction()?;
    let unit_id = Uuid::now_v7().to_string();
    let tags_json = serde_json::to_string(tags)?;
    tx.execute(
        "INSERT INTO units (id, content, type, source, tags) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![unit_id, content, unit_type, source, tags_json],
    )?;
    tx.execute("DELETE FROM inbox WHERE id = ?1", params![inbox_id])?;
    tx.commit()?;
    Ok(unit_id)
}

/// Atomically create multiple units from an inbox item, link children to overview, delete inbox item.
pub fn digest_inbox_item_multi(
    conn: &Connection,
    inbox_id: &str,
    units: &[crate::digest::DigestUnit],
    source: &str,
) -> Result<Vec<String>> {
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::new();
    let mut overview_id: Option<String> = None;

    for unit in units {
        let id = Uuid::now_v7().to_string();
        let tags_json = serde_json::to_string(&unit.tags)?;
        tx.execute(
            "INSERT INTO units (id, content, type, source, tags) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, unit.content, unit.unit_type, source, tags_json],
        )?;

        if unit.is_overview {
            overview_id = Some(id.clone());
        }
        ids.push(id);
    }

    // Link non-overview units to the overview via part_of
    if let Some(ref ov_id) = overview_id {
        for (id, unit) in ids.iter().zip(units.iter()) {
            if !unit.is_overview {
                tx.execute(
                    "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
                    params![id, ov_id, "part_of"],
                )?;
            }
        }
    }

    // Auto-link each new unit to existing units sharing 2+ tags
    for id in &ids {
        auto_link(&tx, id)?;
    }

    tx.execute("DELETE FROM inbox WHERE id = ?1", params![inbox_id])?;
    tx.commit()?;
    Ok(ids)
}

pub fn add_mark(conn: &Connection, unit_id: &str, kind: &str, delta: f64) -> Result<f64> {
    // Verify unit exists
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM units WHERE id = ?)",
        params![unit_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Unit {} not found", unit_id);
    }

    // Insert mark
    let mark_id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO marks (id, unit_id, kind) VALUES (?, ?, ?)",
        params![mark_id, unit_id, kind],
    )?;

    // Update confidence with clamping
    conn.execute(
        "UPDATE units SET confidence = MAX(0.0, MIN(1.0, confidence + ?)) WHERE id = ?",
        params![delta, unit_id],
    )?;

    // Return new confidence
    let confidence: f64 = conn.query_row(
        "SELECT confidence FROM units WHERE id = ?",
        params![unit_id],
        |row| row.get(0),
    )?;

    Ok(confidence)
}

pub fn scan(conn: &Connection, stale_days: u32) -> Result<ScanResult> {
    // Low confidence
    let mut stmt = conn.prepare(
        "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
         FROM units WHERE confidence < ?1",
    )?;
    let low_confidence = stmt
        .query_map(params![LOW_CONFIDENCE_THRESHOLD], row_to_unit)?
        .collect::<Result<Vec<_>, _>>()?;

    // Negative marks (units with wrong or outdated marks in marks table)
    let mut stmt = conn.prepare(
        "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
         FROM units u
         WHERE EXISTS (SELECT 1 FROM marks WHERE unit_id = u.id AND kind IN ('wrong', 'outdated'))",
    )?;
    let negative_marks = stmt
        .query_map([], row_to_unit)?
        .collect::<Result<Vec<_>, _>>()?;

    // Contradictions (dedup via from_id < to_id)
    let mut stmt = conn.prepare(
        "SELECT l.from_id, u1.content, l.to_id, u2.content
         FROM links l
         JOIN units u1 ON u1.id = l.from_id
         JOIN units u2 ON u2.id = l.to_id
         WHERE l.relationship = 'contradicts' AND l.from_id < l.to_id",
    )?;
    let contradictions = stmt
        .query_map([], |row| {
            Ok(ContradictionPair {
                from_id: row.get(0)?,
                from_content: row.get(1)?,
                to_id: row.get(2)?,
                to_content: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Orphans (no links in either direction)
    let mut stmt = conn.prepare(
        "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
         FROM units u
         WHERE NOT EXISTS (SELECT 1 FROM links WHERE from_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM links WHERE to_id = u.id)",
    )?;
    let orphans = stmt
        .query_map([], row_to_unit)?
        .collect::<Result<Vec<_>, _>>()?;

    // Stale (old and never marked)
    let stale_modifier = format!("-{stale_days} days");
    let mut stmt = conn.prepare(
        "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
         FROM units u
         WHERE created < datetime('now', ?1)
           AND NOT EXISTS (SELECT 1 FROM marks WHERE unit_id = u.id)",
    )?;
    let stale = stmt
        .query_map(params![stale_modifier], row_to_unit)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ScanResult {
        low_confidence,
        negative_marks,
        contradictions,
        orphans,
        stale,
    })
}

/// List units that lack a frontmatter block and carry enough body to warrant
/// a rewrite. Ordered for rewrite priority: aspect type first, then mark
/// count descending, then confidence descending, then `updated` descending
/// as a final tiebreak.
///
/// `type_filter` narrows to a single unit type ("aspect", "procedure", …).
///
/// The SQL pre-filter uses `length(content) >= 200` as a cheap gate (SQLite's
/// `length()` on TEXT counts characters, not bytes — close enough for a
/// pre-filter). Rust then confirms byte length and confirms the absence of a
/// parseable frontmatter block via the P0 parser, so malformed-YAML units
/// still surface as unstructured.
pub fn scan_unstructured(
    conn: &Connection,
    type_filter: Option<&str>,
    include_superseded: bool,
    include_unschemaed: bool,
) -> Result<Vec<UnstructuredRow>> {
    // Pre-filter: length guard + optional type match + sort by rewrite
    // priority. Mark count comes from a left-join on a grouped subquery so
    // units with zero marks still appear (as 0).
    //
    // Units with an incoming `supersedes` edge are already obsolete — the
    // superseding unit carries the current content. Default excludes them
    // from rewrite-priority ranking; `--include-superseded` opts back in
    // for debugging or completeness audits (F14).
    let supersede_clause = if include_superseded {
        ""
    } else {
        "AND NOT EXISTS (
             SELECT 1 FROM links l
             WHERE l.to_id = u.id AND l.relationship = 'supersedes'
         )"
    };
    // `idea` and `preference` carry no frontmatter schema (per
    // `rewrite::skeleton_for`), so `rewrite --suggest` emits body-only and
    // the unit can never satisfy the `has_frontmatter` drop signal that
    // pulls it out of `--unstructured`. Default excludes them so the scan
    // measures real migration progress; `--include-unschemaed` opts back
    // in for raw audits (F18).
    let unschemaed_clause = if include_unschemaed {
        ""
    } else {
        "AND u.type NOT IN ('idea','preference')"
    };
    let sql = format!(
        "SELECT u.id, u.content, u.type, u.confidence,
                COALESCE(m.n, 0) AS mark_count
         FROM units u
         LEFT JOIN (
             SELECT unit_id, COUNT(*) AS n FROM marks GROUP BY unit_id
         ) m ON m.unit_id = u.id
         WHERE length(u.content) >= 200
           AND (?1 IS NULL OR u.type = ?1)
           {supersede_clause}
           {unschemaed_clause}
         ORDER BY (u.type = 'aspect') DESC,
                  mark_count DESC,
                  u.confidence DESC,
                  u.updated DESC"
    );
    let sql = sql.as_str();
    let mut stmt = conn.prepare(sql)?;

    let rows = stmt
        .query_map(params![type_filter], |row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let unit_type: String = row.get(2)?;
            let confidence: f64 = row.get(3)?;
            let marks: i64 = row.get(4)?;
            Ok((id, content, unit_type, confidence, marks))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out: Vec<UnstructuredRow> = Vec::new();
    for (id, content, unit_type, confidence, marks) in rows {
        // Byte-accurate body-size gate — spec says 200 B, SQL used chars.
        if content.len() < UNSTRUCTURED_MIN_BYTES {
            continue;
        }
        // Skip units that already have a valid frontmatter block.
        if crate::frontmatter::has_frontmatter(&content) {
            continue;
        }
        let slugs = get_slugs_for_unit(conn, &id)?;
        let first_line = content
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string();
        out.push(UnstructuredRow {
            id,
            unit_type,
            slugs,
            marks: marks.max(0) as u32,
            confidence,
            first_line,
        });
    }
    Ok(out)
}

pub fn drop_item(conn: &Connection, content: &str, source: &str) -> Result<String> {
    if content.trim().is_empty() {
        anyhow::bail!("Content cannot be empty");
    }
    let id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO inbox (id, content, source) VALUES (?1, ?2, ?3)",
        params![id, content, source],
    )?;
    Ok(id)
}

pub fn get_inbox_item(conn: &Connection, id: &str) -> Result<InboxItem> {
    let item = conn
        .query_row(
            "SELECT id, content, source, created FROM inbox WHERE id = ?1",
            params![id],
            |row| {
                Ok(InboxItem {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    source: row.get(2)?,
                    created: row.get(3)?,
                })
            },
        )
        .context(format!("Inbox item {id} not found"))?;
    Ok(item)
}

pub fn promote_item(conn: &Connection, inbox_id: &str, unit_type: &str) -> Result<String> {
    let tx = conn.unchecked_transaction()?;
    let item = get_inbox_item(&tx, inbox_id)?;
    let unit_id = Uuid::now_v7().to_string();
    tx.execute(
        "INSERT INTO units (id, content, type, source) VALUES (?1, ?2, ?3, ?4)",
        params![unit_id, item.content, unit_type, item.source],
    )?;
    tx.execute("DELETE FROM inbox WHERE id = ?1", params![inbox_id])?;
    tx.commit()?;
    Ok(unit_id)
}

pub fn list_inbox(conn: &Connection) -> Result<Vec<InboxItem>> {
    let mut stmt =
        conn.prepare("SELECT id, content, source, created FROM inbox ORDER BY created ASC")?;
    let items = stmt
        .query_map([], |row| {
            Ok(InboxItem {
                id: row.get(0)?,
                content: row.get(1)?,
                source: row.get(2)?,
                created: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

pub fn create_backup(conn: &Connection) -> Result<PathBuf> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir)?;
    let timestamp = conn.query_row("SELECT strftime('%Y%m%d-%H%M%S', 'now')", [], |r| {
        r.get::<_, String>(0)
    })?;
    let backup_path = dir.join(format!("sanctuary-{timestamp}.db"));
    conn.execute("VACUUM INTO ?1", [backup_path.to_str().unwrap()])?;
    prune_backups(&dir, 10)?;
    Ok(backup_path)
}

fn prune_backups(dir: &Path, keep: usize) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("sanctuary-") && n.ends_with(".db"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.len() > keep {
        for entry in &entries[..entries.len() - keep] {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

pub fn list_backups() -> Result<Vec<String>> {
    let dir = backup_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name.starts_with("sanctuary-") && name.ends_with(".db") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    Ok(names)
}

pub fn restore_backup(filename: &str) -> Result<()> {
    let backup_path = backup_dir().join(filename);
    if !backup_path.exists() {
        anyhow::bail!("Backup not found: {filename}");
    }
    let db = db_path();
    let _ = std::fs::remove_file(db.with_extension("db-wal"));
    let _ = std::fs::remove_file(db.with_extension("db-shm"));
    std::fs::copy(&backup_path, &db)?;
    Ok(())
}

/// Row shape for `slugs` table reads.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlugRow {
    pub slug: String,
    pub unit_id: String,
    pub created: String,
}

/// Validate a slug per the slug grammar:
/// - non-empty, max 64 chars
/// - first char in `[a-z_]`
/// - every char in `[a-z0-9_-]`
/// - reject canonical-form UUID (length 36 + parses as UUID) to avoid
///   collision with unit ids in the resolver path
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        anyhow::bail!("Slug must not be empty");
    }
    if slug.len() > 64 {
        anyhow::bail!("Slug exceeds 64-char cap (got {})", slug.len());
    }
    if slug.len() == 36 && Uuid::parse_str(slug).is_ok() {
        anyhow::bail!("Slug must not be UUID-shaped: '{slug}'");
    }
    let mut chars = slug.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_lowercase() || first == '_') {
        anyhow::bail!("Slug first char must be [a-z_], got '{first}' in '{slug}'");
    }
    for c in slug.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if !ok {
            anyhow::bail!("Slug char '{c}' not in [a-z0-9_-] (slug '{slug}')");
        }
    }
    Ok(())
}

/// Set (create or move) a slug to point at `unit_id`.
/// Same slug + new unit_id reassigns ownership while preserving `created`.
/// Unknown unit_id surfaces as an FK error mapped with the offending id.
pub fn set_slug(conn: &Connection, slug: &str, unit_id: &str) -> Result<()> {
    validate_slug(slug)?;
    conn.execute(
        "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)
         ON CONFLICT(slug) DO UPDATE SET unit_id = excluded.unit_id",
        params![slug, unit_id],
    )
    .with_context(|| format!("unit_id '{unit_id}' not found"))?;
    Ok(())
}

/// Delete a slug. Returns true if a row was removed, false if no match.
/// Tolerates any input — does not run validate_slug.
pub fn unset_slug(conn: &Connection, slug: &str) -> Result<bool> {
    let n = conn.execute("DELETE FROM slugs WHERE slug = ?1", params![slug])?;
    Ok(n > 0)
}

/// List every slug in the DB, ordered by slug ASC.
pub fn list_slugs(conn: &Connection) -> Result<Vec<SlugRow>> {
    let mut stmt = conn.prepare("SELECT slug, unit_id, created FROM slugs ORDER BY slug ASC")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SlugRow {
                slug: row.get(0)?,
                unit_id: row.get(1)?,
                created: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// All slugs pointing at a given unit, ordered by slug ASC.
/// Unknown unit returns `Ok(vec![])`, not an error.
pub fn get_slugs_for_unit(conn: &Connection, unit_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT slug FROM slugs WHERE unit_id = ?1 ORDER BY slug ASC")?;
    let rows = stmt
        .query_map(params![unit_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Resolve a CLI-supplied identifier to a unit id.
/// Order: existing unit id wins (collision invariant), then slug lookup, else bail.
/// Empty input short-circuits without a DB hit.
pub fn resolve_id(conn: &Connection, id_or_slug: &str) -> Result<String> {
    if id_or_slug.is_empty() {
        anyhow::bail!("No unit or slug matches ''");
    }
    let unit_hit: Option<String> = conn
        .query_row(
            "SELECT id FROM units WHERE id = ?1",
            params![id_or_slug],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = unit_hit {
        return Ok(id);
    }
    let slug_hit: Option<String> = conn
        .query_row(
            "SELECT unit_id FROM slugs WHERE slug = ?1",
            params![id_or_slug],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = slug_hit {
        return Ok(id);
    }
    anyhow::bail!("No unit or slug matches '{id_or_slug}'");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        initialize(&conn).unwrap();
        conn
    }

    #[test]
    fn test_schema_creation() {
        let conn = memory_db();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('units','links','inbox','slugs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_add_unit() {
        let conn = memory_db();
        let id = add_unit(&conn, "test content", "fact", "inbox").unwrap();
        assert!(!id.is_empty());
        let unit = get_unit(&conn, &id).unwrap();
        assert_eq!(unit.content, "test content");
        assert_eq!(unit.unit_type, "fact");
        assert_eq!(unit.source, "inbox");
        assert_eq!(unit.confidence, 1.0);
        assert!(!unit.verified);
    }

    #[test]
    fn test_constraint_violations() {
        let conn = memory_db();
        let result = add_unit(&conn, "bad type", "invalid_type", "inbox");
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_link() {
        let conn = memory_db();
        let id_a = add_unit(&conn, "unit a", "fact", "inbox").unwrap();
        let id_b = add_unit(&conn, "unit b", "fact", "inbox").unwrap();
        add_link(&conn, &id_a, &id_b, "related_to").unwrap();
        let result = add_link(&conn, &id_a, &id_b, "related_to");
        assert!(result.is_err());
    }

    #[test]
    fn test_cascade_delete() {
        let conn = memory_db();
        let id_a = add_unit(&conn, "unit a", "fact", "inbox").unwrap();
        let id_b = add_unit(&conn, "unit b", "fact", "inbox").unwrap();
        add_link(&conn, &id_a, &id_b, "related_to").unwrap();

        conn.execute("DELETE FROM units WHERE id = ?1", params![id_a])
            .unwrap();

        let links = get_links_to(&conn, &id_b).unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn test_inbox_schema_created() {
        let conn = memory_db();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='inbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_add_inbox_item_defaults() {
        let conn = memory_db();
        let id = drop_item(&conn, "raw thought", "cli").unwrap();
        assert!(!id.is_empty());
        let items = list_inbox(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "raw thought");
        assert_eq!(items[0].source, "cli");
        assert!(!items[0].created.is_empty());
    }

    #[test]
    fn test_add_inbox_item_custom_source() {
        let conn = memory_db();
        drop_item(&conn, "phone idea", "phone").unwrap();
        let items = list_inbox(&conn).unwrap();
        assert_eq!(items[0].source, "phone");
    }

    #[test]
    fn test_get_inbox_items_ordering() {
        let conn = memory_db();
        drop_item(&conn, "first", "cli").unwrap();
        drop_item(&conn, "second", "cli").unwrap();
        drop_item(&conn, "third", "cli").unwrap();
        let items = list_inbox(&conn).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].content, "first");
        assert_eq!(items[1].content, "second");
        assert_eq!(items[2].content, "third");
    }

    #[test]
    fn test_add_inbox_item_empty_content_rejected() {
        let conn = memory_db();
        assert!(drop_item(&conn, "", "cli").is_err());
        assert!(drop_item(&conn, "   ", "cli").is_err());
    }

    #[test]
    fn test_promote_item() {
        let conn = memory_db();
        let inbox_id = drop_item(&conn, "promote me", "cli").unwrap();
        let unit_id = promote_item(&conn, &inbox_id, "fact").unwrap();
        let unit = get_unit(&conn, &unit_id).unwrap();
        assert_eq!(unit.content, "promote me");
        assert_eq!(unit.unit_type, "fact");
        assert_eq!(unit.source, "cli");
    }

    #[test]
    fn test_promote_deletes_inbox_item() {
        let conn = memory_db();
        let inbox_id = drop_item(&conn, "ephemeral", "cli").unwrap();
        promote_item(&conn, &inbox_id, "idea").unwrap();
        let items = list_inbox(&conn).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_promote_nonexistent_fails() {
        let conn = memory_db();
        let result = promote_item(&conn, "nonexistent-id", "fact");
        assert!(result.is_err());
    }

    #[test]
    fn test_promote_preserves_source() {
        let conn = memory_db();
        let inbox_id = drop_item(&conn, "phone thought", "phone").unwrap();
        let unit_id = promote_item(&conn, &inbox_id, "lesson").unwrap();
        let unit = get_unit(&conn, &unit_id).unwrap();
        assert_eq!(unit.source, "phone");
    }

    #[test]
    fn test_list_all_units() {
        let conn = memory_db();
        add_unit(&conn, "fact one", "fact", "test").unwrap();
        add_unit(&conn, "procedure one", "procedure", "test").unwrap();
        add_unit(&conn, "principle one", "principle", "test").unwrap();
        let units = list_units(&conn, None).unwrap();
        assert_eq!(units.len(), 3);
    }

    #[test]
    fn test_list_filter_by_type() {
        let conn = memory_db();
        add_unit(&conn, "fact one", "fact", "test").unwrap();
        add_unit(&conn, "fact two", "fact", "test").unwrap();
        add_unit(&conn, "procedure one", "procedure", "test").unwrap();
        let units = list_units(&conn, Some("fact")).unwrap();
        assert_eq!(units.len(), 2);
        assert!(units.iter().all(|u| u.unit_type == "fact"));
    }

    #[test]
    fn test_list_empty() {
        let conn = memory_db();
        let units = list_units(&conn, None).unwrap();
        assert!(units.is_empty());
    }

    #[test]
    fn test_search_finds_match() {
        let conn = memory_db();
        add_unit(&conn, "caching improves performance", "fact", "test").unwrap();
        add_unit(&conn, "deploy with cargo install", "procedure", "test").unwrap();
        let units = search_units(&conn, "caching", None).unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].content, "caching improves performance");
    }

    #[test]
    fn test_search_no_match() {
        let conn = memory_db();
        add_unit(&conn, "some content here", "fact", "test").unwrap();
        let units = search_units(&conn, "nonexistent", None).unwrap();
        assert!(units.is_empty());
    }

    #[test]
    fn test_add_unit_full() {
        let conn = memory_db();
        let tags = vec!["rust".to_string(), "performance".to_string()];
        let id = add_unit_full(&conn, "tagged content", "fact", "test", &tags).unwrap();
        let unit = get_unit(&conn, &id).unwrap();
        assert_eq!(unit.content, "tagged content");
        assert_eq!(unit.unit_type, "fact");
        assert_eq!(unit.source, "test");
        assert_eq!(unit.tags, vec!["rust", "performance"]);
    }

    #[test]
    fn test_delete_inbox_item() {
        let conn = memory_db();
        drop_item(&conn, "to delete", "cli").unwrap();
        let items = list_inbox(&conn).unwrap();
        assert_eq!(items.len(), 1);
        delete_inbox_item(&conn, &items[0].id).unwrap();
        let items = list_inbox(&conn).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_delete_inbox_item_nonexistent() {
        let conn = memory_db();
        let result = delete_inbox_item(&conn, "nonexistent-id");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent-id"),
            "error should mention the id: {err}"
        );
    }

    #[test]
    fn test_fts_sync_after_add() {
        let conn = memory_db();
        add_unit(&conn, "unique searchable content", "fact", "test").unwrap();
        let units = search_units(&conn, "searchable", None).unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].content, "unique searchable content");
    }

    #[test]
    fn test_search_with_type_filter() {
        let conn = memory_db();
        add_unit(&conn, "deploy with cargo install", "procedure", "test").unwrap();
        add_unit(&conn, "cargo is a fast build tool", "fact", "test").unwrap();
        let units = search_units(&conn, "cargo", Some("procedure")).unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].unit_type, "procedure");
    }

    #[test]
    fn test_search_no_type_filter_returns_all() {
        let conn = memory_db();
        add_unit(&conn, "deploy with cargo install", "procedure", "test").unwrap();
        add_unit(&conn, "cargo is a fast build tool", "fact", "test").unwrap();
        let units = search_units(&conn, "cargo", None).unwrap();
        assert_eq!(units.len(), 2);
    }

    #[test]
    fn test_mark_unit() {
        let conn = memory_db();
        let id = add_unit(&conn, "test", "fact", "test").unwrap();
        let confidence = add_mark(&conn, &id, "helpful", 0.1).unwrap();
        assert!((confidence - 1.0).abs() < f64::EPSILON); // 1.0 + 0.1 clamped to 1.0

        let confidence = add_mark(&conn, &id, "wrong", -0.2).unwrap();
        assert!((confidence - 0.8).abs() < f64::EPSILON);

        // Verify mark was recorded
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM marks WHERE unit_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_mark_confidence_clamping() {
        let conn = memory_db();
        let id = add_unit(&conn, "test", "fact", "test").unwrap();

        // Drive confidence to 0
        for _ in 0..10 {
            add_mark(&conn, &id, "wrong", -0.2).unwrap();
        }
        let confidence: f64 = conn
            .query_row(
                "SELECT confidence FROM units WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(confidence >= 0.0);
        assert!((confidence - 0.0).abs() < f64::EPSILON);

        // Drive confidence back up
        for _ in 0..20 {
            add_mark(&conn, &id, "helpful", 0.1).unwrap();
        }
        let confidence: f64 = conn
            .query_row(
                "SELECT confidence FROM units WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(confidence <= 1.0);
        assert!((confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mark_nonexistent_unit() {
        let conn = memory_db();
        let result = add_mark(&conn, "nonexistent-id", "used", 0.05);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_mark_cascade_delete() {
        let conn = memory_db();
        let id = add_unit(&conn, "test", "fact", "test").unwrap();
        add_mark(&conn, &id, "used", 0.05).unwrap();

        // Delete the unit
        conn.execute("DELETE FROM units WHERE id = ?1", params![id])
            .unwrap();

        // Marks should be gone too
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM marks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_scan_empty_store() {
        let conn = memory_db();
        let result = scan(&conn, 90).unwrap();
        assert!(result.low_confidence.is_empty());
        assert!(result.negative_marks.is_empty());
        assert!(result.contradictions.is_empty());
        assert!(result.orphans.is_empty());
        assert!(result.stale.is_empty());
    }

    #[test]
    fn test_scan_low_confidence() {
        let conn = memory_db();
        let id = add_unit(&conn, "shaky fact", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET confidence = 0.5 WHERE id = ?1",
            params![id],
        )
        .unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.low_confidence.len(), 1);
        assert_eq!(result.low_confidence[0].id, id);
    }

    #[test]
    fn test_scan_low_confidence_boundary() {
        let conn = memory_db();
        let id = add_unit(&conn, "boundary fact", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET confidence = 0.6 WHERE id = ?1",
            params![id],
        )
        .unwrap();
        let result = scan(&conn, 90).unwrap();
        assert!(
            result.low_confidence.is_empty(),
            "unit at exactly 0.6 should NOT appear"
        );
    }

    #[test]
    fn test_scan_negative_marks() {
        let conn = memory_db();
        let id = add_unit(&conn, "wrong fact", "fact", "test").unwrap();
        add_mark(&conn, &id, "wrong", -0.2).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.negative_marks.len(), 1);
        assert_eq!(result.negative_marks[0].id, id);
    }

    #[test]
    fn test_scan_negative_marks_outdated() {
        let conn = memory_db();
        let id = add_unit(&conn, "outdated fact", "fact", "test").unwrap();
        add_mark(&conn, &id, "outdated", -0.1).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.negative_marks.len(), 1);
        assert_eq!(result.negative_marks[0].id, id);
    }

    #[test]
    fn test_scan_contradictions() {
        let conn = memory_db();
        let id_a = add_unit(&conn, "the sky is blue", "fact", "test").unwrap();
        let id_b = add_unit(&conn, "the sky is green", "fact", "test").unwrap();
        add_link(&conn, &id_a, &id_b, "contradicts").unwrap();
        let result = scan(&conn, 90).unwrap();
        // With UUIDs, from_id < to_id is string comparison; we just check one pair exists
        assert_eq!(result.contradictions.len(), 1);
    }

    #[test]
    fn test_scan_orphans() {
        let conn = memory_db();
        let id_a = add_unit(&conn, "connected a", "fact", "test").unwrap();
        let id_b = add_unit(&conn, "connected b", "fact", "test").unwrap();
        let id_c = add_unit(&conn, "lonely orphan", "fact", "test").unwrap();
        add_link(&conn, &id_a, &id_b, "related_to").unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.orphans.len(), 1);
        assert_eq!(result.orphans[0].id, id_c);
        assert_eq!(result.orphans[0].content, "lonely orphan");
    }

    #[test]
    fn test_scan_stale() {
        let conn = memory_db();
        let id = add_unit(&conn, "old fact", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
            params![id],
        )
        .unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.stale.len(), 1);
        assert_eq!(result.stale[0].id, id);
    }

    #[test]
    fn test_scan_stale_with_mark_not_stale() {
        let conn = memory_db();
        let id = add_unit(&conn, "old but marked", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
            params![id],
        )
        .unwrap();
        add_mark(&conn, &id, "used", 0.05).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert!(
            result.stale.is_empty(),
            "unit with a mark should NOT be stale"
        );
    }

    #[test]
    fn test_scan_stale_days_override() {
        let conn = memory_db();
        let id = add_unit(&conn, "91 day old", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
            params![id],
        )
        .unwrap();
        // With 100 days threshold, should not appear
        let result = scan(&conn, 100).unwrap();
        assert!(
            result.stale.is_empty(),
            "91-day unit with 100-day threshold should not be stale"
        );
        // With 90 days threshold, should appear
        let result = scan(&conn, 90).unwrap();
        assert_eq!(
            result.stale.len(),
            1,
            "91-day unit with 90-day threshold should be stale"
        );
    }

    #[test]
    fn test_scan_multi_section() {
        let conn = memory_db();
        // Create a unit that's low-confidence + has wrong mark + orphan + stale
        let id = add_unit(&conn, "troubled unit", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET confidence = 0.3 WHERE id = ?1",
            params![id],
        )
        .unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
            params![id],
        )
        .unwrap();
        // Add wrong mark but need to undo its effect on stale check:
        // marks table entry will make it NOT stale, so insert mark directly
        let mark_id = Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO marks (id, unit_id, kind) VALUES (?1, ?2, 'wrong')",
            params![mark_id, id],
        )
        .unwrap();

        let result = scan(&conn, 90).unwrap();
        // Should appear in low_confidence
        assert!(
            result.low_confidence.iter().any(|u| u.id == id),
            "should be low confidence"
        );
        // Should appear in negative_marks
        assert!(
            result.negative_marks.iter().any(|u| u.id == id),
            "should have negative marks"
        );
        // Should be orphan (no links)
        assert!(
            result.orphans.iter().any(|u| u.id == id),
            "should be orphan"
        );
        // Should NOT be stale (has a mark)
        assert!(
            !result.stale.iter().any(|u| u.id == id),
            "should not be stale because it has a mark"
        );
    }

    #[test]
    fn test_uuid_format() {
        let conn = memory_db();
        let id = add_unit(&conn, "test", "fact", "test").unwrap();
        // UUIDv7 should be a valid UUID string
        assert!(
            Uuid::parse_str(&id).is_ok(),
            "id should be valid UUID: {id}"
        );
    }

    #[test]
    fn test_user_version_set() {
        let conn = memory_db();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn test_auto_link_two_shared_tags() {
        let conn = memory_db();
        let a = add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into(), "perf".into()],
        )
        .unwrap();
        let b = add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "cli".into(), "gui".into()],
        )
        .unwrap();
        let count = auto_link(&conn, &b).unwrap();
        assert_eq!(count, 1);
        let links = get_links_from(&conn, &b).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].to_id, a);
        assert_eq!(links[0].relationship, "related_to");
    }

    #[test]
    fn test_auto_link_one_shared_tag_no_link() {
        let conn = memory_db();
        add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let b = add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "gui".into()],
        )
        .unwrap();
        let count = auto_link(&conn, &b).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_auto_link_no_tags() {
        let conn = memory_db();
        let a = add_unit(&conn, "unit A", "fact", "test").unwrap();
        let count = auto_link(&conn, &a).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_auto_link_skips_existing_link() {
        let conn = memory_db();
        let a = add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let b = add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        add_link(&conn, &b, &a, "part_of").unwrap();
        let count = auto_link(&conn, &b).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_auto_link_skips_self() {
        let conn = memory_db();
        let a = add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let count = auto_link(&conn, &a).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_auto_link_multiple_matches() {
        let conn = memory_db();
        add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let c = add_unit_full(
            &conn,
            "unit C",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let count = auto_link(&conn, &c).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_auto_link_case_insensitive() {
        let conn = memory_db();
        add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["Rust".into(), "CLI".into()],
        )
        .unwrap();
        let b = add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let count = auto_link(&conn, &b).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_auto_link_idempotent() {
        let conn = memory_db();
        add_unit_full(
            &conn,
            "unit A",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        let b = add_unit_full(
            &conn,
            "unit B",
            "fact",
            "test",
            &["rust".into(), "cli".into()],
        )
        .unwrap();
        assert_eq!(auto_link(&conn, &b).unwrap(), 1);
        assert_eq!(auto_link(&conn, &b).unwrap(), 0);
    }

    #[test]
    fn test_slugs_table_exists_fresh_install() {
        let conn = memory_db();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_slugs_table_columns() {
        let conn = memory_db();
        let mut stmt = conn.prepare("PRAGMA table_info(slugs)").unwrap();
        let cols: Vec<(String, String, i32, i32)> = stmt
            .query_map([], |row| {
                // (name, type, notnull, pk)
                Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(5)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(cols.len(), 3, "slugs should have 3 columns: {cols:?}");
        // SQLite quirk: PRAGMA table_info.notnull is 0 for TEXT PRIMARY KEY columns
        // (historic allow-NULL behavior), so slug's notnull flag is 0 even though
        // the PK constraint prevents NULLs in practice.
        assert_eq!(cols[0], ("slug".to_string(), "TEXT".to_string(), 0, 1));
        assert_eq!(cols[1], ("unit_id".to_string(), "TEXT".to_string(), 1, 0));
        assert_eq!(cols[2], ("created".to_string(), "TEXT".to_string(), 1, 0));
    }

    #[test]
    fn test_slugs_index_on_unit_id() {
        let conn = memory_db();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_slugs_unit' AND tbl_name='slugs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_slugs_fk_cascade_delete() {
        let conn = memory_db();
        let fk_on: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk_on, 1, "foreign_keys pragma must be ON");
        let id = add_unit(&conn, "parent", "fact", "test").unwrap();
        conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)",
            params!["hello", id],
        )
        .unwrap();
        conn.execute("DELETE FROM units WHERE id = ?1", params![id])
            .unwrap();
        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM slugs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_slugs_fk_rejects_unknown_unit_id() {
        let conn = memory_db();
        let result = conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)",
            params!["orphan", "no-such-unit"],
        );
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.to_uppercase().contains("FOREIGN KEY"),
            "expected FOREIGN KEY error, got: {msg}"
        );
    }

    #[test]
    fn test_slugs_slug_pk_rejects_duplicate() {
        let conn = memory_db();
        let id = add_unit(&conn, "unit", "fact", "test").unwrap();
        conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)",
            params!["dup", id],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)",
            params!["dup", id],
        );
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err()).to_uppercase();
        assert!(
            msg.contains("UNIQUE") || msg.contains("PRIMARY KEY"),
            "expected UNIQUE / PRIMARY KEY error, got: {msg}"
        );
    }

    #[test]
    fn test_slugs_unit_id_not_null() {
        let conn = memory_db();
        let result = conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES (?1, NULL)",
            params!["nullish"],
        );
        assert!(result.is_err());
        let msg = format!("{:?}", result.unwrap_err()).to_uppercase();
        assert!(
            msg.contains("NOT NULL"),
            "expected NOT NULL error, got: {msg}"
        );
    }

    /// Build an in-memory DB pinned at user_version=2 (the pre-slugs state).
    /// Mirrors the schema that `initialize()` produced prior to the v2→v3 migration.
    fn v2_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE units (
                id          TEXT PRIMARY KEY,
                content     TEXT NOT NULL,
                type        TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea','aspect')),
                source      TEXT NOT NULL DEFAULT 'inbox',
                confidence  REAL NOT NULL DEFAULT 1.0,
                verified    INTEGER NOT NULL DEFAULT 0,
                tags        TEXT NOT NULL DEFAULT '[]',
                conditions  TEXT NOT NULL DEFAULT '{}',
                created     TEXT NOT NULL DEFAULT (datetime('now')),
                updated     TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE links (
                from_id      TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                to_id        TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                relationship TEXT NOT NULL CHECK(relationship IN (
                                 'related_to','part_of','depends_on',
                                 'contradicts','supersedes','sourced_from')),
                PRIMARY KEY (from_id, to_id, relationship)
            );
            CREATE INDEX idx_links_to ON links(to_id);
            CREATE TABLE inbox (
                id      TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                source  TEXT NOT NULL DEFAULT 'cli',
                created TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE marks (
                id       TEXT PRIMARY KEY,
                unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                kind     TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
                created  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_marks_unit ON marks(unit_id);
            CREATE VIRTUAL TABLE units_fts USING fts5(uuid, content, type, tags, source);
            CREATE TRIGGER units_ai AFTER INSERT ON units BEGIN
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            CREATE TRIGGER units_ad AFTER DELETE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
            END;
            CREATE TRIGGER units_au AFTER UPDATE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            PRAGMA user_version = 2;",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_migrate_add_slugs_from_v2() {
        // Isolate the backup write that migrate_add_slugs performs via create_backup.
        let temp = std::env::temp_dir().join(format!(
            "simaris-migrate-v2-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&temp).unwrap();
        // SAFETY: no other unit test sets SIMARIS_HOME, and no other unit test
        // exercises create_backup / data_dir during its run.
        unsafe {
            std::env::set_var("SIMARIS_HOME", &temp);
        }
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    std::env::remove_var("SIMARIS_HOME");
                }
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(temp);

        let conn = v2_memory_db();

        // Sanity: no slugs yet, user_version=2
        let pre_version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pre_version, 2);
        let pre_slugs: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pre_slugs, 0);

        // Seed representative rows across every table.
        let u1 = add_unit(&conn, "seed one", "fact", "test").unwrap();
        let u2 = add_unit(&conn, "seed two", "idea", "test").unwrap();
        add_link(&conn, &u1, &u2, "related_to").unwrap();
        drop_item(&conn, "seed inbox", "cli").unwrap();
        add_mark(&conn, &u1, "used", 0.05).unwrap();

        // Run the migration.
        migrate_add_slugs(&conn).unwrap();

        // user_version advanced.
        let post_version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(post_version, 3);

        // slugs table + index present.
        let slugs_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(slugs_table, 1);
        let slugs_index: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_slugs_unit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(slugs_index, 1);

        // Seeded rows survived untouched.
        let units_count: i64 = conn
            .query_row("SELECT count(*) FROM units", [], |row| row.get(0))
            .unwrap();
        assert_eq!(units_count, 2);
        let links_count: i64 = conn
            .query_row("SELECT count(*) FROM links", [], |row| row.get(0))
            .unwrap();
        assert_eq!(links_count, 1);
        let inbox_count: i64 = conn
            .query_row("SELECT count(*) FROM inbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(inbox_count, 1);
        let marks_count: i64 = conn
            .query_row("SELECT count(*) FROM marks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(marks_count, 1);
    }

    mod slug {
        use super::*;

        // ---- validate_slug ----

        #[test]
        fn validate_accepts_valid_shapes() {
            assert!(validate_slug("abc").is_ok());
            assert!(validate_slug("feat-1").is_ok());
            assert!(validate_slug("user_name").is_ok());
            assert!(validate_slug("_internal").is_ok());
            assert!(validate_slug("a").is_ok());
            // 64-char string at the cap
            let s64: String = "a".repeat(64);
            assert!(validate_slug(&s64).is_ok());
        }

        #[test]
        fn validate_rejects_empty() {
            assert!(validate_slug("").is_err());
        }

        #[test]
        fn validate_rejects_uppercase() {
            assert!(validate_slug("Abc").is_err());
            assert!(validate_slug("FOO").is_err());
        }

        #[test]
        fn validate_rejects_bad_first_char() {
            assert!(validate_slug("1foo").is_err());
            assert!(validate_slug("-foo").is_err());
        }

        #[test]
        fn validate_rejects_disallowed_chars() {
            assert!(validate_slug("a b").is_err());
            assert!(validate_slug("foo!").is_err());
            assert!(validate_slug("foo.bar").is_err());
            assert!(validate_slug("foo/bar").is_err());
            assert!(validate_slug("café").is_err());
            assert!(validate_slug("foo🙂").is_err());
        }

        #[test]
        fn validate_rejects_uuid_shaped() {
            // canonical v7
            let v7 = Uuid::now_v7().to_string();
            assert_eq!(v7.len(), 36);
            assert!(validate_slug(&v7).is_err());
            // nil UUID
            assert!(validate_slug("00000000-0000-0000-0000-000000000000").is_err());
        }

        #[test]
        fn validate_rejects_over_cap() {
            let s65: String = "a".repeat(65);
            assert!(validate_slug(&s65).is_err());
        }

        // ---- set_slug ----

        #[test]
        fn set_inserts_row_with_created_datetime() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "alpha", &id).unwrap();
            let rows = list_slugs(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].slug, "alpha");
            assert_eq!(rows[0].unit_id, id);
            assert!(!rows[0].created.is_empty());
        }

        #[test]
        fn set_rejects_unknown_unit_id() {
            let conn = memory_db();
            let err = set_slug(&conn, "alpha", "no-such-unit").unwrap_err();
            let msg = format!("{err:?}");
            assert!(
                msg.contains("no-such-unit"),
                "error must surface the unit_id, got: {msg}"
            );
            assert!(
                msg.contains("not found"),
                "error must say 'not found', got: {msg}"
            );
        }

        #[test]
        fn set_rejects_invalid_slug() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            assert!(set_slug(&conn, "Bad!", &id).is_err());
            assert!(set_slug(&conn, "", &id).is_err());
        }

        #[test]
        fn set_move_preserves_created_timestamp() {
            let conn = memory_db();
            let a = add_unit(&conn, "a", "fact", "test").unwrap();
            let b = add_unit(&conn, "b", "fact", "test").unwrap();
            set_slug(&conn, "ptr", &a).unwrap();
            let before: String = conn
                .query_row("SELECT created FROM slugs WHERE slug = 'ptr'", [], |row| {
                    row.get(0)
                })
                .unwrap();

            // Move slug to a different unit. Same slug -> single row, created preserved.
            set_slug(&conn, "ptr", &b).unwrap();

            let after: String = conn
                .query_row("SELECT created FROM slugs WHERE slug = 'ptr'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let count: i64 = conn
                .query_row("SELECT count(*) FROM slugs", [], |row| row.get(0))
                .unwrap();
            let owner: String = conn
                .query_row("SELECT unit_id FROM slugs WHERE slug = 'ptr'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1);
            assert_eq!(owner, b);
            assert_eq!(before, after, "created must be preserved across move");
        }

        #[test]
        fn set_allows_multiple_slugs_per_unit() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "one", &id).unwrap();
            set_slug(&conn, "two", &id).unwrap();
            let rows = list_slugs(&conn).unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].slug, "one");
            assert_eq!(rows[1].slug, "two");
        }

        // ---- unset_slug ----

        #[test]
        fn unset_returns_true_on_hit() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "alpha", &id).unwrap();
            assert!(unset_slug(&conn, "alpha").unwrap());
            assert!(list_slugs(&conn).unwrap().is_empty());
        }

        #[test]
        fn unset_returns_false_on_miss() {
            let conn = memory_db();
            assert!(!unset_slug(&conn, "nope").unwrap());
        }

        #[test]
        fn unset_tolerates_invalid_input() {
            let conn = memory_db();
            assert!(!unset_slug(&conn, "Bad!!!").unwrap());
        }

        // ---- list_slugs ----

        #[test]
        fn list_empty_db() {
            let conn = memory_db();
            assert_eq!(list_slugs(&conn).unwrap(), Vec::<SlugRow>::new());
        }

        #[test]
        fn list_orders_ascending() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "z", &id).unwrap();
            set_slug(&conn, "a", &id).unwrap();
            set_slug(&conn, "m", &id).unwrap();
            let rows = list_slugs(&conn).unwrap();
            let slugs: Vec<&str> = rows.iter().map(|r| r.slug.as_str()).collect();
            assert_eq!(slugs, vec!["a", "m", "z"]);
        }

        // ---- get_slugs_for_unit ----

        #[test]
        fn get_for_unit_zero() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            assert_eq!(
                get_slugs_for_unit(&conn, &id).unwrap(),
                Vec::<String>::new()
            );
        }

        #[test]
        fn get_for_unit_one() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "alpha", &id).unwrap();
            assert_eq!(get_slugs_for_unit(&conn, &id).unwrap(), vec!["alpha"]);
        }

        #[test]
        fn get_for_unit_many_sorted() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "z", &id).unwrap();
            set_slug(&conn, "a", &id).unwrap();
            set_slug(&conn, "m", &id).unwrap();
            assert_eq!(get_slugs_for_unit(&conn, &id).unwrap(), vec!["a", "m", "z"]);
        }

        #[test]
        fn get_for_unknown_unit_empty() {
            let conn = memory_db();
            assert_eq!(
                get_slugs_for_unit(&conn, "no-such-unit").unwrap(),
                Vec::<String>::new()
            );
        }

        // ---- FK cascade ----

        #[test]
        fn fk_cascade_deletes_slugs_with_unit() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "alpha", &id).unwrap();
            set_slug(&conn, "beta", &id).unwrap();
            conn.execute("DELETE FROM units WHERE id = ?1", params![id])
                .unwrap();
            assert!(list_slugs(&conn).unwrap().is_empty());
        }

        // ---- resolve_id ----

        #[test]
        fn resolve_returns_existing_unit_id() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            assert_eq!(resolve_id(&conn, &id).unwrap(), id);
        }

        #[test]
        fn resolve_returns_unit_id_for_slug() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "foo", &id).unwrap();
            assert_eq!(resolve_id(&conn, "foo").unwrap(), id);
        }

        #[test]
        fn resolve_unknown_bails_with_input() {
            let conn = memory_db();
            let err = resolve_id(&conn, "ghost").unwrap_err();
            let msg = format!("{err:?}");
            assert!(
                msg.contains("ghost"),
                "error must include the input verbatim, got: {msg}"
            );
        }

        #[test]
        fn resolve_empty_string_bails() {
            let conn = memory_db();
            assert!(resolve_id(&conn, "").is_err());
        }

        #[test]
        fn resolve_is_case_sensitive() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "foo", &id).unwrap();
            assert!(resolve_id(&conn, "FOO").is_err());
        }

        #[test]
        fn resolve_unit_id_wins_collision() {
            // Force a slug whose name equals an existing unit UUID, bypassing
            // validate_slug via raw INSERT, then verify resolve returns the unit id.
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            let other = add_unit(&conn, "other", "fact", "test").unwrap();
            conn.execute(
                "INSERT INTO slugs (slug, unit_id) VALUES (?1, ?2)",
                params![id, other],
            )
            .unwrap();
            // resolve(id) must hit units first and return id (not other)
            assert_eq!(resolve_id(&conn, &id).unwrap(), id);
        }

        #[test]
        fn resolve_after_unset_bails() {
            let conn = memory_db();
            let id = add_unit(&conn, "u", "fact", "test").unwrap();
            set_slug(&conn, "foo", &id).unwrap();
            assert!(unset_slug(&conn, "foo").unwrap());
            assert!(resolve_id(&conn, "foo").is_err());
        }
    }
}
