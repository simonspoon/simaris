//! Write-time tag policy. Mirrors `size_guard` in shape.
//!
//! Two responsibilities:
//! 1. `normalize_tags`: lowercase + trim + dedupe the incoming list so we
//!    don't proliferate case/whitespace variants (`Aspect` vs `aspect`).
//! 2. `check_tags`: hard-reject obvious noise (`task:`, `phase-X`, single
//!    chars, version tags, UUID fragments) and warn on tags that no live
//!    unit currently carries (likely typos or singleton noise) — surface
//!    closest-neighbor suggestions to nudge consolidation.
//!
//! Both behaviors can be overridden by `--force`. Driven by limbo task
//! `yvck` (simaris tag taxonomy cleanup, 2026-05-15).
//!
//! Warnings cite the `tag-taxonomy` slug — bind it via `simaris slug set`.
use anyhow::{Result, bail};
use rusqlite::Connection;

const NOISE_PREFIXES: &[&str] = &[
    "task:",
    "item:",
    "event:",
    "story:",
    "order:",
    "supersedes:",
    "target:",
];

const CITE_SLUG: &str = "tag-taxonomy";

/// Lowercase, trim, dedupe (stable order). Returned to the caller for the
/// actual write so storage stays canonical.
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in tags {
        let n = t.trim().to_lowercase();
        if n.is_empty() {
            continue;
        }
        if seen.insert(n.clone()) {
            out.push(n);
        }
    }
    out
}

/// Inspect normalized tags. Returns `Ok(())` to allow the write, prints
/// stderr warnings for novel tags, returns `Err` (or warns under `--force`)
/// when noise patterns are detected.
pub fn check_tags(conn: &Connection, tags: &[String], force: bool) -> Result<()> {
    let mut rejected: Vec<(String, &'static str)> = Vec::new();
    let mut novel: Vec<String> = Vec::new();

    for t in tags {
        if let Some(reason) = classify_noise(t) {
            rejected.push((t.clone(), reason));
            continue;
        }
        if !tag_exists(conn, t)? {
            novel.push(t.clone());
        }
    }

    if !rejected.is_empty() {
        let summary: Vec<String> = rejected
            .iter()
            .map(|(t, r)| format!("`{t}` ({r})"))
            .collect();
        if force {
            eprintln!(
                "simaris: warning — {} noise tag(s) kept under --force: {}; see slug `{CITE_SLUG}`",
                rejected.len(),
                summary.join(", ")
            );
        } else {
            bail!(
                "{} noise tag(s) rejected: {}. Re-run with --force to override, \
                 or rewrite using domain-meaningful labels (see slug `{CITE_SLUG}`)",
                rejected.len(),
                summary.join(", ")
            );
        }
    }

    for t in &novel {
        let suggestions = suggest_neighbors(conn, t)?;
        if suggestions.is_empty() {
            eprintln!(
                "simaris: warning — tag `{t}` is novel (no other live unit carries it); \
                 see slug `{CITE_SLUG}`"
            );
        } else {
            let s = suggestions
                .iter()
                .map(|(t, n)| format!("{t} ({n})"))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "simaris: warning — tag `{t}` is novel; did you mean: {s}? \
                 (slug `{CITE_SLUG}`)"
            );
        }
    }

    Ok(())
}

