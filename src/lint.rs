//! `simaris lint` — read-only audit for knowledge-store rot.
//!
//! Implements step 1 of the knowledge-system v2 design (slug
//! `simaris-knowledge-v2`). Surfaces four rot categories without ever
//! mutating the store. Always exits 0 — advisory only.
//!
//! Categories:
//! 1. PROCEDURE_NO_TRIGGER — `type=procedure` lacking a `trigger:` scalar in
//!    frontmatter AND lacking a `trigger:` (or `trigger::`) line in body.
//! 2. ORPHAN — unit with no outgoing `part_of` edge AND no slug.
//! 3. DUPE — same-type pair with identical headline OR Jaccard 3-shingle
//!    similarity > 0.85 over lowercased word tokens.
//! 4. DUAL_PARENT_DIVERGENCE — unit with 2+ outgoing `part_of` edges where
//!    the parent `updated` timestamps span more than 14 days.

use crate::db::{self, Unit};
use crate::display::derive_headline;
use crate::frontmatter;
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct LintReport {
    pub procedure_no_trigger: Vec<Finding>,
    pub orphan: Vec<Finding>,
    pub dupe: Vec<DupeFinding>,
    pub dual_parent_divergence: Vec<DivergenceFinding>,
    /// M1: near-duplicate tags grouped by normalized form. Phase-0
    /// side-finding showed 51% singleton tags + auto-link noise from
    /// fragmentation — surface variants so they can be merged later.
    pub tag_variant: Vec<TagVariantFinding>,
    /// M1: bulk tag entropy stats (no per-tag finding cost).
    pub tag_stats: TagStats,
    /// M1: per-aspect rollup of content-category findings (PNT, ORPHAN,
    /// DUPE, DUAL_PARENT_DIVERGENCE). Tag findings are global and
    /// excluded from rollup. Climbs `part_of` up to 5 hops looking for an
    /// ancestor aspect with a slug; falls back to "(unowned)".
    pub by_aspect: Vec<AspectRollup>,
}

impl LintReport {
    pub fn total(&self) -> usize {
        self.procedure_no_trigger.len()
            + self.orphan.len()
            + self.dupe.len()
            + self.dual_parent_divergence.len()
            + self.tag_variant.len()
    }

