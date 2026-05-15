//! `simaris similar <id>` — near-duplicate detection primitive.
//!
//! For a single input unit, return the top-K most similar units in the store.
//! Similarity is a weighted combination of three signals:
//!
//! - `vec_sim` — rank-derived proximity from the lance KNN leg. The source
//!   unit's body is embedded with the same model the lance dataset was
//!   backfilled with (bge-m3 by default); the top-`candidate_pool` KNN
//!   neighbours are scored as `1 - (rank / candidate_pool)`. Skipped when
//!   the lance dataset is absent or `--no-vec` is passed.
//! - `tag_overlap` — Jaccard index over tag sets: `|A ∩ B| / |A ∪ B|`. Zero
//!   when either side has no tags.
//! - `type_match` — 1.0 if both units share the same type, else 0.0.
//!
//! Plus a passive signal carried alongside `score` (not summed in):
//!
//! - `content_overlap` — Jaccard index over word-token sets after stripping
//!   leading YAML frontmatter (case-folded, tokens len≥3). Used by `cluster`
//!   to demote a `near-dup` cluster to `related` when vec sim is high but
//!   the literal text doesn't actually overlap (task pbhm). Computed per-hit
//!   so downstream tools can apply their own threshold.
//!
//! Final score: `α·vec_sim + β·tag_overlap + γ·type_match`. Default weights
//! are `α=0.6, β=0.3, γ=0.1`. Each is overridable via env:
//! `SIMARIS_SIM_ALPHA`, `SIMARIS_SIM_BETA`, `SIMARIS_SIM_GAMMA`. Weights are
//! placeholders pending downstream calibration (caez follow-up).
//!
//! Output: always JSON. The cluster command (gnns) and any downstream dedup
//! tooling consumes this primitive programmatically — no text mode.
//!
//! The source unit is always excluded from results. Archived units are hidden
//! unless `--include-archived` is set, mirroring the rest of the CLI.

use crate::db;
use crate::hybrid;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Hard cap on the lance candidate pool. Top-K is clamped to this on the CLI
/// edge so a malicious or distracted caller can't blow up retrieval cost.
const CANDIDATE_POOL: usize = 50;

/// Default scoring weights. Tunable via `SIMARIS_SIM_ALPHA` / `_BETA` /
/// `_GAMMA`. Calibration is a separate concern (out of scope for caez).
const DEFAULT_ALPHA: f64 = 0.6;
const DEFAULT_BETA: f64 = 0.3;
const DEFAULT_GAMMA: f64 = 0.1;

/// One ranked similarity hit. JSON-serialisable; field names are the public
/// contract consumed by downstream tooling (cluster, /api/similar).
#[derive(Debug, Serialize)]
pub struct SimilarHit {
    pub id: String,
    #[serde(rename = "type")]
    pub unit_type: String,
    pub slug: Option<String>,
    pub vec_sim: f64,
    pub tag_overlap: f64,
    pub type_match: f64,
    /// Word-token Jaccard between source and candidate bodies (frontmatter
    /// stripped). Carried alongside `score` but NOT folded into it — the
    /// cluster command uses it as a separate near-dup gate (task pbhm).
    pub content_overlap: f64,
    pub score: f64,
    pub content_preview: String,
}

/// Resolve scoring weights from env (with defaults). Bad values fall back to
/// the default — never crash the CLI on a malformed env var.
fn weights() -> (f64, f64, f64) {
    let parse = |var: &str, default: f64| {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(default)
    };
    (
        parse("SIMARIS_SIM_ALPHA", DEFAULT_ALPHA),
        parse("SIMARIS_SIM_BETA", DEFAULT_BETA),
        parse("SIMARIS_SIM_GAMMA", DEFAULT_GAMMA),
    )
}

