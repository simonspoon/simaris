use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SIMARIS_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".simaris")
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
        );",
    )?;
    Ok(())
}

pub fn add_unit(conn: &Connection, content: &str, unit_type: &str, source: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO units (content, type, source) VALUES (?1, ?2, ?3)",
        params![content, unit_type, source],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_unit(conn: &Connection, id: i64) -> Result<Unit> {
    let unit = conn.query_row(
        "SELECT id, content, type, source, confidence, verified, tags, conditions, created, updated
         FROM units WHERE id = ?1",
        params![id],
        |row| {
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
        },
    ).context(format!("Unit {id} not found"))?;
    Ok(unit)
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
}
