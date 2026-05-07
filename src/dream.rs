//! Dream subsystem — periodic corpus hygiene operations.
//!
//! `decay` applies Ebbinghaus-style forgetting to unit confidences:
//!
//! ```text
//! new_confidence = old_confidence * 0.5 ^ (days_since_activity / half_life_days)
//! ```
//!
//! `days_since_activity` uses the latest of `units.updated` and the latest
//! mark on the unit, so recent edits or marks freeze decay.
//!
//! After decay, a unit is archived (`archived = 1`) when **all** hold:
//! - `confidence < 0.1`
//! - no other unit links to it via `part_of` (no children depend on it)
//! - it has no slug (not pinned)
//!
//! Idempotent: writing decay also bumps `updated` so a re-run within the
//! same minute computes `days_since = 0`, factor `1.0`, and short-circuits.
//!
//! M9 pick #5 per `lotus-m9-intel-picks-2026-05-06`.
use anyhow::Result;
use rusqlite::{Connection, params};

/// Default Ebbinghaus half-life (days). Tunable via `--half-life-days`.
pub const DEFAULT_HALF_LIFE_DAYS: f64 = 180.0;

/// Confidence threshold for archive eligibility.
const ARCHIVE_THRESHOLD: f64 = 0.1;

/// Minimum confidence delta to bother writing back. Avoids spurious
/// `updated` bumps when the formula yields a no-op factor (≈1.0).
const WRITE_EPSILON: f64 = 1e-6;

/// Cap on the archived-sample list returned to callers (for printing).
const ARCHIVED_SAMPLE_CAP: usize = 10;

#[derive(Debug)]
pub struct ArchivedSample {
    pub id: String,
    pub headline: String,
}

#[derive(Debug)]
pub struct DecayResult {
    pub half_life_days: f64,
    pub dry_run: bool,
    /// Number of units whose confidence changed (or would change).
    pub decayed: usize,
    /// Number of units archived (or that would be).
    pub archived: usize,
    /// Up to 10 representative archive targets (id + first-line headline).
    pub archived_sample: Vec<ArchivedSample>,
}

/// Apply Ebbinghaus decay across all live (non-archived) units.
///
/// `half_life_days` must be > 0. `dry_run = true` computes the result
/// without writing.
pub fn run_decay(conn: &Connection, half_life_days: f64, dry_run: bool) -> Result<DecayResult> {
    if half_life_days <= 0.0 {
        anyhow::bail!("--half-life-days must be > 0 (got {half_life_days})");
    }

    // Pull every live unit with its activity timestamp + days-since.
    // Activity = max(units.updated, latest mark.created).
    let mut stmt = conn.prepare(
        "SELECT u.id,
                u.confidence,
                u.content,
                COALESCE(
                    (SELECT MAX(m.created) FROM marks m WHERE m.unit_id = u.id),
                    u.updated
                ) AS last_mark,
                u.updated
         FROM units u
         WHERE u.archived = 0",
    )?;

    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let confidence: f64 = row.get(1)?;
        let content: String = row.get(2)?;
        let last_mark: String = row.get(3)?;
        let updated: String = row.get(4)?;
        Ok((id, confidence, content, last_mark, updated))
    })?;

    let mut to_decay: Vec<(String, f64, String)> = Vec::new(); // (id, new_conf, content)
    for row in rows {
        let (id, old_conf, content, last_mark, updated) = row?;
        let activity_ts = if last_mark > updated {
            last_mark
        } else {
            updated
        };
        let days_since = days_between(conn, &activity_ts)?;
        if days_since <= 0.0 {
            continue;
        }
        let factor = 0.5_f64.powf(days_since / half_life_days);
        let new_conf = (old_conf * factor).clamp(0.0, 1.0);
        if (old_conf - new_conf).abs() < WRITE_EPSILON {
            continue;
        }
        to_decay.push((id, new_conf, content));
    }

    let mut archived_count = 0usize;
    let mut archived_sample: Vec<ArchivedSample> = Vec::new();

    let mut child_check =
        conn.prepare("SELECT COUNT(*) FROM links WHERE to_id = ?1 AND relationship = 'part_of'")?;
    let mut slug_check = conn.prepare("SELECT COUNT(*) FROM slugs WHERE unit_id = ?1")?;

    let tx = if dry_run {
        None
    } else {
        Some(conn.unchecked_transaction()?)
    };

    for (id, new_conf, content) in &to_decay {
        if !dry_run {
            // Bump `updated` so the next run sees days_since ≈ 0 → idempotent.
            conn.execute(
                "UPDATE units SET confidence = ?1, updated = datetime('now') WHERE id = ?2",
                params![new_conf, id],
            )?;
        }

        if *new_conf < ARCHIVE_THRESHOLD {
            let child_count: i64 = child_check.query_row(params![id], |row| row.get(0))?;
            if child_count > 0 {
                continue;
            }
            let slug_count: i64 = slug_check.query_row(params![id], |row| row.get(0))?;
            if slug_count > 0 {
                continue;
            }
            if !dry_run {
                conn.execute(
                    "UPDATE units SET archived = 1, updated = datetime('now') WHERE id = ?1",
                    params![id],
                )?;
            }
            archived_count += 1;
            if archived_sample.len() < ARCHIVED_SAMPLE_CAP {
                archived_sample.push(ArchivedSample {
                    id: id.clone(),
                    headline: crate::display::derive_headline(content),
                });
            }
        }
    }

    if let Some(tx) = tx {
        tx.commit()?;
    }

    Ok(DecayResult {
        half_life_days,
        dry_run,
        decayed: to_decay.len(),
        archived: archived_count,
        archived_sample,
    })
}