    /// Per-category counts as a `LintTotals` — used by snapshot history
    /// and CI regression compare. Mirrors `db::LintTotals`.
    pub fn totals(&self) -> crate::db::LintTotals {
        crate::db::LintTotals {
            procedure_no_trigger: self.procedure_no_trigger.len(),
            orphan: self.orphan.len(),
            dupe: self.dupe.len(),
            dual_parent_divergence: self.dual_parent_divergence.len(),
            tag_variant: self.tag_variant.len(),
            total: self.total(),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct TagStats {
    pub distinct: usize,
    pub total_uses: usize,
    pub singletons: usize,
    pub low_use: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct TagVariantFinding {
    pub canonical: String,
    pub variants: Vec<TagVariant>,
    pub total_uses: usize,
    pub reason: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TagVariant {
    pub tag: String,
    pub uses: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct AspectRollup {
    pub aspect_id: String,
    pub headline: String,
    pub slug: Option<String>,
    pub procedure_no_trigger: usize,
    pub orphan: usize,
    pub dupe: usize,
    pub dual_parent_divergence: usize,
    pub total: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct Finding {
    pub id: String,
    pub headline: String,
    pub unit_type: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DupeFinding {
    pub a_id: String,
    pub a_headline: String,
    pub b_id: String,
    pub b_headline: String,
    pub unit_type: String,
    pub similarity: f64,
    pub reason: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DivergenceFinding {
    pub id: String,
    pub headline: String,
    pub unit_type: String,
    pub parent_a_id: String,
    pub parent_a_updated: String,
    pub parent_b_id: String,
    pub parent_b_updated: String,
    pub gap_days: i64,
    pub reason: String,
}

const DUPE_SIMILARITY_THRESHOLD: f64 = 0.85;
const DIVERGENCE_GAP_DAYS: f64 = 14.0;

/// Run the full lint pass. Reads `units` once, then derives findings.
pub fn lint(conn: &Connection) -> Result<LintReport> {
    let units = db::list_units(conn, None, false)?;
    let procedure_no_trigger = check_procedure_no_trigger(&units);
    let orphan = check_orphan(conn, &units)?;
    let dupe = check_dupe(&units);
    let dual_parent_divergence = check_dual_parent_divergence(conn, &units)?;
    let (tag_variant, tag_stats) = check_tags(&units);
    let by_aspect = rollup_by_aspect(
        conn,
        &units,
        &procedure_no_trigger,
        &orphan,
        &dupe,
        &dual_parent_divergence,
    )?;
    Ok(LintReport {
        procedure_no_trigger,
        orphan,
        dupe,
        dual_parent_divergence,
        tag_variant,
        tag_stats,
        by_aspect,
    })
}

// --- PROCEDURE_NO_TRIGGER -------------------------------------------------

fn check_procedure_no_trigger(units: &[Unit]) -> Vec<Finding> {
    let mut out = Vec::new();
    for u in units {
        if u.unit_type != "procedure" {
            continue;
        }
        if has_trigger(&u.content) {
            continue;
        }
        out.push(Finding {
            id: u.id.clone(),
            headline: derive_headline(&u.content),
            unit_type: u.unit_type.clone(),
            reason: "procedure has no `trigger:` in frontmatter and no `trigger:` line in body"
                .to_string(),
        });
    }
    out
}

/// True if the unit declares a trigger anywhere — frontmatter scalar with
/// non-empty value, or a body line starting with `trigger:` / `trigger::`.
fn has_trigger(content: &str) -> bool {
    let parsed = frontmatter::parse(content);
    if let Some(fm) = &parsed.frontmatter {
        if let Some(map) = fm.as_mapping() {
            if let Some(v) = map.get(serde_yml::Value::String("trigger".to_string())) {
                if let Some(s) = v.as_str() {
                    if !s.trim().is_empty() {
                        return true;
                    }
                }
            }
        }
    }
    for line in parsed.body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("trigger:") {
            // Accept either `trigger:` or `trigger::` followed by something.
            let after = trimmed.trim_start_matches("trigger:").trim_start_matches(':');
            if !after.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

// --- ORPHAN ---------------------------------------------------------------

fn check_orphan(conn: &Connection, units: &[Unit]) -> Result<Vec<Finding>> {
    // Outgoing `part_of` edges = units that have a parent.
    let has_parent: HashSet<String> = {
        let mut stmt =
            conn.prepare("SELECT DISTINCT from_id FROM links WHERE relationship = 'part_of'")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?
    };

    // Units that own a slug.
    let has_slug: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT unit_id FROM slugs")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?
    };

    let mut out = Vec::new();
    for u in units {
        if has_parent.contains(&u.id) || has_slug.contains(&u.id) {
            continue;
        }
        out.push(Finding {
            id: u.id.clone(),
            headline: derive_headline(&u.content),
            unit_type: u.unit_type.clone(),
            reason: "no outgoing part_of link and no slug".to_string(),
        });
    }
    Ok(out)
}

// --- DUPE -----------------------------------------------------------------

fn check_dupe(units: &[Unit]) -> Vec<DupeFinding> {
    let mut findings = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    let mut by_type: HashMap<&str, Vec<&Unit>> = HashMap::new();
    for u in units {
        by_type.entry(u.unit_type.as_str()).or_default().push(u);
    }

    for (ty, group) in &by_type {
        // Pass 1 — identical-headline cluster (cheap, catches duplicate
        // factoids whose bodies diverge but identity line matches).
        let mut by_headline: HashMap<String, Vec<&Unit>> = HashMap::new();
        for u in group {
            let h = derive_headline(&u.content).trim().to_lowercase();
            if h.is_empty() {
                continue;
            }
            by_headline.entry(h).or_default().push(u);
        }
        for cluster in by_headline.values() {
            if cluster.len() < 2 {
                continue;
            }
            let head = cluster[0];
            for other in &cluster[1..] {
                let key = pair_key(&head.id, &other.id);
                if !seen_pairs.insert(key) {
                    continue;
                }
                findings.push(DupeFinding {
                    a_id: head.id.clone(),
                    a_headline: derive_headline(&head.content),
                    b_id: other.id.clone(),
                    b_headline: derive_headline(&other.content),
                    unit_type: (*ty).to_string(),
                    similarity: 1.0,
                    reason: "identical headline (same type)".to_string(),
                });
            }
        }

        // Pass 2 — Jaccard 3-shingles over full content. Cache per-unit
        // shingle sets, then compute pairwise. Skip pairs already flagged
        // by headline match.
        let shingles: Vec<HashSet<String>> = group.iter().map(|u| shingles_3(&u.content)).collect();
        let n = group.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let key = pair_key(&group[i].id, &group[j].id);
                if seen_pairs.contains(&key) {
                    continue;
                }
                let sim = jaccard(&shingles[i], &shingles[j]);
                if sim > DUPE_SIMILARITY_THRESHOLD {
                    seen_pairs.insert(key);
                    findings.push(DupeFinding {
                        a_id: group[i].id.clone(),
                        a_headline: derive_headline(&group[i].content),
                        b_id: group[j].id.clone(),
                        b_headline: derive_headline(&group[j].content),
                        unit_type: (*ty).to_string(),
                        similarity: sim,
                        reason: format!("jaccard 3-shingle = {sim:.3}"),
                    });
                }
            }
        }
    }

    // Stable order: highest similarity first, then by a_id.
    findings.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.a_id.cmp(&b.a_id))
    });
    findings
}

fn pair_key(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Tokenize on non-alphanumeric runs, lowercase. Apostrophes preserved so
/// "lotus's" stays a single token (matches `auto_link` style).
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// 3-grams of word tokens. Falls back to a 1-gram set when the unit has
/// fewer than 3 tokens — avoids returning an empty set that would Jaccard
/// to NaN.
fn shingles_3(content: &str) -> HashSet<String> {
    let toks = tokens(content);
    let mut set = HashSet::new();
    if toks.len() < 3 {
        for w in toks {
            set.insert(w);
        }
        return set;
    }
    for w in toks.windows(3) {
        set.insert(format!("{}|{}|{}", w[0], w[1], w[2]));
    }
    set
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let uni = a.union(b).count() as f64;
    if uni == 0.0 { 0.0 } else { inter / uni }
}

// --- DUAL_PARENT_DIVERGENCE ----------------------------------------------

fn check_dual_parent_divergence(
    conn: &Connection,
    units: &[Unit],
) -> Result<Vec<DivergenceFinding>> {
    let unit_by_id: HashMap<&str, &Unit> = units.iter().map(|u| (u.id.as_str(), u)).collect();

    // Pull (child, parent, parent_updated) ordered so each child's parents
    // come out sorted by updated ASC. Filter archived parents — a retired
    // parent shouldn't drive divergence noise.
    let mut stmt = conn.prepare(
        "SELECT l.from_id, l.to_id, p.updated
         FROM links l
         JOIN units p ON p.id = l.to_id
         WHERE l.relationship = 'part_of' AND p.archived = 0
         ORDER BY l.from_id, p.updated",
    )?;

    let mut child_parents: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (child, parent, updated) = row?;
        child_parents
            .entry(child)
            .or_default()
            .push((parent, updated));
    }

    let mut out = Vec::new();
    for (child, parents) in &child_parents {
        if parents.len() < 2 {
            continue;
        }
        // parents already sorted by updated ASC, so first = oldest, last = newest.
        let (pa_id, pa_upd) = &parents[0];
        let (pb_id, pb_upd) = parents.last().unwrap();
        let gap_days: f64 = conn.query_row(
            "SELECT julianday(?1) - julianday(?2)",
            params![pb_upd, pa_upd],
            |r| r.get(0),
        )?;
        if gap_days <= DIVERGENCE_GAP_DAYS {
            continue;
        }
        let Some(u) = unit_by_id.get(child.as_str()) else {
            // child archived — skip
            continue;
        };
        out.push(DivergenceFinding {
            id: child.clone(),
            headline: derive_headline(&u.content),
            unit_type: u.unit_type.clone(),
            parent_a_id: pa_id.clone(),
            parent_a_updated: pa_upd.clone(),
            parent_b_id: pb_id.clone(),
            parent_b_updated: pb_upd.clone(),
            gap_days: gap_days.round() as i64,
            reason: format!("parents differ by {gap_days:.0} days (>14)"),
        });
    }

    out.sort_by(|a, b| b.gap_days.cmp(&a.gap_days).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

// --- TAG_VARIANT + tag entropy stats ------------------------------------

const TAG_VARIANT_MIN_USES: usize = 3;

/// Normalize a tag for fragmentation detection. Lowercase, strip non-
/// alphanumeric characters, and drop a trailing `s` to fold trivial
/// plurals. Conservative on purpose — the goal is to surface obvious
/// near-duplicates (e.g. `procedure` vs `procedures`, `auto-link` vs
/// `autolink`), not to merge semantically related but distinct tags.
fn normalize_tag(tag: &str) -> String {
    let mut s: String = tag
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    if s.len() > 3 && s.ends_with('s') {
        s.pop();
    }
    s
}

fn check_tags(units: &[Unit]) -> (Vec<TagVariantFinding>, TagStats) {
    // Tally raw tag uses across live units.
    let mut uses: HashMap<String, usize> = HashMap::new();
    for u in units {
        for t in &u.tags {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                continue;
            }
            *uses.entry(trimmed.to_string()).or_insert(0) += 1;
        }
    }

    // Bucket by normalized form.
    let mut buckets: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for (tag, count) in &uses {
        let norm = normalize_tag(tag);
        if norm.is_empty() {
            continue;
        }
        buckets.entry(norm).or_default().push((tag.clone(), *count));
    }

    let mut findings = Vec::new();
    for (canonical, mut variants) in buckets {
        if variants.len() < 2 {
            continue;
        }
        let total_uses: usize = variants.iter().map(|(_, c)| *c).sum();
        if total_uses < TAG_VARIANT_MIN_USES {
            continue;
        }
        // Highest-use variant first — likely canonical winner.
        variants.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let variant_strs: Vec<String> = variants
            .iter()
            .map(|(t, c)| format!("{t}({c})"))
            .collect();
        findings.push(TagVariantFinding {
            canonical: canonical.clone(),
            variants: variants
                .into_iter()
                .map(|(tag, uses)| TagVariant { tag, uses })
                .collect(),
            total_uses,
            reason: format!(
                "{} variants share normalized form `{canonical}`: {}",
                variant_strs.len(),
                variant_strs.join(", ")
            ),
        });
    }

    findings.sort_by(|a, b| b.total_uses.cmp(&a.total_uses).then_with(|| a.canonical.cmp(&b.canonical)));

    let distinct = uses.len();
    let total_uses: usize = uses.values().sum();
    let singletons = uses.values().filter(|c| **c == 1).count();
    let low_use = uses.values().filter(|c| **c < 3).count();
    let stats = TagStats {
        distinct,
        total_uses,
        singletons,
        low_use,
    };

    (findings, stats)
}

// --- per-aspect rollup ---------------------------------------------------

const ROLLUP_MAX_DEPTH: usize = 5;
const UNOWNED_KEY: &str = "(unowned)";

fn rollup_by_aspect(
    conn: &Connection,
    units: &[Unit],
    pnt: &[Finding],
    orphan: &[Finding],
    dupe: &[DupeFinding],
    div: &[DivergenceFinding],
) -> Result<Vec<AspectRollup>> {
    // Map id -> Unit for parent lookup.
    let unit_by_id: HashMap<&str, &Unit> = units.iter().map(|u| (u.id.as_str(), u)).collect();

    // Map id -> first slug (deterministic — minimum slug string).
    let mut slug_by_id: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT slug, unit_id FROM slugs")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (slug, uid) = row?;
            slug_by_id
                .entry(uid)
                .and_modify(|s| {
                    if slug.as_str() < s.as_str() {
                        *s = slug.clone();
                    }
                })
                .or_insert(slug);
        }
    }

    // child -> [parent] via part_of.
    let mut parents: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT from_id, to_id FROM links WHERE relationship = 'part_of'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (child, parent) = row?;
            parents.entry(child).or_default().push(parent);
        }
    }

    // Memoized owner lookup: id -> Option<aspect_id>.
    let mut owner_cache: HashMap<String, Option<String>> = HashMap::new();
    let find_owner = |id: &str,
                          unit_by_id: &HashMap<&str, &Unit>,
                          parents: &HashMap<String, Vec<String>>,
                          slug_by_id: &HashMap<String, String>,
                          owner_cache: &mut HashMap<String, Option<String>>|
     -> Option<String> {
        if let Some(hit) = owner_cache.get(id) {
            return hit.clone();
        }
        let mut frontier: Vec<String> = vec![id.to_string()];
        let mut visited: HashSet<String> = HashSet::new();
        for _ in 0..=ROLLUP_MAX_DEPTH {
            let mut next: Vec<String> = Vec::new();
            for n in &frontier {
                if !visited.insert(n.clone()) {
                    continue;
                }
                if let Some(u) = unit_by_id.get(n.as_str()) {
                    if u.unit_type == "aspect" && slug_by_id.contains_key(n) {
                        owner_cache.insert(id.to_string(), Some(n.clone()));
                        return Some(n.clone());
                    }
                }
                if let Some(ps) = parents.get(n) {
                    for p in ps {
                        next.push(p.clone());
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        owner_cache.insert(id.to_string(), None);
        None
    };

    // Tally counts per aspect_id (or UNOWNED_KEY).
    let mut tally: HashMap<String, [usize; 4]> = HashMap::new();
    let bump = |tally: &mut HashMap<String, [usize; 4]>, key: String, idx: usize| {
        tally.entry(key).or_insert([0; 4])[idx] += 1;
    };
    for f in pnt {
        let key = find_owner(&f.id, &unit_by_id, &parents, &slug_by_id, &mut owner_cache)
            .unwrap_or_else(|| UNOWNED_KEY.to_string());
        bump(&mut tally, key, 0);
    }
    for f in orphan {
        let key = find_owner(&f.id, &unit_by_id, &parents, &slug_by_id, &mut owner_cache)
            .unwrap_or_else(|| UNOWNED_KEY.to_string());
        bump(&mut tally, key, 1);
    }
    // Dupe pairs: attribute to BOTH sides' owners (each side counts as 1).
    for f in dupe {
        for side in [&f.a_id, &f.b_id] {
            let key = find_owner(side, &unit_by_id, &parents, &slug_by_id, &mut owner_cache)
                .unwrap_or_else(|| UNOWNED_KEY.to_string());
            bump(&mut tally, key, 2);
        }
    }
    for f in div {
        let key = find_owner(&f.id, &unit_by_id, &parents, &slug_by_id, &mut owner_cache)
            .unwrap_or_else(|| UNOWNED_KEY.to_string());
        bump(&mut tally, key, 3);
    }

    let mut out: Vec<AspectRollup> = tally
        .into_iter()
        .map(|(key, c)| {
            let total = c[0] + c[1] + c[2] + c[3];
            let (headline, slug) = if key == UNOWNED_KEY {
                ("(no owning aspect)".to_string(), None)
            } else {
                let h = unit_by_id
                    .get(key.as_str())
                    .map(|u| derive_headline(&u.content))
                    .unwrap_or_else(|| key.clone());
                (h, slug_by_id.get(&key).cloned())
            };
            AspectRollup {
                aspect_id: key,
                headline,
                slug,
                procedure_no_trigger: c[0],
                orphan: c[1],
                dupe: c[2],
                dual_parent_divergence: c[3],
                total,
            }
        })
        .collect();

    out.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.aspect_id.cmp(&b.aspect_id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_identical_sets() {
        let a: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let b = a.clone();
        assert!((jaccard(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_disjoint() {
        let a: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(jaccard(&a, &b), 0.0);
    }

    #[test]
    fn shingles_short_falls_back_to_1grams() {
        let s = shingles_3("one two");
        assert_eq!(s.len(), 2);
        assert!(s.contains("one"));
        assert!(s.contains("two"));
    }

    #[test]
    fn shingles_three_word_window() {
        let s = shingles_3("alpha beta gamma delta");
        assert_eq!(s.len(), 2);
        assert!(s.contains("alpha|beta|gamma"));
        assert!(s.contains("beta|gamma|delta"));
    }

    #[test]
    fn has_trigger_frontmatter() {
        let content = "---\ntrigger: foo\ncheck: bar\n---\nbody";
        assert!(has_trigger(content));
    }

    #[test]
    fn has_trigger_empty_frontmatter_no_body() {
        let content = "---\ntrigger:\n---\nbody";
        assert!(!has_trigger(content));
    }

    #[test]
    fn has_trigger_body_line() {
        let content = "no fm\ntrigger: do thing\n";
        assert!(has_trigger(content));
    }

    #[test]
    fn has_trigger_body_double_colon() {
        let content = "no fm\ntrigger:: do thing\n";
        assert!(has_trigger(content));
    }

    #[test]
    fn has_trigger_missing() {
        let content = "no fm\nthis procedure has no trigger declared\n";
        assert!(!has_trigger(content));
    }
}
