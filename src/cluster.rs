//! `simaris cluster` — store-wide redundancy survey.
//!
//! Walks a candidate set (one tag, one type, or `--all`), runs the
//! `similar` primitive per-unit to gather top-K neighbours, then unions
//! them into connected components. Each component is annotated with one
//! or more *patterns* describing why the cluster is interesting:
//!
//! - `near-dup` — average edge `vec_sim` ≥ 0.85 AND average edge
//!   `content_overlap` ≥ 0.30 (likely true duplicates).
//! - `related` — average edge `vec_sim` ≥ 0.85 but `content_overlap`
//!   below 0.30 (vector model fired on shared vocabulary; bodies cover
//!   distinct subjects). Surfaced separately so the consolidation UI
//!   doesn't push the user toward archiving complementary units.
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
//!           "age_days": <i64>, "marks": <i64>, "content_preview": "<≤80 chars>",
//!           "body_length": <usize>, "tag_count": <usize> },
//!         ...
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! `body_length` is the byte length of the member's `content` (matches the
//! size_guard thresholds and `len(simaris show <id> --json .unit.content)`
//! when content is ASCII; for multi-byte UTF-8 the byte count is the
//! authoritative measure since that's what `SIMARIS_WARN_BYTES` /
//! `SIMARIS_HARD_BYTES` gate on). `tag_count` is the number of tags on
//! the unit. Both are lifted once at `load_signals` time so downstream
//! tooling (pilot byte-reduction deltas, consolidation UI) doesn't have
//! to round-trip via `simaris show --json` per member.
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
/// Minimum average edge `content_overlap` (word-token Jaccard, frontmatter
/// stripped) to confirm a `near-dup` cluster. Below this the cluster is
/// labelled `related` instead — bge-m3 cosine fires on shared vocabulary
/// even when the actual content is distinct (task pbhm pilot finding:
/// allowedTools full-path vs compound-command lessons hit vec_sim 0.97
/// but only 0.17 word-token overlap). Default is conservative — any
/// overlap above ~30% is enough to keep the near-dup label.
pub const NEAR_DUP_MIN_CONTENT_OVERLAP: f64 = 0.30;
pub const TYPE_CONFUSED_AVG_VEC_SIM: f64 = 0.75;
pub const LOW_SIGNAL_MIN_AGE_DAYS: i64 = 90;
pub const TEMPORAL_LOG_MIN_FRACTION: f64 = 0.5;

/// Default cluster size cap before the split post-pass activates.
/// Clusters with more members than this get re-clustered using
/// `split_threshold` as a tighter edge cutoff to break shared-tag bleed.
/// `0` disables the post-pass entirely.
pub const DEFAULT_MAX_CLUSTER_SIZE: usize = 10;
/// Default edge-score cutoff used by the split post-pass. Sits well
/// above the main `threshold` (0.3) so only solid edges survive.
pub const DEFAULT_SPLIT_THRESHOLD: f64 = 0.55;

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
    /// Maximum members a cluster can carry before the split post-pass
    /// activates. `0` disables splitting. Defaults to
    /// `DEFAULT_MAX_CLUSTER_SIZE` (10) — large enough to leave genuine
    /// near-dup clusters alone, small enough to catch shared-tag bleed.
    pub max_cluster_size: usize,
    /// Edge-score cutoff used by the split post-pass — within an
    /// oversized cluster, only edges scoring ≥ this value are kept when
    /// re-running union-find on its members. Bridge edges driven by tag
    /// overlap alone fall below; genuine near-dup edges survive.
    pub split_threshold: f64,
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
            max_cluster_size: DEFAULT_MAX_CLUSTER_SIZE,
            split_threshold: DEFAULT_SPLIT_THRESHOLD,
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
    /// Byte length of the unit's `content` field. Mirrors what the
    /// `size_guard` thresholds (`SIMARIS_WARN_BYTES` /
    /// `SIMARIS_HARD_BYTES`) check at `add`/`edit` time, so callers can
    /// compute byte-reduction deltas without a per-member `simaris show
    /// --json` round-trip (task rmps — pilot smqs gap).
    pub body_length: usize,
    /// Tag count on the unit (length of the `tags` array). Useful for
    /// the consolidation UI to spot tag-bloat candidates and for pilots
    /// to track tag deltas alongside body deltas.
    pub tag_count: usize,
}