/// Days between `ts` (SQLite datetime string) and "now". Negative or zero
/// when `ts` is in the future or essentially equal to now.
fn days_between(conn: &Connection, ts: &str) -> Result<f64> {
    let days: f64 = conn.query_row(
        "SELECT julianday('now') - julianday(?1)",
        params![ts],
        |row| row.get(0),
    )?;
    Ok(days)
}

/// Render a `DecayResult` to stdout (text or JSON).
pub fn print_decay(result: &DecayResult, json: bool) {
    if json {
        let sample: Vec<_> = result
            .archived_sample
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "headline": s.headline,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "decayed": result.decayed,
                "archived": result.archived,
                "half_life_days": result.half_life_days,
                "dry_run": result.dry_run,
                "archived_sample": sample,
            })
        );
        return;
    }

    let prefix = if result.dry_run { "[dry-run] " } else { "" };
    println!(
        "{prefix}dream decay (half-life {:.1}d): {} units decayed, {} units archived",
        result.half_life_days, result.decayed, result.archived
    );
    if !result.archived_sample.is_empty() {
        println!("archived sample (top {}):", result.archived_sample.len());
        for s in &result.archived_sample {
            let head = if s.headline.is_empty() {
                "(no headline)"
            } else {
                &s.headline
            };
            println!("  {} {head}", &s.id[..8.min(s.id.len())]);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — synthetic in-memory schema; we cannot import db::initialize because
// it depends on data_dir(). Recreate the minimal schema we need for decay.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE units (
                id          TEXT PRIMARY KEY,
                content     TEXT NOT NULL,
                type        TEXT NOT NULL DEFAULT 'fact',
                source      TEXT NOT NULL DEFAULT 'inbox',
                confidence  REAL NOT NULL DEFAULT 1.0,
                verified    INTEGER NOT NULL DEFAULT 0,
                tags        TEXT NOT NULL DEFAULT '[]',
                conditions  TEXT NOT NULL DEFAULT '{}',
                created     TEXT NOT NULL DEFAULT (datetime('now')),
                updated     TEXT NOT NULL DEFAULT (datetime('now')),
                archived    INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE links (
                from_id      TEXT NOT NULL,
                to_id        TEXT NOT NULL,
                relationship TEXT NOT NULL,
                PRIMARY KEY (from_id, to_id, relationship)
             );
             CREATE TABLE marks (
                id       TEXT PRIMARY KEY,
                unit_id  TEXT NOT NULL,
                kind     TEXT NOT NULL,
                created  TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE slugs (
                slug     TEXT PRIMARY KEY,
                unit_id  TEXT NOT NULL,
                created  TEXT NOT NULL DEFAULT (datetime('now'))
             );",
        )
        .unwrap();
        conn
    }

    /// Insert a unit with a chosen `updated` offset (negative days from now).
    fn insert_unit(conn: &Connection, id: &str, content: &str, confidence: f64, days_old: f64) {
        let stamp = format!("datetime('now', '-{days_old} days')");
        conn.execute(
            &format!(
                "INSERT INTO units (id, content, confidence, created, updated)
                 VALUES (?1, ?2, ?3, {stamp}, {stamp})"
            ),
            params![id, content, confidence],
        )
        .unwrap();
    }

    #[test]
    fn formula_matches_expected_decay() {
        let conn = open_test_db();
        // 180-day-old unit at 1.0 → exactly half-life → 0.5
        insert_unit(&conn, "u1", "old unit", 1.0, 180.0);
        // 360-day-old unit at 1.0 → two half-lives → 0.25
        insert_unit(&conn, "u2", "older unit", 1.0, 360.0);
        // brand new unit → no decay (skipped)
        insert_unit(&conn, "u3", "fresh", 1.0, 0.0);

        let result = run_decay(&conn, 180.0, false).unwrap();
        assert_eq!(result.decayed, 2, "u1 + u2 should decay, u3 fresh");

        let c1: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let c2: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let c3: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u3'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert!(
            (c1 - 0.5).abs() < 0.01,
            "u1 at 180d should be ~0.5, got {c1}"
        );
        assert!(
            (c2 - 0.25).abs() < 0.01,
            "u2 at 360d should be ~0.25, got {c2}"
        );
        assert!((c3 - 1.0).abs() < 1e-6, "u3 should be untouched, got {c3}");
    }

    #[test]
    fn pinned_units_with_slugs_are_not_archived() {
        let conn = open_test_db();
        // Old + low-confidence unit, but pinned by a slug.
        insert_unit(&conn, "u1", "pinned old", 0.05, 720.0);
        conn.execute(
            "INSERT INTO slugs (slug, unit_id) VALUES ('pinned', 'u1')",
            [],
        )
        .unwrap();
        // Same shape, no slug — should be archived.
        insert_unit(&conn, "u2", "old unpinned", 0.05, 720.0);

        let result = run_decay(&conn, 180.0, false).unwrap();
        assert_eq!(result.archived, 1, "only u2 should archive");

        let archived_u1: i32 = conn
            .query_row("SELECT archived FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let archived_u2: i32 = conn
            .query_row("SELECT archived FROM units WHERE id = 'u2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(archived_u1, 0, "u1 (pinned) must remain live");
        assert_eq!(archived_u2, 1, "u2 (unpinned) must archive");
    }

    #[test]
    fn part_of_parents_are_not_archived() {
        let conn = open_test_db();
        // u_parent is referenced by u_child via part_of — should survive.
        insert_unit(&conn, "u_parent", "parent old", 0.05, 720.0);
        insert_unit(&conn, "u_child", "child fresh", 1.0, 0.0);
        conn.execute(
            "INSERT INTO links (from_id, to_id, relationship)
             VALUES ('u_child', 'u_parent', 'part_of')",
            [],
        )
        .unwrap();

        let result = run_decay(&conn, 180.0, false).unwrap();
        assert_eq!(result.archived, 0, "parent must not archive");

        let archived_parent: i32 = conn
            .query_row(
                "SELECT archived FROM units WHERE id = 'u_parent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(archived_parent, 0);
    }

    #[test]
    fn recent_marks_freeze_decay() {
        let conn = open_test_db();
        // Updated 720 days ago, but a mark dropped 1 day ago should freeze decay.
        insert_unit(&conn, "u1", "marked recently", 1.0, 720.0);
        conn.execute(
            "INSERT INTO marks (id, unit_id, kind, created)
             VALUES ('m1', 'u1', 'used', datetime('now', '-1 days'))",
            [],
        )
        .unwrap();

        let result = run_decay(&conn, 180.0, false).unwrap();
        // 1-day-old activity, half-life 180d → factor 0.5^(1/180) ≈ 0.9962.
        // Confidence ≈ 0.9962, still counts as "decayed" (delta > epsilon).
        let c1: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            c1 > 0.99,
            "recent mark should hold decay near 1.0, got {c1}"
        );
        assert_eq!(result.archived, 0, "high confidence not archive-eligible");
    }

    #[test]
    fn idempotent_within_same_minute() {
        let conn = open_test_db();
        insert_unit(&conn, "u1", "decayable", 1.0, 360.0);

        let r1 = run_decay(&conn, 180.0, false).unwrap();
        let c1: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();

        // Second invocation: must be a no-op — `updated` was bumped by r1.
        let r2 = run_decay(&conn, 180.0, false).unwrap();
        let c2: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert!(r1.decayed >= 1, "first run should decay");
        assert_eq!(r2.decayed, 0, "second run within the same minute must skip");
        assert!(
            (c1 - c2).abs() < 1e-9,
            "confidence must not change on rerun"
        );
    }

    #[test]
    fn dry_run_does_not_write() {
        let conn = open_test_db();
        insert_unit(&conn, "u1", "decayable", 1.0, 360.0);
        insert_unit(&conn, "u2", "archive bait", 0.05, 720.0);

        let r = run_decay(&conn, 180.0, true).unwrap();
        assert!(r.decayed >= 2);
        assert_eq!(r.archived, 1, "u2 would archive in non-dry mode");

        let c1: f64 = conn
            .query_row("SELECT confidence FROM units WHERE id = 'u1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let archived_u2: i32 = conn
            .query_row("SELECT archived FROM units WHERE id = 'u2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!((c1 - 1.0).abs() < 1e-9, "dry-run must not write confidence");
        assert_eq!(archived_u2, 0, "dry-run must not flip archived");
    }

    #[test]
    fn invalid_half_life_rejected() {
        let conn = open_test_db();
        let err = run_decay(&conn, 0.0, true).unwrap_err();
        assert!(err.to_string().contains("half-life-days"));
    }
}
