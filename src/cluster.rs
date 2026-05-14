//! `simaris cluster` — store-wide redundancy survey.
//!
//! Walks a candidate set (one tag, one type, or `--all`), runs the
//! `similar` primitive per-unit to gather top-K neighbours, then unions
//! them into connected components. Each component is annotated with one
//! or more *patterns* describing why the cluster is interesting:
//!
//! - `near-dup` — average edge `vec_sim` ≥ 0.85 (likely true duplicates).
//! - `temporal-log` — ≥ 50% of members carry a slug shaped `-YYYY-MM-DD`
//!   (incremental log entries that should roll up).
//! - `type-confused` — cluster spans ≥ 2 unit types and avg `vec_sim`
//!   ≥ 0.75 (same idea, inconsistently typed).
//! - `low-signal` (singleton) — 0 marks, no inbound links, age > 90 days.
//! - `orphan` (singleton) — no outbound `part_of` or `related_to` edges.
//!
//! Output schema (JSON):
//!
//! ```text
//! {
//!   "summary": {
//!     "total_units":   <int>,
//!     "cluster_count": <int>,
//!     "by_pattern":    { "<pattern>": <int>, ... }
//!   },
//!   "clusters": [
//!     {
//!       "cluster_id":       "<short>",
//!       "patterns":         ["<pattern>", ...],
//!       "suggested_action": "<action>",
//!       "reason":           "<short rationale>",
//!       "avg_vec_sim":      <f64>,
//!       "members": [
//!         { "id": "<uuid>", "slug": "<slug-or-null>", "type": "<type>",
//!           "age_days": <i64>, "marks": <i64>, "content_preview": "<≤80 chars>" },
//!         ...
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! Performance: per-unit KNN (bounded `--max-similar`) keeps the work
//! O(N·K), not O(N²). On the production store the embedding pass
//! dominates; pass `--no-vec` to skip it (tag/type-only ranking).

use crate::db;
use crate::similar;
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Default thresholds — exposed as constants so tests + downstream tools
/// can reference them by name rather than re-deriving the numbers.
pub const NEAR_DUP_AVG_VEC_SIM: f64 = 0.85;
pub const TYPE_CONFUSED_AVG_VEC_SIM: f64 = 0.75;
pub const LOW_SIGNAL_MIN_AGE_DAYS: i64 = 90;
pub const TEMPORAL_LOG_MIN_FRACTION: f64 = 0.5;

/// CLI-facing parameters. Mirrors the clap struct shape so main.rs can
/// pass-through without re-shuffling.
#[derive(Debug, Clone)]
pub struct ClusterParams {
    /// Tag filter (mutually exclusive with `all` at the CLI level).
    pub tag: Option<String>,
    /// Type filter (composes with `tag` / `all`).
    pub unit_type: Option<String>,
    /// `--all` — scan every live (non-archived) unit. The cluster
    /// implementation already treats absent filters as "everything";
    /// this flag exists at the API level so callers can be explicit.
    #[allow(dead_code)]
    pub all: bool,
    /// Drop multi-member clusters smaller than this; singletons can still
    /// surface if they match the low-signal / orphan single-unit patterns.
    pub min_cluster_size: usize,
    /// Per-unit top-K neighbour fetch — bounds the work to O(N·K).
    pub max_similar: usize,
    /// Edge cutoff — drop neighbour relationships scoring strictly below
    /// this value before union-find. Defaults to a low non-zero value so
    /// the graph isn't dense with noise.
    pub threshold: f64,
    /// Pass-through to `similar()` — disables the embedding leg.
    pub no_vec: bool,
}

impl Default for ClusterParams {
    fn default() -> Self {
        Self {
            tag: None,
            unit_type: None,
            all: false,
            min_cluster_size: 2,
            max_similar: 5,
            threshold: 0.3,
            no_vec: false,
        }
    }
}

/// One member of a cluster, with the signals that pattern detection
/// uses. JSON-serialised verbatim.
#[derive(Debug, Serialize)]
pub struct ClusterMember {
    pub id: String,
    pub slug: Option<String>,
    #[serde(rename = "type")]
    pub unit_type: String,
    pub age_days: i64,
    pub marks: i64,
    pub content_preview: String,
}

/// One cluster (component) in the output report.
#[derive(Debug, Serialize)]
pub struct Cluster {
    pub cluster_id: String,
    pub patterns: Vec<String>,
    pub suggested_action: String,
    pub reason: String,
    pub avg_vec_sim: f64,
    pub members: Vec<ClusterMember>,
}

