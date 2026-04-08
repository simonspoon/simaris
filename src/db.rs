use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Unit {
    pub id: i64,
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
    pub from_id: i64,
    pub to_id: i64,
    pub relationship: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: i64,
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
    pub from_id: i64,
    pub from_content: String,
    pub to_id: i64,
    pub to_content: String,
}

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
    initialize(&conn)?;
    Ok(conn)
}

fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS units (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            content     TEXT NOT NULL,
            type        TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea')),
            source      TEXT NOT NULL DEFAULT 'inbox',
            confidence  REAL NOT NULL DEFAULT 1.0,
            verified    INTEGER NOT NULL DEFAULT 0,
            tags        TEXT NOT NULL DEFAULT '[]',
            conditions  TEXT NOT NULL DEFAULT '{}',
            created     TEXT NOT NULL DEFAULT (datetime('now')),
            updated     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS links (
            from_id      INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            to_id        INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            relationship TEXT NOT NULL CHECK(relationship IN (
                             'related_to','part_of','depends_on',
                             'contradicts','supersedes','sourced_from')),
            PRIMARY KEY (from_id, to_id, relationship)
        );

        CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_id);

        CREATE TABLE IF NOT EXISTS inbox (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            source  TEXT NOT NULL DEFAULT 'cli',
            created TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS marks (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            unit_id  INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
            kind     TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
            created  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_marks_unit ON marks(unit_id);",
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
                content, type, tags, source,
                content=units, content_rowid=id
            );

            CREATE TRIGGER units_ai AFTER INSERT ON units BEGIN
                INSERT INTO units_fts(rowid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;

            CREATE TRIGGER units_ad AFTER DELETE ON units BEGIN
                INSERT INTO units_fts(units_fts, rowid, content, type, tags, source)
                VALUES ('delete', old.id, old.content, old.type, old.tags, old.source);
            END;

            CREATE TRIGGER units_au AFTER UPDATE ON units BEGIN
                INSERT INTO units_fts(units_fts, rowid, content, type, tags, source)
                VALUES ('delete', old.id, old.content, old.type, old.tags, old.source);
                INSERT INTO units_fts(rowid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            ",
        )?;
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

pub fn add_unit(conn: &Connection, content: &str, unit_type: &str, source: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO units (content, type, source) VALUES (?1, ?2, ?3)",
        params![content, unit_type, source],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_unit(conn: &Connection, id: i64) -> Result<Unit> {
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

pub fn search_units(conn: &Connection, query: &str) -> Result<Vec<Unit>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.content, u.type, u.source, u.confidence, u.verified, u.tags, u.conditions, u.created, u.updated
         FROM units_fts
         JOIN units u ON u.id = units_fts.rowid
         WHERE units_fts MATCH ?1
         ORDER BY rank",
    )?;
    let units = stmt
        .query_map(params![query], row_to_unit)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(units)
}

pub fn get_links_from(conn: &Connection, id: i64) -> Result<Vec<Link>> {
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

pub fn get_links_to(conn: &Connection, id: i64) -> Result<Vec<Link>> {
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
pub fn get_linked_unit_ids(conn: &Connection, id: i64) -> Result<Vec<(i64, String, String)>> {
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

pub fn add_link(conn: &Connection, from_id: i64, to_id: i64, relationship: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
        params![from_id, to_id, relationship],
    )
    .context(format!(
        "Failed to create link {from_id} -> {to_id} ({relationship})"
    ))?;
    Ok(())
}

#[allow(dead_code)]
pub fn add_unit_full(
    conn: &Connection,
    content: &str,
    unit_type: &str,
    source: &str,
    tags: &[String],
) -> Result<i64> {
    let tags_json = serde_json::to_string(tags)?;
    conn.execute(
        "INSERT INTO units (content, type, source, tags) VALUES (?1, ?2, ?3, ?4)",
        params![content, unit_type, source, tags_json],
    )?;
    Ok(conn.last_insert_rowid())
}

#[allow(dead_code)]
pub fn delete_inbox_item(conn: &Connection, id: i64) -> Result<()> {
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
    inbox_id: i64,
    content: &str,
    unit_type: &str,
    source: &str,
    tags: &[String],
) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let tags_json = serde_json::to_string(tags)?;
    tx.execute(
        "INSERT INTO units (content, type, source, tags) VALUES (?1, ?2, ?3, ?4)",
        params![content, unit_type, source, tags_json],
    )?;
    let unit_id = tx.last_insert_rowid();
    tx.execute("DELETE FROM inbox WHERE id = ?1", params![inbox_id])?;
    tx.commit()?;
    Ok(unit_id)
}

/// Atomically create multiple units from an inbox item, link children to overview, delete inbox item.
pub fn digest_inbox_item_multi(
    conn: &Connection,
    inbox_id: i64,
    units: &[crate::digest::DigestUnit],
    source: &str,
) -> Result<Vec<i64>> {
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::new();
    let mut overview_id: Option<i64> = None;

    for unit in units {
        let tags_json = serde_json::to_string(&unit.tags)?;
        tx.execute(
            "INSERT INTO units (content, type, source, tags) VALUES (?1, ?2, ?3, ?4)",
            params![unit.content, unit.unit_type, source, tags_json],
        )?;
        let id = tx.last_insert_rowid();
        ids.push(id);

        if unit.is_overview {
            overview_id = Some(id);
        }
    }

    // Link non-overview units to the overview via part_of
    if let Some(ov_id) = overview_id {
        for (id, unit) in ids.iter().zip(units.iter()) {
            if !unit.is_overview {
                tx.execute(
                    "INSERT INTO links (from_id, to_id, relationship) VALUES (?1, ?2, ?3)",
                    params![id, ov_id, "part_of"],
                )?;
            }
        }
    }

    tx.execute("DELETE FROM inbox WHERE id = ?1", params![inbox_id])?;
    tx.commit()?;
    Ok(ids)
}

pub fn add_mark(conn: &Connection, unit_id: i64, kind: &str, delta: f64) -> Result<f64> {
    // Verify unit exists
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM units WHERE id = ?)",
        [unit_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Unit {} not found", unit_id);
    }

    // Insert mark
    conn.execute(
        "INSERT INTO marks (unit_id, kind) VALUES (?, ?)",
        params![unit_id, kind],
    )?;

    // Update confidence with clamping
    conn.execute(
        "UPDATE units SET confidence = MAX(0.0, MIN(1.0, confidence + ?)) WHERE id = ?",
        params![delta, unit_id],
    )?;

    // Return new confidence
    let confidence: f64 = conn.query_row(
        "SELECT confidence FROM units WHERE id = ?",
        [unit_id],
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

pub fn drop_item(conn: &Connection, content: &str, source: &str) -> Result<i64> {
    if content.trim().is_empty() {
        anyhow::bail!("Content cannot be empty");
    }
    conn.execute(
        "INSERT INTO inbox (content, source) VALUES (?1, ?2)",
        params![content, source],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_inbox_item(conn: &Connection, id: i64) -> Result<InboxItem> {
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

pub fn promote_item(conn: &Connection, inbox_id: i64, unit_type: &str) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let item = get_inbox_item(&tx, inbox_id)?;
    tx.execute(
        "INSERT INTO units (content, type, source) VALUES (?1, ?2, ?3)",
        params![item.content, unit_type, item.source],
    )?;
    let unit_id = tx.last_insert_rowid();
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
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('units','links','inbox')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_add_unit() {
        let conn = memory_db();
        let id = add_unit(&conn, "test content", "fact", "inbox").unwrap();
        assert_eq!(id, 1);
        let unit = get_unit(&conn, id).unwrap();
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
        add_unit(&conn, "unit a", "fact", "inbox").unwrap();
        add_unit(&conn, "unit b", "fact", "inbox").unwrap();
        add_link(&conn, 1, 2, "related_to").unwrap();
        let result = add_link(&conn, 1, 2, "related_to");
        assert!(result.is_err());
    }

    #[test]
    fn test_cascade_delete() {
        let conn = memory_db();
        add_unit(&conn, "unit a", "fact", "inbox").unwrap();
        add_unit(&conn, "unit b", "fact", "inbox").unwrap();
        add_link(&conn, 1, 2, "related_to").unwrap();

        conn.execute("DELETE FROM units WHERE id = 1", []).unwrap();

        let links = get_links_to(&conn, 2).unwrap();
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
        assert_eq!(id, 1);
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
        drop_item(&conn, "promote me", "cli").unwrap();
        let unit_id = promote_item(&conn, 1, "fact").unwrap();
        let unit = get_unit(&conn, unit_id).unwrap();
        assert_eq!(unit.content, "promote me");
        assert_eq!(unit.unit_type, "fact");
        assert_eq!(unit.source, "cli");
    }

    #[test]
    fn test_promote_deletes_inbox_item() {
        let conn = memory_db();
        drop_item(&conn, "ephemeral", "cli").unwrap();
        promote_item(&conn, 1, "idea").unwrap();
        let items = list_inbox(&conn).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_promote_nonexistent_fails() {
        let conn = memory_db();
        let result = promote_item(&conn, 999, "fact");
        assert!(result.is_err());
    }

    #[test]
    fn test_promote_preserves_source() {
        let conn = memory_db();
        drop_item(&conn, "phone thought", "phone").unwrap();
        let unit_id = promote_item(&conn, 1, "lesson").unwrap();
        let unit = get_unit(&conn, unit_id).unwrap();
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
        let units = search_units(&conn, "caching").unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].content, "caching improves performance");
    }

    #[test]
    fn test_search_no_match() {
        let conn = memory_db();
        add_unit(&conn, "some content here", "fact", "test").unwrap();
        let units = search_units(&conn, "nonexistent").unwrap();
        assert!(units.is_empty());
    }

    #[test]
    fn test_add_unit_full() {
        let conn = memory_db();
        let tags = vec!["rust".to_string(), "performance".to_string()];
        let id = add_unit_full(&conn, "tagged content", "fact", "test", &tags).unwrap();
        let unit = get_unit(&conn, id).unwrap();
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
        delete_inbox_item(&conn, items[0].id).unwrap();
        let items = list_inbox(&conn).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_delete_inbox_item_nonexistent() {
        let conn = memory_db();
        let result = delete_inbox_item(&conn, 999);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("999"), "error should mention the id: {err}");
    }

    #[test]
    fn test_fts_sync_after_add() {
        let conn = memory_db();
        add_unit(&conn, "unique searchable content", "fact", "test").unwrap();
        let units = search_units(&conn, "searchable").unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].content, "unique searchable content");
    }

    #[test]
    fn test_mark_unit() {
        let conn = memory_db();
        add_unit(&conn, "test", "fact", "test").unwrap();
        let confidence = add_mark(&conn, 1, "helpful", 0.1).unwrap();
        assert!((confidence - 1.0).abs() < f64::EPSILON); // 1.0 + 0.1 clamped to 1.0

        let confidence = add_mark(&conn, 1, "wrong", -0.2).unwrap();
        assert!((confidence - 0.8).abs() < f64::EPSILON);

        // Verify mark was recorded
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM marks WHERE unit_id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_mark_confidence_clamping() {
        let conn = memory_db();
        add_unit(&conn, "test", "fact", "test").unwrap();

        // Drive confidence to 0
        for _ in 0..10 {
            add_mark(&conn, 1, "wrong", -0.2).unwrap();
        }
        let confidence: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(confidence >= 0.0);
        assert!((confidence - 0.0).abs() < f64::EPSILON);

        // Drive confidence back up
        for _ in 0..20 {
            add_mark(&conn, 1, "helpful", 0.1).unwrap();
        }
        let confidence: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(confidence <= 1.0);
        assert!((confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mark_nonexistent_unit() {
        let conn = memory_db();
        let result = add_mark(&conn, 999, "used", 0.05);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_mark_cascade_delete() {
        let conn = memory_db();
        add_unit(&conn, "test", "fact", "test").unwrap();
        add_mark(&conn, 1, "used", 0.05).unwrap();

        // Delete the unit
        conn.execute("DELETE FROM units WHERE id = 1", []).unwrap();

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
        add_unit(&conn, "shaky fact", "fact", "test").unwrap();
        conn.execute("UPDATE units SET confidence = 0.5 WHERE id = 1", [])
            .unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.low_confidence.len(), 1);
        assert_eq!(result.low_confidence[0].id, 1);
    }

    #[test]
    fn test_scan_low_confidence_boundary() {
        let conn = memory_db();
        add_unit(&conn, "boundary fact", "fact", "test").unwrap();
        conn.execute("UPDATE units SET confidence = 0.6 WHERE id = 1", [])
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
        add_unit(&conn, "wrong fact", "fact", "test").unwrap();
        add_mark(&conn, 1, "wrong", -0.2).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.negative_marks.len(), 1);
        assert_eq!(result.negative_marks[0].id, 1);
    }

    #[test]
    fn test_scan_negative_marks_outdated() {
        let conn = memory_db();
        add_unit(&conn, "outdated fact", "fact", "test").unwrap();
        add_mark(&conn, 1, "outdated", -0.1).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.negative_marks.len(), 1);
        assert_eq!(result.negative_marks[0].id, 1);
    }

    #[test]
    fn test_scan_contradictions() {
        let conn = memory_db();
        add_unit(&conn, "the sky is blue", "fact", "test").unwrap();
        add_unit(&conn, "the sky is green", "fact", "test").unwrap();
        add_link(&conn, 1, 2, "contradicts").unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.contradictions.len(), 1);
        assert_eq!(result.contradictions[0].from_id, 1);
        assert_eq!(result.contradictions[0].to_id, 2);
    }

    #[test]
    fn test_scan_orphans() {
        let conn = memory_db();
        add_unit(&conn, "connected a", "fact", "test").unwrap();
        add_unit(&conn, "connected b", "fact", "test").unwrap();
        add_unit(&conn, "lonely orphan", "fact", "test").unwrap();
        add_link(&conn, 1, 2, "related_to").unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.orphans.len(), 1);
        assert_eq!(result.orphans[0].id, 3);
        assert_eq!(result.orphans[0].content, "lonely orphan");
    }

    #[test]
    fn test_scan_stale() {
        let conn = memory_db();
        add_unit(&conn, "old fact", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = 1",
            [],
        )
        .unwrap();
        let result = scan(&conn, 90).unwrap();
        assert_eq!(result.stale.len(), 1);
        assert_eq!(result.stale[0].id, 1);
    }

    #[test]
    fn test_scan_stale_with_mark_not_stale() {
        let conn = memory_db();
        add_unit(&conn, "old but marked", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = 1",
            [],
        )
        .unwrap();
        add_mark(&conn, 1, "used", 0.05).unwrap();
        let result = scan(&conn, 90).unwrap();
        assert!(
            result.stale.is_empty(),
            "unit with a mark should NOT be stale"
        );
    }

    #[test]
    fn test_scan_stale_days_override() {
        let conn = memory_db();
        add_unit(&conn, "91 day old", "fact", "test").unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = 1",
            [],
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
        add_unit(&conn, "troubled unit", "fact", "test").unwrap();
        conn.execute("UPDATE units SET confidence = 0.3 WHERE id = 1", [])
            .unwrap();
        conn.execute(
            "UPDATE units SET created = datetime('now', '-91 days') WHERE id = 1",
            [],
        )
        .unwrap();
        // Add wrong mark but need to undo its effect on stale check:
        // marks table entry will make it NOT stale, so insert mark directly
        conn.execute("INSERT INTO marks (unit_id, kind) VALUES (1, 'wrong')", [])
            .unwrap();

        let result = scan(&conn, 90).unwrap();
        // Should appear in low_confidence
        assert!(
            result.low_confidence.iter().any(|u| u.id == 1),
            "should be low confidence"
        );
        // Should appear in negative_marks
        assert!(
            result.negative_marks.iter().any(|u| u.id == 1),
            "should have negative marks"
        );
        // Should be orphan (no links)
        assert!(result.orphans.iter().any(|u| u.id == 1), "should be orphan");
        // Should NOT be stale (has a mark)
        assert!(
            !result.stale.iter().any(|u| u.id == 1),
            "should not be stale because it has a mark"
        );
    }
}
