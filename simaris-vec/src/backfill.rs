//! Idempotent embedding backfill — `simaris vec backfill` engine.
//!
//! Strategy (per `simaris-m5-impl-plan-2026-05-04` M5.3): row-by-row
//! presence check by unit id. We identify the IDs already present in the
//! lance `units.lance` dataset, embed only the missing IDs via ollama bge-m3,
//! and `WriteMode::Append` them onto the dataset.
//!
//! Bootstrap (lance dataset absent on first run) builds the side tables
//! (links / slugs / marks / inbox) via [`crate::migrate`] helpers, plus the
//! tantivy index. Re-running on a populated dataset is a fast no-op:
//! existing-id read + zero embeds + summary printed.
//!
//! Deadlock workaround: if the Rust ollama HTTP path hangs (per
//! `simaris-m3-2-falsifiers-2026-05-03`), the direct-write Python fallback
//! at `tools/direct_backfill_ollama.py` produces an equivalent dataset. The
//! `simaris vec backfill --help` text surfaces this fallback.
//!
//! Resume: backfill is interrupt-safe. Each appended batch flushes a new
//! lance fragment, so re-running picks up where it left off via the
//! presence-check (already-appended ids are skipped).

use anyhow::{Context, Result, bail};
use arrow_array::builder::Float32Builder;
use arrow_array::{
    FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field};
use futures::TryStreamExt;
use lance::Dataset;
use lance::dataset::{WriteMode, WriteParams};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::embed::OllamaEmbedClient;
use crate::migrate;

/// Result of a backfill run. Stable for sitrep emission.
#[derive(Debug, Clone)]
pub struct BackfillStats {
    pub total_units: usize,
    pub embedded: usize,
    pub skipped: usize,
    pub elapsed_secs: f64,
    pub throughput: f64,
    pub bootstrapped: bool,
}

/// One unit row pulled from sqlite. Only the columns the production
/// `simaris search` lance schema needs: id, content, tags, embedding.
/// Matches the `/tmp/m3-bench/lance/bge-m3/units.lance` baseline that the
/// M5.2 sitrep regression numbers were measured against.
struct UnitRow {
    id: String,
    content: String,
    tags: String,
}