/// Top-level summary block — used by the consolidation UI to render
/// the by-pattern histogram without re-walking the cluster list.
#[derive(Debug, Serialize)]
pub struct ClusterSummary {
    pub total_units: usize,
    pub cluster_count: usize,
    pub by_pattern: BTreeMap<String, usize>,
}

/// Full report — the shape printed to stdout (always JSON).
#[derive(Debug, Serialize)]
pub struct ClusterReport {
    pub summary: ClusterSummary,
    pub clusters: Vec<Cluster>,
}

/// Per-unit data lifted out of SQL once at the top of the run so the
/// pattern detectors aren't re-hitting the DB per cluster.
struct UnitSignals {
    id: String,
    slug: Option<String>,
    unit_type: String,
    age_days: i64,
    marks: i64,
    content_preview: String,
    inbound_links: usize,
    has_part_of_or_related_out: bool,
}

/// Simple integer union-find. Indexed by candidate-set position.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // Keep the smaller index as the canonical root for
            // stable cluster IDs.
            if ra < rb {
                self.parent[rb] = ra;
            } else {
                self.parent[ra] = rb;
            }
        }
    }
}

/// First-line preview (≤80 chars). Mirrors the helper in `similar.rs`
/// — kept private here so the two modules stay independent.
fn content_preview(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    first_line.chars().take(80).collect()
}

/// Marks count for one unit.
fn marks_for(conn: &Connection, id: &str) -> Result<i64> {
    let c: i64 = conn.query_row(
        "SELECT COUNT(*) FROM marks WHERE unit_id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    Ok(c)
}

/// Age in days via SQLite `julianday()` — matches the rest of simaris
/// (e.g. `dream.rs::days_between`).
fn age_days_for(conn: &Connection, created: &str) -> Result<i64> {
    let d: f64 = conn.query_row(
        "SELECT julianday('now') - julianday(?1)",
        params![created],
        |row| row.get(0),
    )?;
    Ok(d.floor() as i64)
}

/// Lightweight `-YYYY-MM-DD` matcher (any position) without pulling in
/// a regex crate. Returns true when a slug contains a `-` followed by
/// 4 digits, `-`, 2 digits, `-`, 2 digits.
fn slug_has_date_stamp(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    if bytes.len() < 11 {
        return false;
    }
    // Slide a window of length 11: '-YYYY-MM-DD'.
    for w in bytes.windows(11) {
        if w[0] == b'-'
            && w[1..5].iter().all(|c| c.is_ascii_digit())
            && w[5] == b'-'
            && w[6..8].iter().all(|c| c.is_ascii_digit())
            && w[8] == b'-'
            && w[9..11].iter().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

/// Resolve the candidate set under the supplied filters.
///
/// Filter composition:
/// - `all = true` → every live unit (type filter still applies).
/// - `tag = Some(t)` → JSON-tag-array contains `t`.
/// - `unit_type = Some(y)` → type matches.
/// - Archived units are always excluded; cluster is a survey of the
///   live store.
fn candidate_units(conn: &Connection, p: &ClusterParams) -> Result<Vec<db::Unit>> {
    // Pull the broad set via list_units, then post-filter by tag in
    // Rust. list_units already handles archive + type, so we delegate.
    let units = db::list_units(conn, p.unit_type.as_deref(), false)?;
    let filtered: Vec<db::Unit> = match &p.tag {
        Some(tag) => units
            .into_iter()
            .filter(|u| u.tags.iter().any(|t| t == tag))
            .collect(),
        None => units,
    };
    Ok(filtered)
}

/// Build per-unit signals (slug, marks, age, link counts). Heavy SQL
/// stays here so the pattern detectors below are pure functions over
/// in-memory data.
fn load_signals(conn: &Connection, units: &[db::Unit]) -> Result<HashMap<String, UnitSignals>> {
    let mut signals = HashMap::with_capacity(units.len());
    for u in units {
        let slug = db::get_slugs_for_unit(conn, &u.id)
            .ok()
            .and_then(|v| v.into_iter().next());
        let marks = marks_for(conn, &u.id)?;
        let age_days = age_days_for(conn, &u.created)?;
        let inbound_links = db::get_links_to(conn, &u.id).map(|v| v.len()).unwrap_or(0);
        let outbound = db::get_links_from(conn, &u.id).unwrap_or_default();
        let has_part_of_or_related_out = outbound
            .iter()
            .any(|l| l.relationship == "part_of" || l.relationship == "related_to");
        signals.insert(
            u.id.clone(),
            UnitSignals {
                id: u.id.clone(),
                slug,
                unit_type: u.unit_type.clone(),
                age_days,
                marks,
                content_preview: content_preview(&u.content),
                inbound_links,
                has_part_of_or_related_out,
            },
        );
    }
    Ok(signals)
}

/// One edge in the similarity graph — directed (`from` ran similar()
/// and `to` is one of its hits) but treated as undirected by union-find.
struct Edge {
    from: usize,
    to: usize,
    vec_sim: f64,
}

/// Walk every candidate, run `similar::similar`, keep hits inside the
/// candidate set whose score clears `threshold`. The candidate set is
/// the universe — anything outside it is ignored even if the KNN
/// returns it.
fn build_edges(
    conn: &Connection,
    units: &[db::Unit],
    p: &ClusterParams,
) -> Result<Vec<Edge>> {
    let index: HashMap<&str, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.id.as_str(), i))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let hits = similar::similar(
            conn,
            &u.id,
            p.max_similar,
            p.threshold,
            p.no_vec,
            false, // include_archived: cluster surveys live store only
        )?;
        for h in hits {
            if let Some(&j) = index.get(h.id.as_str()) {
                if i == j {
                    continue;
                }
                edges.push(Edge {
                    from: i,
                    to: j,
                    vec_sim: h.vec_sim,
                });
            }
        }
    }
    Ok(edges)
}