/// One cluster (component) in the output report.
#[derive(Debug, Serialize)]
pub struct Cluster {
    pub cluster_id: String,
    pub patterns: Vec<String>,
    pub suggested_action: String,
    pub reason: String,
    pub avg_vec_sim: f64,
    /// Average edge `content_overlap` (word-token Jaccard, frontmatter
    /// stripped). Mirrors `avg_vec_sim` but for the literal-text leg —
    /// drives the `near-dup` vs `related` split (task pbhm). 0.0 when
    /// the component carries no internal edges (singleton).
    pub avg_content_overlap: f64,
    pub members: Vec<ClusterMember>,
    /// When the split post-pass carved this cluster out of a larger
    /// parent (shared-tag bleed survey), records the parent cluster id.
    /// `None` for clusters that emerged unsplit from the main union-find.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_from: Option<String>,
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
    body_length: usize,
    tag_count: usize,
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
                body_length: u.content.len(),
                tag_count: u.tags.len(),
                inbound_links,
                has_part_of_or_related_out,
            },
        );
    }
    Ok(signals)
}

/// One edge in the similarity graph — directed (`from` ran similar()
/// and `to` is one of its hits) but treated as undirected by union-find.
/// `score` is the full weighted similarity (`α·vec_sim + β·tag_overlap
/// + γ·type_match`); the split post-pass uses it to drop bridge edges
/// driven by tag overlap alone within an oversized component.
/// `content_overlap` is the literal word-token Jaccard between bodies —
/// fed into the near-dup gate independently of `vec_sim` (task pbhm).
struct Edge {
    from: usize,
    to: usize,
    vec_sim: f64,
    content_overlap: f64,
    score: f64,
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
                    content_overlap: h.content_overlap,
                    score: h.score,
                });
            }
        }
    }
    Ok(edges)
}