fn classify_noise(t: &str) -> Option<&'static str> {
    if t.is_empty() {
        return Some("empty");
    }
    if t.chars().count() == 1 {
        return Some("single-char");
    }
    for p in NOISE_PREFIXES {
        if t.starts_with(p) {
            return Some("namespace-prefix");
        }
    }
    if t.starts_with("phase-") {
        return Some("phase-X");
    }
    if let Some(rest) = t.strip_prefix("gate-")
        && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        && !rest.is_empty()
    {
        return Some("gate-N");
    }
    if let Some(rest) = t.strip_prefix("priority-")
        && rest.chars().all(|c| c.is_ascii_digit())
        && !rest.is_empty()
    {
        return Some("priority-N");
    }
    if let Some(rest) = t.strip_prefix("story-")
        && rest.chars().all(|c| c.is_ascii_digit())
        && !rest.is_empty()
    {
        return Some("story-N");
    }
    if t.len() > 1 && t.starts_with('v') {
        let rest = &t[1..];
        if rest.chars().all(|c| c.is_ascii_digit() || c == '.') && !rest.is_empty() {
            return Some("version-tag");
        }
    }
    if t.len() >= 8 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some("hex-uuid-fragment");
    }
    None
}

fn tag_exists(conn: &Connection, tag: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM units u, json_each(u.tags) je
         WHERE u.archived = 0 AND lower(je.value) = ?1",
        [tag],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn suggest_neighbors(conn: &Connection, tag: &str) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT lower(je.value) AS t, COUNT(*) AS n
           FROM units u, json_each(u.tags) je
          WHERE u.archived = 0
          GROUP BY t",
    )?;
    let all: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let prefix3: String = tag.chars().take(3).collect();
    let mut scored: Vec<(String, i64, usize)> = Vec::new();
    for (cand, n) in all {
        let d = levenshtein(tag, &cand);
        let cand_prefix3: String = cand.chars().take(3).collect();
        let near = d <= 2
            || (!prefix3.is_empty() && cand.starts_with(&prefix3))
            || (!cand_prefix3.is_empty() && tag.starts_with(&cand_prefix3));
        if near {
            scored.push((cand, n, d));
        }
    }
    scored.sort_by(|a, b| a.2.cmp(&b.2).then(b.1.cmp(&a.1)));
    Ok(scored.into_iter().take(3).map(|(c, n, _)| (c, n)).collect())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (la, lb) = (a.len(), b.len());
    if la == 0 {
        return lb;
    }
    if lb == 0 {
        return la;
    }
    let mut prev: Vec<usize> = (0..=lb).collect();
    let mut curr = vec![0usize; lb + 1];
    for i in 1..=la {
        curr[0] = i;
        for j in 1..=lb {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[lb]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_pass(t: &str) {
        assert_eq!(classify_noise(t), None, "expected `{t}` to pass");
    }

    #[test]
    fn normalize_lowercases_trims_dedupes() {
        let input = vec![
            "  Aspect ".to_string(),
            "aspect".to_string(),
            "BANSHEE".to_string(),
            "".to_string(),
            "banshee".to_string(),
        ];
        let out = normalize_tags(&input);
        assert_eq!(out, vec!["aspect".to_string(), "banshee".to_string()]);
    }

    #[test]
    fn classify_noise_catches_obvious() {
        assert_eq!(classify_noise("task:yvck"), Some("namespace-prefix"));
        assert_eq!(classify_noise("item:a"), Some("namespace-prefix"));
        assert_eq!(classify_noise("event:pretooluse"), Some("namespace-prefix"));
        assert_eq!(classify_noise("phase-1"), Some("phase-X"));
        assert_eq!(classify_noise("gate-2.5"), Some("gate-N"));
        assert_eq!(classify_noise("priority-3"), Some("priority-N"));
        assert_eq!(classify_noise("v02"), Some("version-tag"));
        classify_pass("rust");
        classify_pass("simaris");
        classify_pass("aspect-v2");
    }

    #[test]
    fn classify_real_2char_tags_pass() {
        for t in &["pm", "ui", "ci", "qa", "ux", "go", "pr", "jq", "gh"] {
            assert_eq!(classify_noise(t), None, "expected `{t}` to pass");
        }
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("aspect", "aspects"), 1);
        assert_eq!(levenshtein("aspect", "aspect"), 0);
        assert_eq!(levenshtein("rust", "rusty"), 1);
        assert_eq!(levenshtein("cat", "dog"), 3);
    }
}