/// Pattern classification for a multi-member cluster. Returns the
/// ordered pattern label list (priority order: near-dup, temporal-log,
/// type-confused). A cluster may carry multiple labels.
fn classify_cluster(members: &[&UnitSignals], avg_vec_sim: f64) -> Vec<String> {
    let mut patterns: Vec<String> = Vec::new();

    if avg_vec_sim >= NEAR_DUP_AVG_VEC_SIM {
        patterns.push("near-dup".into());
    }

    // temporal-log — count slugs that contain a `-YYYY-MM-DD` window.
    let dated = members
        .iter()
        .filter(|m| m.slug.as_deref().is_some_and(slug_has_date_stamp))
        .count();
    if !members.is_empty()
        && (dated as f64 / members.len() as f64) >= TEMPORAL_LOG_MIN_FRACTION
    {
        patterns.push("temporal-log".into());
    }

    // type-confused — distinct types AND non-trivial average vec sim.
    let distinct_types: BTreeSet<&str> =
        members.iter().map(|m| m.unit_type.as_str()).collect();
    if distinct_types.len() >= 2 && avg_vec_sim >= TYPE_CONFUSED_AVG_VEC_SIM {
        patterns.push("type-confused".into());
    }

    patterns
}

/// Singleton pattern classification (orphan / low-signal). Returns the
/// label list — empty list = singleton should NOT be emitted.
fn classify_singleton(m: &UnitSignals) -> Vec<String> {
    let mut patterns: Vec<String> = Vec::new();
    if m.marks == 0 && m.inbound_links == 0 && m.age_days > LOW_SIGNAL_MIN_AGE_DAYS {
        patterns.push("low-signal".into());
    }
    if !m.has_part_of_or_related_out {
        patterns.push("orphan".into());
    }
    patterns
}

/// Pick a single dominant pattern to drive `suggested_action`. Order
/// matters: near-dup > temporal-log > type-confused > low-signal >
/// orphan (multi-member first, then singleton categories).
fn suggested_action_for(patterns: &[String]) -> (String, String) {
    for p in patterns {
        match p.as_str() {
            "near-dup" => {
                return (
                    "archive non-canonical, supersedes-link canonical".into(),
                    "near-dup: average vec_sim ≥ 0.85 across cluster edges".into(),
                );
            }
            "temporal-log" => {
                return (
                    "roll up into single log unit (manual)".into(),
                    "temporal-log: ≥50% of members carry -YYYY-MM-DD slug".into(),
                );
            }
            "type-confused" => {
                return (
                    "retype (manual)".into(),
                    "type-confused: ≥2 types and avg vec_sim ≥ 0.75".into(),
                );
            }
            "low-signal" => {
                return (
                    "archive".into(),
                    "low-signal: 0 marks, no inbound links, age > 90d".into(),
                );
            }
            "orphan" => {
                return (
                    "link or archive (manual)".into(),
                    "orphan: no outbound part_of / related_to edges".into(),
                );
            }
            _ => {}
        }
    }
    ("skip".into(), "no dominant pattern".into())
}

/// Short cluster id — first 8 chars of the canonical member uuid. Stable
/// across reruns as long as union-find roots remain deterministic
/// (they do: roots are the smallest index in each component).
fn cluster_id_for(canonical: &str) -> String {
    canonical.chars().take(8).collect()
}