/// Pattern classification for a multi-member cluster. Returns the
/// ordered pattern label list (priority order: near-dup OR related,
/// temporal-log, type-confused). A cluster may carry multiple labels.
///
/// `near-dup` requires BOTH a high vector-sim cluster AND high literal
/// content overlap. When vector sim is high but content overlap is low,
/// the cluster is downgraded to `related` — the bodies cover distinct
/// subjects even though the embedding model treats them as neighbours
/// (task pbhm).
fn classify_cluster(
    members: &[&UnitSignals],
    avg_vec_sim: f64,
    avg_content_overlap: f64,
) -> Vec<String> {
    let mut patterns: Vec<String> = Vec::new();

    if avg_vec_sim >= NEAR_DUP_AVG_VEC_SIM {
        if avg_content_overlap >= NEAR_DUP_MIN_CONTENT_OVERLAP {
            patterns.push("near-dup".into());
        } else {
            patterns.push("related".into());
        }
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
/// matters: near-dup > related > temporal-log > type-confused >
/// low-signal > orphan (multi-member first, then singleton categories).
fn suggested_action_for(patterns: &[String]) -> (String, String) {
    for p in patterns {
        match p.as_str() {
            "near-dup" => {
                return (
                    "archive non-canonical, supersedes-link canonical".into(),
                    "near-dup: avg vec_sim ≥ 0.85 AND avg content_overlap ≥ 0.30".into(),
                );
            }
            "related" => {
                return (
                    "link related_to (do NOT merge)".into(),
                    "related: avg vec_sim ≥ 0.85 but avg content_overlap < 0.30 — distinct content under shared vocabulary".into(),
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
            "shared-tag-bleed" => {
                return (
                    "review sub-cluster (manual)".into(),
                    "shared-tag-bleed: surfaced by split post-pass; no other pattern fired".into(),
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

/// Split-pass result: list of sub-component member-index groups (within
/// the original cluster's member slice) that cleared the higher edge
/// cutoff, plus the count of bleed members that fell out (members with
/// no surviving high-score edges to any sub-component).
struct SplitResult {
    sub_components: Vec<Vec<usize>>,
    dropped_count: usize,
}

/// Re-cluster the members of an oversized component using only edges
/// with `score ≥ split_threshold`. Returns the sub-component groups
/// (each a list of indices into `member_idxs`) that survived. Members
/// that end up isolated (no surviving high-score edges) are reported via
/// `dropped_count` — bleed members from shared-tag union, dropped from
/// the output.
fn split_oversized_cluster(
    member_idxs: &[usize],
    edges: &[Edge],
    uf_find_for: &dyn Fn(usize) -> usize,
    component_root: usize,
    split_threshold: f64,
    min_cluster_size: usize,
) -> SplitResult {
    // Build a local index: original candidate index -> position within
    // member_idxs. Sub union-find runs in member-local index space.
    let mut local: HashMap<usize, usize> = HashMap::with_capacity(member_idxs.len());
    for (i, &orig) in member_idxs.iter().enumerate() {
        local.insert(orig, i);
    }

    let mut sub_uf = UnionFind::new(member_idxs.len());
    let mut has_high_edge = vec![false; member_idxs.len()];
    for e in edges {
        // Only edges within this component, scoring above the cutoff.
        if uf_find_for(e.from) != component_root || uf_find_for(e.to) != component_root {
            continue;
        }
        if e.score < split_threshold {
            continue;
        }
        if let (Some(&li), Some(&lj)) = (local.get(&e.from), local.get(&e.to)) {
            sub_uf.union(li, lj);
            has_high_edge[li] = true;
            has_high_edge[lj] = true;
        }
    }

    // Group local indices by sub-root, but only for members that
    // actually participated in a surviving edge. Isolated members
    // (no high-score edge) are the bleed; drop them.
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut dropped_count = 0usize;
    for (li, &had_edge) in has_high_edge.iter().enumerate() {
        if !had_edge {
            dropped_count += 1;
            continue;
        }
        let r = sub_uf.find(li);
        groups.entry(r).or_default().push(li);
    }

    // Keep only sub-components that clear min_cluster_size — anything
    // smaller is a residual bridge fragment, treat as bleed.
    let mut sub_components: Vec<Vec<usize>> = Vec::new();
    for (_, g) in groups {
        if g.len() >= min_cluster_size {
            sub_components.push(g);
        } else {
            dropped_count += g.len();
        }
    }

    SplitResult {
        sub_components,
        dropped_count,
    }
}

/// Stable deterministic order: by slug if present, else by id.
/// Applied to every cluster (and split sub-cluster) so the canonical
/// member (used to derive `cluster_id`) and rendered member list are
/// reproducible across runs.
fn sort_members(mut members: Vec<&UnitSignals>) -> Vec<&UnitSignals> {
    members.sort_by(|a, b| {
        a.slug
            .as_deref()
            .unwrap_or("")
            .cmp(b.slug.as_deref().unwrap_or(""))
            .then_with(|| a.id.cmp(&b.id))
    });
    members
}

/// Build a `Cluster` from member signals + an avg_vec_sim already
/// computed by the caller. Returns `None` when classify yields no
/// patterns (multi-member below min_cluster_size, or singleton that
/// fits neither low-signal nor orphan). On success, updates the
/// `by_pattern` summary counter. `split_from` is threaded through so
/// the consumer can tell parent-derived sub-clusters apart.
///
/// Sub-clusters that emerged from the split post-pass always emit —
/// the split itself is the signal of a coherent sub-theme. If
/// classify yields no patterns (common under `--no-vec` where
/// `avg_vec_sim` is zero), they're labeled `shared-tag-bleed` so the
/// schema contract (every cluster carries ≥1 pattern) holds.
fn build_cluster(
    members: &[&UnitSignals],
    avg_vec_sim: f64,
    avg_content_overlap: f64,
    split_from: Option<String>,
    p: &ClusterParams,
    by_pattern: &mut BTreeMap<String, usize>,
) -> Option<Cluster> {
    let mut patterns = if members.len() >= p.min_cluster_size {
        classify_cluster(members, avg_vec_sim, avg_content_overlap)
    } else if members.len() == 1 {
        classify_singleton(members[0])
    } else {
        Vec::new()
    };

    if patterns.is_empty() {
        if split_from.is_some() {
            // Split-emerged sub-cluster with no detected pattern under
            // the default detectors. Tag it so it surfaces in the
            // output — the bleed-split itself is the signal.
            patterns.push("shared-tag-bleed".into());
        } else {
            return None;
        }
    }

    for pat in &patterns {
        *by_pattern.entry(pat.clone()).or_insert(0) += 1;
    }

    let (suggested_action, reason) = suggested_action_for(&patterns);
    let canonical_id = members[0].id.clone();
    Some(Cluster {
        cluster_id: cluster_id_for(&canonical_id),
        patterns,
        suggested_action,
        reason,
        avg_vec_sim,
        avg_content_overlap,
        members: members.iter().map(|m| member_from(m)).collect(),
        split_from,
    })
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
        body_length: s.body_length,
        tag_count: s.tag_count,
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
    // Per-component edge accumulators: (vec_sim_sum, content_overlap_sum, count).
    let mut edge_sums: HashMap<usize, (f64, f64, usize)> = HashMap::new();
    for e in &edges {
        let r = uf.find(e.from);
        let entry = edge_sums.entry(r).or_insert((0.0, 0.0, 0));
        entry.0 += e.vec_sim;
        entry.1 += e.content_overlap;
        entry.2 += 1;
    }

    // Emit clusters. Multi-member clusters below min_cluster_size are
    // dropped entirely. Singletons are emitted only when classify
    // returns non-empty.
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut by_pattern: BTreeMap<String, usize> = BTreeMap::new();

    // Snapshot uf.find() results so the closure passed to the split
    // pass can resolve roots without borrowing the mutable UnionFind.
    let roots_for_candidates: Vec<usize> = (0..units.len()).map(|i| uf.find(i)).collect();
    let root_of = |i: usize| -> usize { roots_for_candidates[i] };

    for (root, idxs) in &components {
        let members: Vec<&UnitSignals> = sort_members(
            idxs.iter()
                .filter_map(|&i| signals.get(&units[i].id))
                .collect(),
        );

        let (avg_vec_sim, avg_content_overlap) = edge_sums
            .get(root)
            .map(|(vsum, csum, n)| {
                if *n == 0 {
                    (0.0, 0.0)
                } else {
                    (vsum / *n as f64, csum / *n as f64)
                }
            })
            .unwrap_or((0.0, 0.0));

        // Oversized component? Run the split post-pass. If it produces
        // ≥2 sub-components, emit each as its own cluster tagged
        // `split_from = <parent cluster_id>` and skip the parent. Bleed
        // members (no surviving high-score edge) are dropped silently.
        // Bypass when: split disabled (max_cluster_size=0), under cap,
        // or singleton component (no edges to split on).
        if p.max_cluster_size > 0
            && members.len() > p.max_cluster_size
            && idxs.len() > 1
        {
            let parent_canonical = members[0].id.clone();
            let parent_cluster_id = cluster_id_for(&parent_canonical);
            let split = split_oversized_cluster(
                idxs,
                &edges,
                &root_of,
                *root,
                p.split_threshold,
                p.min_cluster_size,
            );

            if split.sub_components.len() >= 2 {
                if split.dropped_count > 0 {
                    eprintln!(
                        "note: split cluster {} ({} members) into {} sub-clusters; dropped {} bleed member(s) below score {}",
                        parent_cluster_id,
                        members.len(),
                        split.sub_components.len(),
                        split.dropped_count,
                        p.split_threshold,
                    );
                }
                for sub_local_indices in &split.sub_components {
                    // sub_local_indices are positions inside `idxs`; map
                    // back to candidate indices, then to signals.
                    let sub_members: Vec<&UnitSignals> = sort_members(
                        sub_local_indices
                            .iter()
                            .filter_map(|&li| {
                                let cand_idx = idxs[li];
                                signals.get(&units[cand_idx].id)
                            })
                            .collect(),
                    );

                    // Recompute avg_vec_sim + avg_content_overlap across
                    // edges internal to this sub-component, using the
                    // local index set as the membership filter. Only
                    // edges with both endpoints in the local set
                    // contribute.
                    let local_set: BTreeSet<usize> =
                        sub_local_indices.iter().map(|&li| idxs[li]).collect();
                    let (sub_vsum, sub_csum, sub_n) =
                        edges.iter().fold((0.0, 0.0, 0usize), |(vs, cs, n), e| {
                            if local_set.contains(&e.from) && local_set.contains(&e.to) {
                                (vs + e.vec_sim, cs + e.content_overlap, n + 1)
                            } else {
                                (vs, cs, n)
                            }
                        });
                    let (sub_avg_vec, sub_avg_content) = if sub_n == 0 {
                        (0.0, 0.0)
                    } else {
                        (sub_vsum / sub_n as f64, sub_csum / sub_n as f64)
                    };

                    if let Some(c) = build_cluster(
                        &sub_members,
                        sub_avg_vec,
                        sub_avg_content,
                        Some(parent_cluster_id.clone()),
                        p,
                        &mut by_pattern,
                    ) {
                        clusters.push(c);
                    }
                }
                continue; // parent cluster is replaced by its sub-clusters.
            }
            // Split didn't help (≤1 sub-component or all bleed). Fall
            // through and emit the parent unchanged so visibility is
            // preserved — the user can re-run with a tighter threshold.
        }

        if let Some(c) = build_cluster(
            &members,
            avg_vec_sim,
            avg_content_overlap,
            None,
            p,
            &mut by_pattern,
        ) {
            clusters.push(c);
        }
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
            body_length: 0,
            tag_count: 0,
            inbound_links: 0,
            has_part_of_or_related_out: false,
        }
    }

    #[test]
    fn classify_cluster_near_dup_fires_at_threshold() {
        let a = mock_signal("a", "fact", None, 1);
        let b = mock_signal("b", "fact", None, 1);
        let members = [&a, &b];
        // avg_vec_sim 0.85 + content overlap 0.30 exactly → near-dup.
        let pats = classify_cluster(&members, 0.85, 0.30);
        assert!(pats.iter().any(|p| p == "near-dup"), "got {pats:?}");
        // 0.84 vec — strictly below cutoff → neither near-dup nor related.
        let pats = classify_cluster(&members, 0.84, 0.50);
        assert!(!pats.iter().any(|p| p == "near-dup"), "got {pats:?}");
        assert!(!pats.iter().any(|p| p == "related"), "got {pats:?}");
    }

    #[test]
    fn classify_cluster_demotes_to_related_when_content_overlap_low() {
        // High vec_sim but low content_overlap → 'related', not 'near-dup'.
        // Mirrors task pbhm pilot: vec_sim 0.97 with word-jaccard 0.17.
        let a = mock_signal("a", "lesson", None, 1);
        let b = mock_signal("b", "lesson", None, 1);
        let pats = classify_cluster(&[&a, &b], 0.97, 0.17);
        assert!(
            pats.iter().any(|p| p == "related"),
            "expected 'related', got {pats:?}",
        );
        assert!(
            !pats.iter().any(|p| p == "near-dup"),
            "must NOT carry near-dup, got {pats:?}",
        );

        // Same vec but content overlap exactly at the cutoff → near-dup.
        let pats = classify_cluster(&[&a, &b], 0.97, NEAR_DUP_MIN_CONTENT_OVERLAP);
        assert!(pats.iter().any(|p| p == "near-dup"), "got {pats:?}");
        assert!(!pats.iter().any(|p| p == "related"), "got {pats:?}");
    }

    #[test]
    fn classify_cluster_type_confused_requires_two_types() {
        let a = mock_signal("a", "fact", None, 1);
        let b = mock_signal("b", "fact", None, 1);
        let pats = classify_cluster(&[&a, &b], 0.80, 0.0);
        assert!(!pats.iter().any(|p| p == "type-confused"));

        let c = mock_signal("c", "idea", None, 1);
        let pats = classify_cluster(&[&a, &c], 0.80, 0.0);
        assert!(pats.iter().any(|p| p == "type-confused"), "got {pats:?}");

        // Below avg_vec_sim 0.75 → no type-confused even with mixed types.
        let pats = classify_cluster(&[&a, &c], 0.70, 0.0);
        assert!(!pats.iter().any(|p| p == "type-confused"));
    }

    #[test]
    fn classify_cluster_temporal_log_majority() {
        let a = mock_signal("a", "fact", Some("log-2026-05-01"), 1);
        let b = mock_signal("b", "fact", Some("log-2026-05-02"), 1);
        let c = mock_signal("c", "fact", Some("plain-slug"), 1);
        // 2/3 dated → ≥0.5 → temporal-log.
        let pats = classify_cluster(&[&a, &b, &c], 0.0, 0.0);
        assert!(pats.iter().any(|p| p == "temporal-log"), "got {pats:?}");

        // 1/3 dated → < 0.5 → no temporal-log.
        let pats = classify_cluster(&[&a, &c, &c], 0.0, 0.0);
        assert!(!pats.iter().any(|p| p == "temporal-log"));
    }

    /// Helper: build edges with synthetic scores. `(from, to, score)`.
    /// vec_sim is set equal to score for simplicity — the split pass
    /// only reads `score`, not `vec_sim`, so this keeps the test focus
    /// on the cutoff behavior. content_overlap defaults to 0.0; the
    /// split pass doesn't read it.
    fn mock_edges(specs: &[(usize, usize, f64)]) -> Vec<Edge> {
        specs
            .iter()
            .map(|&(from, to, score)| Edge {
                from,
                to,
                vec_sim: score,
                content_overlap: 0.0,
                score,
            })
            .collect()
    }

    /// Split pass splits a triangle of two real sub-themes plus one
    /// bridge member: members {0,1,2} form sub-theme A (high score
    /// edges), members {3,4,5} form sub-theme B (high score edges),
    /// and member {6} only has a low-score edge to {0} (the bridge).
    /// Result: two sub-components, bridge member dropped.
    #[test]
    fn split_oversized_drops_bridge_member() {
        // Component members in candidate-index space: 0..=6.
        let member_idxs: Vec<usize> = (0..=6).collect();

        // High-score edges within each sub-theme + one weak bridge to
        // member 6 + one weak bridge between the sub-themes (member 2
        // ↔ member 3) — that bridge unified them in the parent
        // union-find but should NOT survive split_threshold=0.6.
        let edges = mock_edges(&[
            // Sub-theme A: 0-1-2 fully connected.
            (0, 1, 0.9),
            (1, 2, 0.85),
            (0, 2, 0.88),
            // Sub-theme B: 3-4-5 fully connected.
            (3, 4, 0.9),
            (4, 5, 0.87),
            (3, 5, 0.86),
            // Inter-theme bridge (low score — would be dropped).
            (2, 3, 0.4),
            // Bridge member 6 to sub-theme A via 0 — low score.
            (0, 6, 0.35),
        ]);

        // All candidates fall under the same component root for this
        // test — uf_find returns 0 for every input.
        let uf_find = |_i: usize| -> usize { 0 };
        let component_root = 0;

        let res = split_oversized_cluster(
            &member_idxs,
            &edges,
            &uf_find,
            component_root,
            0.6,
            2, // min_cluster_size
        );

        // Two sub-components survive (the two sub-themes), bridge
        // member 6 is dropped (no surviving high-score edge), and the
        // inter-theme bridge edge is dropped (score 0.4 < 0.6).
        assert_eq!(
            res.sub_components.len(),
            2,
            "expected 2 sub-components, got {:?}",
            res.sub_components
        );
        assert_eq!(res.dropped_count, 1, "expected 1 bleed member dropped");

        // Sub-components together cover indices 0..=5 (the bridge is
        // dropped). Local indices are positions within member_idxs.
        let mut covered: Vec<usize> = res
            .sub_components
            .iter()
            .flat_map(|sub| sub.iter().copied())
            .collect();
        covered.sort();
        assert_eq!(covered, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn split_oversized_below_min_cluster_size_counts_as_dropped() {
        // 4 members: {0,1} sub-theme A (size 2), {2} alone with a
        // surviving edge to itself? — can't have self-edge; instead
        // {2,3} sub-theme B (size 2). With min_cluster_size=3, both
        // sub-components fall under the cutoff → both dropped.
        let member_idxs: Vec<usize> = (0..=3).collect();
        let edges = mock_edges(&[(0, 1, 0.9), (2, 3, 0.9)]);
        let uf_find = |_i: usize| -> usize { 0 };

        let res = split_oversized_cluster(
            &member_idxs,
            &edges,
            &uf_find,
            0,
            0.5,
            3, // min_cluster_size
        );
        assert!(
            res.sub_components.is_empty(),
            "no sub-component should survive min_cluster_size=3"
        );
        assert_eq!(res.dropped_count, 4, "all 4 members should drop");
    }

    #[test]
    fn split_oversized_no_high_edges_drops_all() {
        // Every edge is below the cutoff — nothing survives.
        let member_idxs: Vec<usize> = (0..=3).collect();
        let edges = mock_edges(&[(0, 1, 0.3), (1, 2, 0.25), (2, 3, 0.2)]);
        let uf_find = |_i: usize| -> usize { 0 };

        let res = split_oversized_cluster(&member_idxs, &edges, &uf_find, 0, 0.5, 2);
        assert!(res.sub_components.is_empty());
        assert_eq!(res.dropped_count, 4);
    }

    #[test]
    fn split_oversized_ignores_edges_outside_component() {
        // Edge endpoints live outside this component → not seen.
        let member_idxs: Vec<usize> = vec![10, 11, 12, 13];
        // Edges in component root=0 (these); plus a stray edge for
        // candidate indices 99/100 in a different component.
        let edges = mock_edges(&[
            (10, 11, 0.9),
            (12, 13, 0.9),
            (99, 100, 0.95), // outside this component
        ]);
        // uf_find returns 0 only for indices 10..=13; others -> 7.
        let uf_find = |i: usize| -> usize {
            if (10..=13).contains(&i) { 0 } else { 7 }
        };

        let res = split_oversized_cluster(&member_idxs, &edges, &uf_find, 0, 0.5, 2);
        // Two sub-components survive — 10-11 and 12-13.
        assert_eq!(res.sub_components.len(), 2);
        assert_eq!(res.dropped_count, 0);
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