/// Jaccard index over two tag lists. Treats both as sets (case-sensitive).
/// `|A ∩ B| / |A ∪ B|`. Both empty → 0.0 (no signal, not "perfect match").
fn tag_jaccard(a: &[String], b: &[String]) -> f64 {
    let a_set: HashSet<&String> = a.iter().collect();
    let b_set: HashSet<&String> = b.iter().collect();
    let inter = a_set.intersection(&b_set).count();
    let union = a_set.union(&b_set).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// Strip a leading YAML frontmatter block delimited by `---` … `---`.
/// Returns the input unchanged when no leading `---` is present.
fn strip_leading_frontmatter(s: &str) -> &str {
    let s = s.trim_start_matches('\u{feff}');
    if let Some(rest) = s.strip_prefix("---") {
        // Look for the closing `---` on its own line.
        if let Some(end) = rest.find("\n---") {
            let after = &rest[end + 4..];
            // Skip the trailing newline after the closing ---, if any.
            return after.strip_prefix('\n').unwrap_or(after);
        }
    }
    s
}

/// Lower-case word-token set for content-overlap Jaccard. Tokens are
/// `[a-z0-9_]+` runs of length ≥ 3 — drops single letters, punctuation,
/// and tiny stop-words that would inflate the union without signal.
fn content_tokens(s: &str) -> HashSet<String> {
    let body = strip_leading_frontmatter(s).to_ascii_lowercase();
    let mut out: HashSet<String> = HashSet::new();
    let mut current = String::new();
    for ch in body.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            if current.chars().count() >= 3 {
                out.insert(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if current.chars().count() >= 3 {
        out.insert(current);
    }
    out
}

/// Jaccard index over word-token sets after frontmatter strip + lowercase.
/// Both empty → 0.0 (no signal). Used by cluster as a near-dup gate
/// independent of `vec_sim` — high cosine on bge-m3 can fire on units that
/// share vocabulary but cover distinct subjects (task pbhm pilot finding).
pub fn content_overlap_jaccard(a: &str, b: &str) -> f64 {
    let ta = content_tokens(a);
    let tb = content_tokens(b);
    let union = ta.union(&tb).count();
    if union == 0 {
        0.0
    } else {
        ta.intersection(&tb).count() as f64 / union as f64
    }
}

/// First-line preview (≤80 chars). Mirrors `display::short_id`-style
/// rendering used elsewhere — gives the caller a human anchor without
/// pulling the full body.
fn content_preview(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    first_line.chars().take(80).collect()
}

/// Main entry point. `id` is already resolved (slug → uuid) by the CLI layer.
///
/// Behaviour:
/// - `no_vec` = true → skip lance entirely; rank every live unit by
///   `β·tag_overlap + γ·type_match`. O(N) scan; fine for current store size.
/// - `no_vec` = false → embed the source body, pull lance top-`CANDIDATE_POOL`,
///   score with all three signals.
///
/// `top_k` is clamped to `CANDIDATE_POOL` (50). `threshold` filters results
/// strictly below the cut-off BEFORE the top-K slice (a tight threshold can
/// return fewer than `top_k` results — by design).
pub fn similar(
    conn: &Connection,
    id: &str,
    top_k: usize,
    threshold: f64,
    no_vec: bool,
    include_archived: bool,
) -> Result<Vec<SimilarHit>> {
    let source = db::get_unit(conn, id).with_context(|| format!("source unit {id} not found"))?;
    let (alpha, beta, gamma) = weights();
    let top_k = top_k.clamp(1, CANDIDATE_POOL);

    // Build the candidate set + per-candidate vec_sim. Two paths:
    //
    // - vec path: lance KNN gives an ordered id list; vec_sim = 1 - rank/pool.
    // - no-vec path: every live unit is a candidate with vec_sim = 0; the
    //   tag/type legs do all the work.
    let mut vec_sim_by_id: HashMap<String, f64> = HashMap::new();
    let candidate_ids: Vec<String> = if no_vec {
        // Pure tag/type ranking — scan the whole store and let the scorer
        // filter by threshold + top_k. Excludes self in the loop below.
        db::list_units(conn, None, include_archived)?
            .into_iter()
            .map(|u| u.id)
            .collect()
    } else {
        match hybrid::HybridConfig::discover()? {
            Some(cfg) => {
                // Mirror the backfill pre-processing: strip frontmatter before
                // embedding so the query vector represents prose + scope, not
                // bookkeeping fields like `refs:` (task ppjs).
                let qtext = simaris_vec::embed::embed_input(&source.content);
                let qvec = cfg.embed_text(&qtext)?;
                let ranking = hybrid::run_vec_knn(&cfg, &qvec, CANDIDATE_POOL)?;
                for (rank, cid) in ranking.iter().enumerate() {
                    let sim = 1.0 - (rank as f64 / CANDIDATE_POOL as f64);
                    vec_sim_by_id.insert(cid.clone(), sim);
                }
                ranking
            }
            None => {
                // Lance absent — gracefully fall back to tag/type-only ranking
                // instead of erroring. Warn to stderr so the caller knows.
                eprintln!(
                    "warning: lance dataset not found; similar falling back to tag/type-only ranking (run `simaris vec backfill` to enable vector leg)"
                );
                db::list_units(conn, None, include_archived)?
                    .into_iter()
                    .map(|u| u.id)
                    .collect()
            }
        }
    };

    // Score every candidate. Self is always dropped. Archived candidates are
    // dropped unless include_archived is set.
    let mut hits: Vec<SimilarHit> = Vec::new();
    for cid in &candidate_ids {
        if cid == &source.id {
            continue;
        }
        let Ok(unit) = db::get_unit(conn, cid) else {
            // Lance can carry stale rows after deletes; tolerate the miss.
            continue;
        };
        if unit.archived && !include_archived {
            continue;
        }

        let vec_sim = vec_sim_by_id.get(cid).copied().unwrap_or(0.0);
        let tag_overlap = tag_jaccard(&source.tags, &unit.tags);
        let type_match = if unit.unit_type == source.unit_type {
            1.0
        } else {
            0.0
        };
        let score = alpha * vec_sim + beta * tag_overlap + gamma * type_match;

        if score < threshold {
            continue;
        }

        let content_overlap = content_overlap_jaccard(&source.content, &unit.content);
        let slug = db::get_slugs_for_unit(conn, &unit.id)
            .ok()
            .and_then(|v| v.into_iter().next());
        hits.push(SimilarHit {
            id: unit.id.clone(),
            unit_type: unit.unit_type.clone(),
            slug,
            vec_sim,
            tag_overlap,
            type_match,
            content_overlap,
            score,
            content_preview: content_preview(&unit.content),
        });
    }

    // Stable sort: score desc, then id asc on ties. Matches the determinism
    // contract elsewhere in simaris (rrf_fuse, search).
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(top_k);
    Ok(hits)
}

/// JSON-only printer. The command is always-JSON regardless of the global
/// `--json` flag — `similar` is a tooling primitive, not an interactive view.
pub fn print(hits: &[SimilarHit]) {
    // Pretty-printed: stdout is human-inspectable AND parses cleanly with jq.
    // matches the rest of the CLI's JSON output style (display::print_units).
    let json = serde_json::to_string_pretty(hits).expect("serialize SimilarHit list");
    println!("{json}");
}

/// Test-facing helper — runs the full similarity computation but lets the
/// caller pre-supply the vec ranking instead of calling lance. Used by
/// integration tests that need to exercise scoring + filtering without
/// standing up a lance dataset. Kept `pub(crate)` so it doesn't leak to
/// the public API.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn similar_with_ranking(
    conn: &Connection,
    id: &str,
    top_k: usize,
    threshold: f64,
    include_archived: bool,
    vec_ranking: &[String],
) -> Result<Vec<SimilarHit>> {
    let source = db::get_unit(conn, id)?;
    let (alpha, beta, gamma) = weights();
    let top_k = top_k.clamp(1, CANDIDATE_POOL);

    let mut vec_sim_by_id: HashMap<String, f64> = HashMap::new();
    for (rank, cid) in vec_ranking.iter().enumerate() {
        let sim = 1.0 - (rank as f64 / CANDIDATE_POOL as f64);
        vec_sim_by_id.insert(cid.clone(), sim);
    }

    let mut hits: Vec<SimilarHit> = Vec::new();
    for cid in vec_ranking {
        if cid == &source.id {
            continue;
        }
        let Ok(unit) = db::get_unit(conn, cid) else {
            continue;
        };
        if unit.archived && !include_archived {
            continue;
        }
        let vec_sim = *vec_sim_by_id.get(cid).unwrap_or(&0.0);
        let tag_overlap = tag_jaccard(&source.tags, &unit.tags);
        let type_match = if unit.unit_type == source.unit_type {
            1.0
        } else {
            0.0
        };
        let score = alpha * vec_sim + beta * tag_overlap + gamma * type_match;
        if score < threshold {
            continue;
        }
        let content_overlap = content_overlap_jaccard(&source.content, &unit.content);
        let slug = db::get_slugs_for_unit(conn, &unit.id)
            .ok()
            .and_then(|v| v.into_iter().next());
        hits.push(SimilarHit {
            id: unit.id.clone(),
            unit_type: unit.unit_type.clone(),
            slug,
            vec_sim,
            tag_overlap,
            type_match,
            content_overlap,
            score,
            content_preview: content_preview(&unit.content),
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(top_k);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_jaccard_basic() {
        let a: Vec<String> = vec!["x".into(), "y".into(), "z".into()];
        let b: Vec<String> = vec!["y".into(), "z".into(), "w".into()];
        // intersect = 2 (y, z), union = 4 (x, y, z, w) → 0.5
        assert!((tag_jaccard(&a, &b) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tag_jaccard_empty_both() {
        let a: Vec<String> = vec![];
        let b: Vec<String> = vec![];
        assert_eq!(tag_jaccard(&a, &b), 0.0);
    }

    #[test]
    fn tag_jaccard_empty_one_side() {
        let a: Vec<String> = vec!["x".into()];
        let b: Vec<String> = vec![];
        assert_eq!(tag_jaccard(&a, &b), 0.0);
    }

    #[test]
    fn tag_jaccard_identical() {
        let a: Vec<String> = vec!["x".into(), "y".into()];
        let b: Vec<String> = vec!["y".into(), "x".into()];
        assert!((tag_jaccard(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn content_preview_truncates_to_80() {
        let body = "a".repeat(200);
        assert_eq!(content_preview(&body).chars().count(), 80);
    }

    #[test]
    fn content_preview_first_line_only() {
        let body = "first line\nsecond line";
        assert_eq!(content_preview(body), "first line");
    }

    #[test]
    fn strip_leading_frontmatter_block() {
        let body = "---\ntype: lesson\nrefs:\n---\nactual body here";
        assert_eq!(strip_leading_frontmatter(body), "actual body here");
    }

    #[test]
    fn strip_leading_frontmatter_passthrough() {
        let body = "no frontmatter here";
        assert_eq!(strip_leading_frontmatter(body), body);
    }

    #[test]
    fn content_tokens_filter_short_and_lowercase() {
        let toks = content_tokens("Foo Bar a IS the_KEY x");
        // a/is/the/x dropped (len<3), foo/bar/the_key kept (case-folded).
        assert!(toks.contains("foo"));
        assert!(toks.contains("bar"));
        assert!(toks.contains("the_key"));
        assert!(!toks.contains("is"));
        assert!(!toks.contains("a"));
        assert!(!toks.contains("x"));
    }

    #[test]
    fn content_overlap_jaccard_basic() {
        // Symmetric, identical bodies → 1.0 (after frontmatter strip).
        let a = "alpha beta gamma";
        let b = "alpha beta gamma";
        assert!((content_overlap_jaccard(a, b) - 1.0).abs() < 1e-9);

        // Disjoint vocab → 0.0.
        let a = "alpha beta gamma";
        let b = "delta epsilon zeta";
        assert!(content_overlap_jaccard(a, b) < 1e-9);
    }

    #[test]
    fn content_overlap_jaccard_handles_frontmatter() {
        // Frontmatter words must NOT contribute. With strip, the two bodies
        // share only "actual body here" → overlap is high.
        let a = "---\nshared:\n  - tag-a\n  - tag-b\n---\nactual body here";
        let b = "---\nshared:\n  - tag-c\n  - tag-d\n---\nactual body here";
        let j = content_overlap_jaccard(a, b);
        assert!(j > 0.99, "expected near-1.0, got {j}");
    }

    #[test]
    fn content_overlap_jaccard_empty_both() {
        // Two frontmatter-only bodies leave empty token sets → 0.0.
        let a = "---\n---\n";
        let b = "---\n---\n";
        assert_eq!(content_overlap_jaccard(a, b), 0.0);
    }

    // Env-touching weight tests run sequentially under a single #[test] so
    // they don't race in cargo's default parallel runner. Splitting them into
    // separate tests races on the shared process env.
    #[test]
    fn weights_env_handling() {
        // Clean baseline.
        unsafe {
            std::env::remove_var("SIMARIS_SIM_ALPHA");
            std::env::remove_var("SIMARIS_SIM_BETA");
            std::env::remove_var("SIMARIS_SIM_GAMMA");
        }
        let (a, b, g) = weights();
        assert_eq!(a, DEFAULT_ALPHA);
        assert_eq!(b, DEFAULT_BETA);
        assert_eq!(g, DEFAULT_GAMMA);

        // Override path.
        unsafe {
            std::env::set_var("SIMARIS_SIM_ALPHA", "0.5");
            std::env::set_var("SIMARIS_SIM_BETA", "0.25");
            std::env::set_var("SIMARIS_SIM_GAMMA", "0.25");
        }
        let (a, b, g) = weights();
        assert!((a - 0.5).abs() < 1e-9);
        assert!((b - 0.25).abs() < 1e-9);
        assert!((g - 0.25).abs() < 1e-9);

        // Bad values fall back to defaults — non-finite, negative, garbage.
        unsafe {
            std::env::set_var("SIMARIS_SIM_ALPHA", "not-a-number");
            std::env::set_var("SIMARIS_SIM_BETA", "-1.0");
            std::env::set_var("SIMARIS_SIM_GAMMA", "inf");
        }
        let (a, b, g) = weights();
        assert_eq!(a, DEFAULT_ALPHA);
        assert_eq!(b, DEFAULT_BETA);
        // `inf` parses but is_finite filters it.
        assert_eq!(g, DEFAULT_GAMMA);

        // Clean up so we don't leak into other test files.
        unsafe {
            std::env::remove_var("SIMARIS_SIM_ALPHA");
            std::env::remove_var("SIMARIS_SIM_BETA");
            std::env::remove_var("SIMARIS_SIM_GAMMA");
        }
    }
}