/// Build a `ClusterMember` from in-memory signals.
fn member_from(s: &UnitSignals) -> ClusterMember {
    ClusterMember {
        id: s.id.clone(),
        slug: s.slug.clone(),
        unit_type: s.unit_type.clone(),
        age_days: s.age_days,
        marks: s.marks,
        content_preview: s.content_preview.clone(),
    }
}

/// Main entry point.
pub fn cluster(conn: &Connection, p: &ClusterParams) -> Result<ClusterReport> {
    let units = candidate_units(conn, p)?;
    let total_units = units.len();
    if total_units == 0 {
        return Ok(ClusterReport {
            summary: ClusterSummary {
                total_units: 0,
                cluster_count: 0,
                by_pattern: BTreeMap::new(),
            },
            clusters: Vec::new(),
        });
    }

    let signals = load_signals(conn, &units)?;
    let edges = build_edges(conn, &units, p)?;

    // Union-find over edge endpoints. The candidate index is the node
    // namespace; isolates remain in their own singleton component.
    let mut uf = UnionFind::new(units.len());
    for e in &edges {
        uf.union(e.from, e.to);
    }

    // Group node indices by root, plus per-component edge accumulator
    // for avg_vec_sim. Edges in different components are filtered out
    // implicitly (only same-root edges are accumulated).
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..units.len() {
        components.entry(uf.find(i)).or_default().push(i);
    }
    let mut edge_sums: HashMap<usize, (f64, usize)> = HashMap::new();
    for e in &edges {
        let r = uf.find(e.from);
        let entry = edge_sums.entry(r).or_insert((0.0, 0));
        entry.0 += e.vec_sim;
        entry.1 += 1;
    }

    // Emit clusters. Multi-member clusters below min_cluster_size are
    // dropped entirely. Singletons are emitted only when classify
    // returns non-empty.
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut by_pattern: BTreeMap<String, usize> = BTreeMap::new();

    for (root, idxs) in &components {
        let mut members: Vec<&UnitSignals> = idxs
            .iter()
            .filter_map(|&i| signals.get(&units[i].id))
            .collect();
        // Stable order: by slug if present, else by id, so output is
        // deterministic.
        members.sort_by(|a, b| {
            a.slug
                .as_deref()
                .unwrap_or("")
                .cmp(b.slug.as_deref().unwrap_or(""))
                .then_with(|| a.id.cmp(&b.id))
        });

        let avg_vec_sim = edge_sums
            .get(root)
            .map(|(sum, n)| if *n == 0 { 0.0 } else { sum / *n as f64 })
            .unwrap_or(0.0);

        let patterns = if members.len() >= p.min_cluster_size {
            classify_cluster(&members, avg_vec_sim)
        } else if members.len() == 1 {
            classify_singleton(members[0])
        } else {
            // 2..min_cluster_size — too small to classify, too big to
            // treat as singleton. Drop.
            Vec::new()
        };

        if patterns.is_empty() {
            continue;
        }

        for pat in &patterns {
            *by_pattern.entry(pat.clone()).or_insert(0) += 1;
        }

        let (suggested_action, reason) = suggested_action_for(&patterns);
        let canonical_id = members[0].id.clone();
        clusters.push(Cluster {
            cluster_id: cluster_id_for(&canonical_id),
            patterns,
            suggested_action,
            reason,
            avg_vec_sim,
            members: members.iter().map(|m| member_from(m)).collect(),
        });
    }

    // Larger clusters first; ties broken by cluster_id for determinism.
    clusters.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.cluster_id.cmp(&b.cluster_id))
    });

    Ok(ClusterReport {
        summary: ClusterSummary {
            total_units,
            cluster_count: clusters.len(),
            by_pattern,
        },
        clusters,
    })
}