/// Run the backfill. `dim` is the embedding width (1024 for bge-m3, 768 for nomic).
///
/// `lance_dir` holds `units.lance` plus the side-table datasets. `tantivy_dir`
/// is the tantivy index directory; conventionally `lance_dir/tantivy`.
pub async fn run(
    sqlite: &Path,
    lance_dir: &Path,
    tantivy_dir: &Path,
    model: &str,
    dim: usize,
    batch_size: usize,
    ollama_url: &str,
) -> Result<BackfillStats> {
    let conn = Connection::open(sqlite).with_context(|| format!("open {}", sqlite.display()))?;

    // Total active units = the working set the dataset must cover.
    let total_units: usize =
        conn.query_row("select count(*) from units where archived = 0", [], |r| {
            r.get::<_, i64>(0)
        })? as usize;

    let units_path = lance_dir.join("units.lance");
    let bootstrapped = !units_path.exists();

    // Bootstrap: empty units dataset + side tables + tantivy.
    if bootstrapped {
        eprintln!(
            "backfill: bootstrapping new dataset at {}",
            lance_dir.display()
        );
        std::fs::create_dir_all(lance_dir)?;
        write_empty_units(&units_path, dim).await?;
        bootstrap_sides(&conn, lance_dir, tantivy_dir).await?;
    } else {
        eprintln!(
            "backfill: extending existing dataset at {}",
            lance_dir.display()
        );
    }

    // Existing ids — drives idempotence.
    let existing_ids = read_existing_ids(&units_path).await?;
    eprintln!(
        "backfill: existing={} sqlite_active={} model={}",
        existing_ids.len(),
        total_units,
        model
    );

    let missing = collect_missing(&conn, &existing_ids)?;
    let n_missing = missing.len();

    if n_missing == 0 {
        eprintln!("backfill: 0 missing — no-op");
        // Still rebuild tantivy if it's gone (kept in lockstep with sqlite).
        ensure_tantivy(&conn, tantivy_dir)?;
        return Ok(BackfillStats {
            total_units,
            embedded: 0,
            skipped: existing_ids.len(),
            elapsed_secs: 0.0,
            throughput: 0.0,
            bootstrapped,
        });
    }

    // Match the M3-redo-2 backfill pipeline (`tools/direct_backfill_ollama.py`):
    // truncate input to MAX_CHARS chars before embedding. Two reasons:
    //   1. bge-m3 hard-fails (ollama 500) on inputs that overflow the
    //      8192-token context window — observed on the 11644-char unit
    //      `019df3f1-22e9-73c1-904d-fa571e124f6c` during M5.3 dispatch.
    //   2. Port-fidelity with the existing /tmp/m3-bench/lance/bge-m3 dataset
    //      that M3-redo-2 produced (and that M5.2's regression baseline reads).
    // Default 2000 matches the prior pipeline. `SIMARIS_EMBED_MAX_CHARS`
    // overrides for callers who want a larger budget.
    let max_chars: usize = std::env::var("SIMARIS_EMBED_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    eprintln!(
        "backfill: missing={n_missing} batch_size={batch_size} ollama={ollama_url} max_chars={max_chars}"
    );
    let client = OllamaEmbedClient::new(ollama_url, model);
    let started = Instant::now();
    let mut embedded = 0usize;

    for (batch_idx, chunk) in missing.chunks(batch_size).enumerate() {
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(chunk.len());
        for u in chunk {
            let truncated: String = u.content.chars().take(max_chars).collect();
            let v = client.embed(&truncated).with_context(|| {
                format!(
                    "ollama embed failed (model={model}, unit={}); see `simaris vec backfill --help` for the direct-write Python fallback",
                    u.id
                )
            })?;
            if v.len() != dim {
                bail!(
                    "ollama returned dim {} != expected {} (model={}, unit={})",
                    v.len(),
                    dim,
                    model,
                    u.id
                );
            }
            vectors.push(v);
        }
        append_units_batch(&units_path, dim, chunk, &vectors).await?;
        embedded += chunk.len();
        let elapsed = started.elapsed().as_secs_f64();
        let tput = embedded as f64 / elapsed.max(0.001);
        eprintln!(
            "backfill: batch {} embedded {}/{} ({:.2}/s elapsed={:.1}s)",
            batch_idx + 1,
            embedded,
            n_missing,
            tput,
            elapsed
        );
    }

    // Refresh tantivy whenever new units land — it indexes the live sqlite
    // state, which is the same view `simaris search` reads.
    ensure_tantivy(&conn, tantivy_dir)?;

    let elapsed = started.elapsed().as_secs_f64();
    let throughput = embedded as f64 / elapsed.max(0.001);

    Ok(BackfillStats {
        total_units,
        embedded,
        skipped: existing_ids.len(),
        elapsed_secs: elapsed,
        throughput,
        bootstrapped,
    })
}

