//! ask: hybrid query — lance vector KNN ∪ tantivy text search, RRF fusion.
//!
//! - k=60 matches the sqlite-vec sibling for benchmark comparability.
//! - Top-50 per leg, top-10 fused. Matches M3-redo-2 cell at lance × bge-m3
//!   (`simaris-m3-redo-2-results-2026-05-03`).
//! - Vector path returns zero-distance ties when run with placeholder
//!   embeddings; structural smoke test of the fusion pipeline.

use anyhow::{Context, Result};
use arrow_array::StringArray;
use futures::TryStreamExt;
use lance::Dataset;
use lance::dataset::scanner::Scanner;
use std::collections::HashMap;
use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::{Index, schema::Value};

use crate::RRF_K;

/// Pure RRF fusion on two ranked id lists. Useful for unit tests and reuse
/// from the M5.2 hybrid `simaris search` path.
pub fn rrf_fuse(vec_ranking: &[String], text_ranking: &[String], n: usize) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, id) in vec_ranking.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / ((rank + 1 + RRF_K) as f64);
    }
    for (rank, id) in text_ranking.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / ((rank + 1 + RRF_K) as f64);
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused.truncate(n);
    fused
}

pub async fn run(lance_dir: &Path, tantivy_dir: &Path, query: &str, n: usize) -> Result<()> {
    let units = Dataset::open(lance_dir.join("units.lance").to_str().context("utf8")?).await?;

    // VECTOR side: placeholder — KNN against zero query vector returns
    // ties; ranks are content-id ordered. Real embeddings ship via M5.3
    // backfill tooling.
    let n_v = (n * 5).max(50);
    let mut scanner: Scanner = units.scan();
    scanner.project(&["id"])?;
    scanner.limit(Some(n_v as i64), None)?;
    let stream = scanner.try_into_stream().await?;
    let batches: Vec<_> = stream.try_collect().await?;
    let mut vec_ranking: Vec<String> = Vec::new();
    for b in &batches {
        let ids = b
            .column_by_name("id")
            .context("missing id col")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("id not string")?;
        for i in 0..b.num_rows() {
            vec_ranking.push(ids.value(i).to_string());
            if vec_ranking.len() >= n_v {
                break;
            }
        }
    }

    // TEXT side: tantivy.
    let index = Index::open_in_dir(tantivy_dir)?;
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let f_id = schema.get_field("id")?;
    let f_content = schema.get_field("content")?;
    let f_tags = schema.get_field("tags")?;
    let parser = QueryParser::for_index(&index, vec![f_content, f_tags]);
    let q = parser.parse_query(query)?;
    let n_t = (n * 5).max(50);
    let top = searcher.search(&q, &TopDocs::with_limit(n_t))?;
    let mut text_ranking: Vec<String> = Vec::new();
    for (_score, addr) in top {
        let d: tantivy::TantivyDocument = searcher.doc(addr)?;
        if let Some(v) = d.get_first(f_id)
            && let Some(s) = v.as_str()
        {
            text_ranking.push(s.to_string());
        }
    }

    let fused = rrf_fuse(&vec_ranking, &text_ranking, n);

    println!("query: {query}");
    println!("vec.candidates: {}", vec_ranking.len());
    println!("text.candidates: {}", text_ranking.len());
    println!("rrf.k: {RRF_K}");
    println!("results:");
    for (id, score) in &fused {
        println!("  {id}\t{score:.6}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn rrf_single_leg_matches_input_order() {
        let v = s(&["a", "b", "c"]);
        let t: Vec<String> = vec![];
        let f = rrf_fuse(&v, &t, 3);
        assert_eq!(f[0].0, "a");
        assert_eq!(f[1].0, "b");
        assert_eq!(f[2].0, "c");
    }

    #[test]
    fn rrf_overlap_boosts_shared_id() {
        let v = s(&["a", "b", "c"]);
        let t = s(&["b", "x", "y"]);
        let f = rrf_fuse(&v, &t, 4);
        // b is rank-2 in vec and rank-1 in text → both legs contribute.
        assert_eq!(f[0].0, "b");
    }

    #[test]
    fn rrf_truncates_to_n() {
        let v = s(&["a", "b", "c", "d", "e"]);
        let t = s(&["a", "b"]);
        let f = rrf_fuse(&v, &t, 3);
        assert_eq!(f.len(), 3);
    }

    #[test]
    fn rrf_score_uses_k_constant() {
        // First-rank single-leg contribution = 1 / (1 + RRF_K).
        let v = s(&["only"]);
        let t: Vec<String> = vec![];
        let f = rrf_fuse(&v, &t, 1);
        let expected = 1.0 / ((1 + RRF_K) as f64);
        assert!((f[0].1 - expected).abs() < 1e-12);
    }
}