/// JSON-only printer. Cluster is a tooling primitive; always-JSON keeps
/// the contract simple for the consolidation UI + downstream tasks.
pub fn print(report: &ClusterReport) {
    let json = serde_json::to_string_pretty(report).expect("serialize ClusterReport");
    println!("{json}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_date_stamp_matches() {
        assert!(slug_has_date_stamp("lotus-failure-2026-05-01"));
        assert!(slug_has_date_stamp("foo-2026-05-01-suffix"));
        assert!(slug_has_date_stamp("-2024-01-01"));
    }

    #[test]
    fn slug_date_stamp_rejects() {
        assert!(!slug_has_date_stamp("just-a-slug"));
        assert!(!slug_has_date_stamp("2026-05-01")); // missing leading '-'
        assert!(!slug_has_date_stamp("foo-26-05-01")); // 2-digit year
        assert!(!slug_has_date_stamp(""));
        assert!(!slug_has_date_stamp("short"));
    }

    #[test]
    fn union_find_components_basic() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);
        assert_eq!(uf.find(0), uf.find(2));
        assert_eq!(uf.find(3), uf.find(4));
        assert_ne!(uf.find(0), uf.find(3));
    }

    #[test]
    fn union_find_smaller_index_root() {
        let mut uf = UnionFind::new(3);
        uf.union(2, 0);
        // Smaller index (0) wins as canonical root.
        assert_eq!(uf.find(2), 0);
        assert_eq!(uf.find(0), 0);
    }

    #[test]
    fn suggested_action_priority() {
        let (a, _) = suggested_action_for(&["near-dup".into(), "type-confused".into()]);
        assert!(a.contains("archive non-canonical"));
        let (a, _) = suggested_action_for(&["temporal-log".into()]);
        assert!(a.contains("roll up"));
        let (a, _) = suggested_action_for(&["orphan".into()]);
        assert!(a.contains("link or archive"));
        let (a, _) = suggested_action_for(&[]);
        assert_eq!(a, "skip");
    }

    #[test]
    fn cluster_id_short() {
        let id = "019dbb6e-1887-7383-b2a8-de22b7a2eb56";
        assert_eq!(cluster_id_for(id), "019dbb6e");
        assert_eq!(cluster_id_for(id).len(), 8);
    }

    fn mock_signal(id: &str, ty: &str, slug: Option<&str>, age_days: i64) -> UnitSignals {
        UnitSignals {
            id: id.into(),
            slug: slug.map(String::from),
            unit_type: ty.into(),
            age_days,
            marks: 0,
            content_preview: String::new(),
            inbound_links: 0,
            has_part_of_or_related_out: false,
        }
    }

    #[test]
    fn classify_cluster_near_dup_fires_at_threshold() {
        let a = mock_signal("a", "fact", None, 1);
        let b = mock_signal("b", "fact", None, 1);
        let members = [&a, &b];
        // avg_vec_sim 0.85 exactly → near-dup.
        let pats = classify_cluster(&members, 0.85);
        assert!(pats.iter().any(|p| p == "near-dup"), "got {pats:?}");
        // 0.84 — strictly below cutoff → no near-dup.
        let pats = classify_cluster(&members, 0.84);
        assert!(!pats.iter().any(|p| p == "near-dup"), "got {pats:?}");
    }

    #[test]
    fn classify_cluster_type_confused_requires_two_types() {
        let a = mock_signal("a", "fact", None, 1);
        let b = mock_signal("b", "fact", None, 1);
        let pats = classify_cluster(&[&a, &b], 0.80);
        assert!(!pats.iter().any(|p| p == "type-confused"));

        let c = mock_signal("c", "idea", None, 1);
        let pats = classify_cluster(&[&a, &c], 0.80);
        assert!(pats.iter().any(|p| p == "type-confused"), "got {pats:?}");

        // Below avg_vec_sim 0.75 → no type-confused even with mixed types.
        let pats = classify_cluster(&[&a, &c], 0.70);
        assert!(!pats.iter().any(|p| p == "type-confused"));
    }

    #[test]
    fn classify_cluster_temporal_log_majority() {
        let a = mock_signal("a", "fact", Some("log-2026-05-01"), 1);
        let b = mock_signal("b", "fact", Some("log-2026-05-02"), 1);
        let c = mock_signal("c", "fact", Some("plain-slug"), 1);
        // 2/3 dated → ≥0.5 → temporal-log.
        let pats = classify_cluster(&[&a, &b, &c], 0.0);
        assert!(pats.iter().any(|p| p == "temporal-log"), "got {pats:?}");

        // 1/3 dated → < 0.5 → no temporal-log.
        let pats = classify_cluster(&[&a, &c, &c], 0.0);
        assert!(!pats.iter().any(|p| p == "temporal-log"));
    }

    #[test]
    fn classify_singleton_low_signal_age_gate() {
        let mut m = mock_signal("a", "fact", None, 91);
        let pats = classify_singleton(&m);
        // 0 marks, 0 inbound, age > 90 → low-signal. Also orphan (no out links).
        assert!(pats.iter().any(|p| p == "low-signal"));
        assert!(pats.iter().any(|p| p == "orphan"));

        // Inside the 90d window → no low-signal, still orphan.
        m.age_days = 30;
        let pats = classify_singleton(&m);
        assert!(!pats.iter().any(|p| p == "low-signal"));
        assert!(pats.iter().any(|p| p == "orphan"));

        // With outbound link → no orphan.
        m.has_part_of_or_related_out = true;
        let pats = classify_singleton(&m);
        assert!(!pats.iter().any(|p| p == "orphan"));
    }
}