fn collect_missing(conn: &Connection, existing: &HashSet<String>) -> Result<Vec<UnitRow>> {
    let mut stmt =
        conn.prepare("select id, content, tags from units where archived = 0 order by id")?;
    let rows = stmt.query_map([], |r| {
        Ok(UnitRow {
            id: r.get(0)?,
            content: r.get(1)?,
            tags: r.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let u = row?;
        if !existing.contains(&u.id) {
            out.push(u);
        }
    }
    Ok(out)
}

async fn read_existing_ids(units_path: &Path) -> Result<HashSet<String>> {
    if !units_path.exists() {
        return Ok(HashSet::new());
    }
    let ds = Dataset::open(units_path.to_str().context("lance path utf8")?).await?;
    let mut scanner = ds.scan();
    scanner.project(&["id"])?;
    let stream = scanner.try_into_stream().await?;
    let batches: Vec<RecordBatch> = stream.try_collect().await?;
    let mut out = HashSet::with_capacity(1024);
    for b in &batches {
        let col = b
            .column_by_name("id")
            .context("units.lance missing id column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("id column not Utf8")?;
        for i in 0..b.num_rows() {
            out.insert(col.value(i).to_string());
        }
    }
    Ok(out)
}

/// Production lance schema for `units.lance`. 4 columns: id, content, tags,
/// embedding. Mirrors the `/tmp/m3-bench/lance/bge-m3` M3-redo-2 baseline so
/// the M5.2 hybrid leg + M5.3 backfill share a single binding schema.
fn production_units_schema(dim: usize) -> Arc<arrow_schema::Schema> {
    Arc::new(arrow_schema::Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("tags", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}

async fn write_empty_units(units_path: &Path, dim: usize) -> Result<()> {
    let schema = production_units_schema(dim);
    let empty_str = StringArray::from(Vec::<String>::new());
    let empty_emb_values = Float32Array::from(Vec::<f32>::new());
    let emb_field = Arc::new(Field::new("item", DataType::Float32, true));
    let empty_emb =
        FixedSizeListArray::try_new(emb_field, dim as i32, Arc::new(empty_emb_values), None)?;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(empty_str.clone()),
            Arc::new(empty_str.clone()),
            Arc::new(empty_str),
            Arc::new(empty_emb),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());
    let params = WriteParams {
        mode: WriteMode::Create,
        ..WriteParams::default()
    };
    let uri = units_path.to_str().context("path utf8")?;
    Dataset::write(reader, uri, Some(params)).await?;
    Ok(())
}

async fn bootstrap_sides(conn: &Connection, lance_dir: &Path, tantivy_dir: &Path) -> Result<()> {
    let links = lance_dir.join("links.lance");
    let slugs = lance_dir.join("slugs.lance");
    let marks = lance_dir.join("marks.lance");
    let inbox = lance_dir.join("inbox.lance");
    if !links.exists() {
        migrate::migrate_links(conn, &links).await?;
    }
    if !slugs.exists() {
        migrate::migrate_slugs(conn, &slugs).await?;
    }
    if !marks.exists() {
        migrate::migrate_marks(conn, &marks).await?;
    }
    if !inbox.exists() {
        migrate::migrate_inbox(conn, &inbox).await?;
    }
    if !tantivy_dir.exists() {
        std::fs::create_dir_all(tantivy_dir)?;
        migrate::build_tantivy(conn, tantivy_dir)?;
    }
    Ok(())
}

fn ensure_tantivy(conn: &Connection, tantivy_dir: &Path) -> Result<()> {
    // Always rebuild from the live sqlite state. tantivy build is fast
    // (seconds for 3.5k units) and avoids the complexity of incremental
    // tantivy commits across runs. Existing dirs are wiped first.
    if tantivy_dir.exists() {
        std::fs::remove_dir_all(tantivy_dir).ok();
    }
    std::fs::create_dir_all(tantivy_dir)?;
    migrate::build_tantivy(conn, tantivy_dir)?;
    Ok(())
}

async fn append_units_batch(
    units_path: &Path,
    dim: usize,
    rows: &[UnitRow],
    vectors: &[Vec<f32>],
) -> Result<()> {
    let schema = production_units_schema(dim);
    let n = rows.len();

    let ids = StringArray::from(rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
    let content = StringArray::from(rows.iter().map(|r| r.content.clone()).collect::<Vec<_>>());
    let tags = StringArray::from(rows.iter().map(|r| r.tags.clone()).collect::<Vec<_>>());

    let mut emb_b = Float32Builder::with_capacity(n * dim);
    for v in vectors {
        for &x in v {
            emb_b.append_value(x);
        }
    }
    let values_arr = emb_b.finish();
    let emb_field = Arc::new(Field::new("item", DataType::Float32, true));
    let emb = FixedSizeListArray::try_new(emb_field, dim as i32, Arc::new(values_arr), None)?;

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ids),
            Arc::new(content),
            Arc::new(tags),
            Arc::new(emb),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());
    let params = WriteParams {
        mode: WriteMode::Append,
        ..WriteParams::default()
    };
    let uri = units_path.to_str().context("path utf8")?;
    Dataset::write(reader, uri, Some(params)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_roundtrip() {
        // Trivial sanity — Debug + Clone derives present.
        let s = BackfillStats {
            total_units: 10,
            embedded: 7,
            skipped: 3,
            elapsed_secs: 1.5,
            throughput: 4.6,
            bootstrapped: true,
        };
        let copy = s.clone();
        assert_eq!(copy.total_units, 10);
        assert_eq!(copy.embedded, 7);
        assert!(copy.bootstrapped);
        let formatted = format!("{:?}", s);
        assert!(formatted.contains("BackfillStats"));
    }
}
