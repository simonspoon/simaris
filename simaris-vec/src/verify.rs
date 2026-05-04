//! verify-links: read back lance datasets and compare counts to source sqlite.
//! Falsifier #3: zero drops. We fail loud if any count mismatches.
//! size-report: walk lance + tantivy directories and report bytes.

use anyhow::{Context, Result};
use arrow_array::{Array, RecordBatch, StringArray};
use futures::StreamExt;
use lance::Dataset;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::Path;

pub async fn run(sqlite: &Path, lance_dir: &Path) -> Result<()> {
    let conn = Connection::open(sqlite)?;

    // sqlite reference counts.
    let src_units: i64 = conn.query_row("select count(*) from units", [], |r| r.get(0))?;
    let src_units_aspect: i64 = conn.query_row(
        "select count(*) from units where type='aspect' and archived=0",
        [],
        |r| r.get(0),
    )?;
    let src_links: i64 = conn.query_row("select count(*) from links", [], |r| r.get(0))?;
    let src_slugs: i64 = conn.query_row("select count(*) from slugs", [], |r| r.get(0))?;
    let src_marks: i64 = conn.query_row("select count(*) from marks", [], |r| r.get(0))?;
    let src_inbox: i64 = conn.query_row("select count(*) from inbox", [], |r| r.get(0))?;

    let mut src_links_by_rel: BTreeMap<String, i64> = BTreeMap::new();
    {
        let mut stmt =
            conn.prepare("select relationship, count(*) from links group by relationship")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (rel, c) = row?;
            src_links_by_rel.insert(rel, c);
        }
    }

    // lance load + counts.
    let units = open_dataset(&lance_dir.join("units.lance")).await?;
    let links = open_dataset(&lance_dir.join("links.lance")).await?;
    let slugs = open_dataset(&lance_dir.join("slugs.lance")).await?;
    let marks = open_dataset(&lance_dir.join("marks.lance")).await?;
    let inbox = open_dataset(&lance_dir.join("inbox.lance")).await?;

    let dst_units = units.count_rows(None).await? as i64;
    let dst_links = links.count_rows(None).await? as i64;
    let dst_slugs = slugs.count_rows(None).await? as i64;
    let dst_marks = marks.count_rows(None).await? as i64;
    let dst_inbox = inbox.count_rows(None).await? as i64;

    // count aspects in lance.
    let dst_units_aspect = count_filter(&units, |b| {
        let t = b
            .column_by_name("type")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let a = b
            .column_by_name("archived")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::BooleanArray>()
            .unwrap();
        let mut n = 0i64;
        for i in 0..b.num_rows() {
            if t.value(i) == "aspect" && !a.value(i) {
                n += 1;
            }
        }
        n
    })
    .await?;

    // count links by relationship in lance.
    let dst_links_by_rel = count_links_by_rel(&links).await?;

    let mut all_ok = true;
    let mut diffs = Vec::<String>::new();

    macro_rules! cmp {
        ($name:expr, $src:expr, $dst:expr) => {
            if $src != $dst {
                diffs.push(format!("{}: src={} dst={}", $name, $src, $dst));
                all_ok = false;
            }
        };
    }
    cmp!("units.total", src_units, dst_units);
    cmp!("units.aspect_live", src_units_aspect, dst_units_aspect);
    cmp!("links.total", src_links, dst_links);
    cmp!("slugs.total", src_slugs, dst_slugs);
    cmp!("marks.total", src_marks, dst_marks);
    cmp!("inbox.total", src_inbox, dst_inbox);

    let all_rels = [
        "related_to",
        "part_of",
        "depends_on",
        "contradicts",
        "supersedes",
        "sourced_from",
    ];
    let mut rel_lines = Vec::new();
    for rel in all_rels {
        let s = *src_links_by_rel.get(rel).unwrap_or(&0);
        let d = *dst_links_by_rel.get(rel).unwrap_or(&0);
        rel_lines.push(format!("links.{rel}: src={s} dst={d}"));
        if s != d {
            diffs.push(format!("links.{rel}: src={s} dst={d}"));
            all_ok = false;
        }
    }

    println!("verify.units.total: src={src_units} dst={dst_units}");
    println!("verify.units.aspect_live: src={src_units_aspect} dst={dst_units_aspect}");
    println!("verify.links.total: src={src_links} dst={dst_links}");
    println!("verify.slugs.total: src={src_slugs} dst={dst_slugs}");
    println!("verify.marks.total: src={src_marks} dst={dst_marks}");
    println!("verify.inbox.total: src={src_inbox} dst={dst_inbox}");
    for l in &rel_lines {
        println!("verify.{l}");
    }
    println!("---");
    if all_ok {
        println!("FALSIFIER_3_RESULT: PASS (zero drops)");
    } else {
        println!("FALSIFIER_3_RESULT: FAIL");
        for d in &diffs {
            println!("  diff: {d}");
        }
    }
    Ok(())
}

async fn open_dataset(p: &Path) -> Result<Dataset> {
    let uri = p.to_str().context("utf8")?;
    Ok(Dataset::open(uri).await?)
}

async fn count_filter<F>(ds: &Dataset, mut f: F) -> Result<i64>
where
    F: FnMut(&RecordBatch) -> i64,
{
    let mut scan = ds.scan();
    scan.project(&["type", "archived"])?;
    let mut stream = scan.try_into_stream().await?;
    let mut total = 0i64;
    while let Some(b) = stream.next().await {
        total += f(&b?);
    }
    Ok(total)
}

async fn count_links_by_rel(ds: &Dataset) -> Result<BTreeMap<String, i64>> {
    let mut scan = ds.scan();
    scan.project(&["relationship"])?;
    let mut stream = scan.try_into_stream().await?;
    let mut out = BTreeMap::<String, i64>::new();
    while let Some(b) = stream.next().await {
        let b = b?;
        let r = b
            .column_by_name("relationship")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..b.num_rows() {
            *out.entry(r.value(i).to_string()).or_insert(0) += 1;
        }
    }
    Ok(out)
}

pub fn size_report(lance_dir: &Path, tantivy_dir: &Path) -> Result<()> {
    let lance_bytes = dir_size(lance_dir)?;
    let tantivy_bytes = dir_size(tantivy_dir)?;
    let total = lance_bytes + tantivy_bytes;
    println!("size.lance_bytes: {lance_bytes}");
    println!("size.tantivy_bytes: {tantivy_bytes}");
    println!("size.total_bytes: {total}");
    println!("size.lance_mb: {:.2}", lance_bytes as f64 / 1_048_576.0);
    println!("size.tantivy_mb: {:.2}", tantivy_bytes as f64 / 1_048_576.0);
    println!("size.total_mb: {:.2}", total as f64 / 1_048_576.0);
    let limit = 100u64 * 1_048_576;
    println!("size.limit_mb: 100");
    if total <= limit {
        println!("FALSIFIER_2_RESULT: PASS (total <= 100 MB)");
    } else if total <= 5 * limit {
        println!("FALSIFIER_2_RESULT: SOFT_FAIL (>100MB but <500MB)");
    } else {
        println!("FALSIFIER_2_RESULT: HARD_FAIL (>500MB)");
    }
    Ok(())
}

fn dir_size(p: &Path) -> Result<u64> {
    if !p.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in walkdir(p)? {
        let m = std::fs::metadata(&entry)?;
        if m.is_file() {
            total += m.len();
        }
    }
    Ok(total)
}

fn walkdir(p: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut stack = vec![p.to_path_buf()];
    let mut out = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Ok(out)
}
