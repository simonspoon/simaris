use crate::db;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::process::Command;

#[derive(Debug, Serialize)]
pub struct AskResult {
    pub query: String,
    pub units: Vec<MatchedUnit>,
    pub units_used: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugTrace>,
}

#[derive(Debug, Serialize)]
pub struct MatchedUnit {
    pub id: String,
    pub content: String,
    pub unit_type: String,
    pub tags: Vec<String>,
    pub source: String,
    pub is_direct_match: bool,
    pub links: Vec<LinkInfo>,
}

#[derive(Debug, Serialize)]
pub struct DebugTrace {
    pub fts_query: String,
    pub matches_per_query: HashMap<String, usize>,
    pub total_gathered: usize,
    pub filter_kept: usize,
    pub filter_total: usize,
    pub filter_fallback: bool,
    pub units_in_result: usize,
}

#[derive(Debug, Serialize)]
struct ContextUnit {
    id: String,
    content: String,
    unit_type: String,
    tags: Vec<String>,
    source: String,
    links_to: Vec<LinkInfo>,
    links_from: Vec<LinkInfo>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_direct_match: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkInfo {
    pub unit_id: String,
    pub relationship: String,
    pub title: String,
}

/// Extract first 80 chars of the first line as a preview title.
fn content_preview(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    first_line.chars().take(80).collect()
}

/// Main entry point: search the knowledge graph and optionally synthesize a response.
pub fn ask(
    conn: &Connection,
    query: &str,
    synthesize: bool,
    debug: bool,
    type_filter: Option<&str>,
) -> Result<AskResult> {
    // Phase 1: FTS5 search + 1-hop graph expansion
    let fts_query = sanitize_fts_query(query);
    let search_queries = vec![query.to_string()];
    let gather = search_and_expand(conn, &search_queries, type_filter)?;
    let gathered = gather.units;
    let matches_per_query = gather.matches_per_query;

    if debug {
        eprintln!("\u{250c}\u{2500} PHASE 1: FTS5 Search + Graph Expansion");
        eprintln!("\u{2502}  query: {:?}", query);
        eprintln!("\u{2502}  fts_query: {:?}", fts_query);
        for (sq, count) in &matches_per_query {
            let label = if *count == 1 { "match" } else { "matches" };
            eprintln!("\u{2502}  {:?} \u{2192} {} {}", sq, count, label);
        }
        eprintln!(
            "\u{2502}  deduplicated: {} unique units",
            gather.direct_count
        );
        eprintln!(
            "\u{2502}  1-hop expansion: +{} linked units \u{2192} {} total",
            gather.expansion_count,
            gathered.len()
        );
        eprintln!("\u{2502}");
    }

    if gathered.is_empty() {
        return Ok(AskResult {
            query: query.to_string(),
            units: vec![],
            units_used: vec![],
            response: None,
            debug: if debug {
                Some(DebugTrace {
                    fts_query,
                    matches_per_query,
                    total_gathered: 0,
                    filter_kept: 0,
                    filter_total: 0,
                    filter_fallback: false,
                    units_in_result: 0,
                })
            } else {
                None
            },
        });
    }

    // Phase 2: Haiku relevance filter
    let filter_total = gathered.len();
    let (filtered, filter_fallback) = filter_relevance(query, &gathered);
    let filter_kept = filtered.len();

    if debug {
        eprintln!("\u{251c}\u{2500} PHASE 2: Relevance Filter (haiku)");
        eprintln!("\u{2502}  input: {} units", filter_total);
        eprintln!("\u{2502}  kept: {} units", filter_kept);
        eprintln!("\u{2502}  fallback: {}", filter_fallback);
        eprintln!("\u{2502}");
    }

    // Build result units — only keep links pointing outside the result set
    let result_ids: HashSet<&String> = filtered.iter().map(|u| &u.id).collect();
    let units: Vec<MatchedUnit> = filtered
        .iter()
        .map(|u| {
            let mut links = Vec::new();
            links.extend(u.links_to.iter().cloned());
            links.extend(u.links_from.iter().cloned());
            links.retain(|l| !result_ids.contains(&l.unit_id));
            MatchedUnit {
                id: u.id.clone(),
                content: u.content.clone(),
                unit_type: u.unit_type.clone(),
                tags: u.tags.clone(),
                source: u.source.clone(),
                is_direct_match: u.is_direct_match,
                links,
            }
        })
        .collect();
    let units_used: Vec<String> = units.iter().map(|u| u.id.clone()).collect();
    let units_in_result = units.len();

    // Phase 3: Optional synthesis
    let response = if synthesize {
        if debug {
            eprintln!("\u{2514}\u{2500} PHASE 3: Synthesis (sonnet)");
            eprintln!("   units_used: {}", units_used.len());
        }
        Some(synthesize_response(query, &filtered)?)
    } else {
        None
    };

    Ok(AskResult {
        query: query.to_string(),
        units,
        units_used,
        response,
        debug: if debug {
            Some(DebugTrace {
                fts_query,
                matches_per_query,
                total_gathered: filter_total,
                filter_kept,
                filter_total,
                filter_fallback,
                units_in_result,
            })
        } else {
            None
        },
    })
}

/// Common English stop words that hurt FTS5 AND queries.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "do", "does", "for", "from", "had",
    "has", "have", "he", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "out", "she", "so", "some", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "to", "up", "us", "was", "we",
    "what", "when", "which", "who", "will", "with", "would", "you", "your",
];

/// Sanitize a query string for FTS5 by quoting each word and removing stop words.
fn sanitize_fts_query(query: &str) -> String {
    let query = query.replace('-', " ");
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            // Strip characters that are FTS5 operators/syntax
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            cleaned.to_lowercase()
        })
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(&w.as_str()))
        .map(|w| format!("\"{}\"", w))
        .collect();

    if terms.is_empty() {
        // Fall back to OR of all original words if stop-word removal ate everything
        return query
            .split_whitespace()
            .map(|word| {
                let cleaned: String = word
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                cleaned.to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"", w))
            .collect::<Vec<_>>()
            .join(" OR ");
    }

    terms.join(" OR ")
}

struct GatherResult {
    units: Vec<ContextUnit>,
    matches_per_query: HashMap<String, usize>,
    direct_count: usize,
    expansion_count: usize,
}

/// FTS5 search using query terms + 1-hop link expansion.
fn search_and_expand(
    conn: &Connection,
    search_queries: &[String],
    type_filter: Option<&str>,
) -> Result<GatherResult> {
    // Run each search query and collect unique matches
    let mut seen_ids = std::collections::HashSet::new();
    let mut all_matches = vec![];
    let mut matches_per_query = HashMap::new();

    for sq in search_queries {
        let fts_query = sanitize_fts_query(sq);
        if fts_query.is_empty() {
            matches_per_query.insert(sq.clone(), 0);
            continue;
        }
        let results = db::search_units(conn, &fts_query, type_filter).unwrap_or_default();
        let mut count = 0;
        for unit in results {
            if seen_ids.insert(unit.id.clone()) {
                all_matches.push(unit);
                count += 1;
            }
        }
        matches_per_query.insert(sq.clone(), count);
    }

    let matches: Vec<_> = all_matches.into_iter().take(15).collect();
    let direct_count = matches.len();

    let mut units_by_id: HashMap<String, ContextUnit> = HashMap::new();

    for unit in &matches {
        let linked = db::get_linked_unit_ids(conn, &unit.id)?;
        let mut links_to = vec![];
        let mut links_from = vec![];

        for (linked_id, relationship, direction) in &linked {
            let title = db::get_unit(conn, linked_id)
                .map(|u| content_preview(&u.content))
                .unwrap_or_default();
            match direction.as_str() {
                "outgoing" => links_to.push(LinkInfo {
                    unit_id: linked_id.clone(),
                    relationship: relationship.clone(),
                    title,
                }),
                "incoming" => links_from.push(LinkInfo {
                    unit_id: linked_id.clone(),
                    relationship: relationship.clone(),
                    title,
                }),
                _ => {}
            }
        }

        units_by_id.insert(
            unit.id.clone(),
            ContextUnit {
                id: unit.id.clone(),
                content: unit.content.clone(),
                unit_type: unit.unit_type.clone(),
                tags: unit.tags.clone(),
                source: unit.source.clone(),
                links_to,
                links_from,
                is_direct_match: true,
            },
        );

        // Fetch 1-hop linked units
        for (linked_id, _relationship, _direction) in &linked {
            if units_by_id.contains_key(linked_id) {
                continue;
            }
            if let Ok(linked_unit) = db::get_unit(conn, linked_id) {
                let linked_links = db::get_linked_unit_ids(conn, linked_id)?;
                let mut lt = vec![];
                let mut lf = vec![];
                for (lid, rel, dir) in &linked_links {
                    let title = db::get_unit(conn, lid)
                        .map(|u| content_preview(&u.content))
                        .unwrap_or_default();
                    match dir.as_str() {
                        "outgoing" => lt.push(LinkInfo {
                            unit_id: lid.clone(),
                            relationship: rel.clone(),
                            title,
                        }),
                        "incoming" => lf.push(LinkInfo {
                            unit_id: lid.clone(),
                            relationship: rel.clone(),
                            title,
                        }),
                        _ => {}
                    }
                }
                units_by_id.insert(
                    linked_id.clone(),
                    ContextUnit {
                        id: linked_unit.id,
                        content: linked_unit.content.clone(),
                        unit_type: linked_unit.unit_type.clone(),
                        tags: linked_unit.tags.clone(),
                        source: linked_unit.source.clone(),
                        links_to: lt,
                        links_from: lf,
                        is_direct_match: false,
                    },
                );
            }
        }
    }

    let mut result: Vec<ContextUnit> = units_by_id.into_values().collect();
    // Sort direct matches first, then by ID for stability
    result.sort_by(|a, b| {
        b.is_direct_match
            .cmp(&a.is_direct_match)
            .then(a.id.cmp(&b.id))
    });
    let expansion_count = result.len().saturating_sub(direct_count);
    Ok(GatherResult {
        units: result,
        matches_per_query,
        direct_count,
        expansion_count,
    })
}

/// Single Haiku call to filter gathered units by relevance to the query.
/// Returns (filtered_units, fallback_used). On any failure, returns all units unfiltered.
fn filter_relevance<'a>(query: &str, gathered: &'a [ContextUnit]) -> (Vec<&'a ContextUnit>, bool) {
    let mut summaries = String::new();
    for unit in gathered {
        let preview: String = unit.content.chars().take(150).collect();
        let tags_str = if unit.tags.is_empty() {
            String::new()
        } else {
            format!(" tags=[{}]", unit.tags.join(", "))
        };
        summaries.push_str(&format!(
            "- id={} type={}{}: {}\n",
            unit.id, unit.unit_type, tags_str, preview
        ));
    }

    let prompt = format!(
        r#"You are a relevance filter. Given a query and a list of knowledge units, return ONLY the IDs of units relevant to the query.

Query: {query}

Units:
{summaries}
Return ONLY JSON (no markdown, no code fences):
{{"relevant_ids": [1, 3, 5]}}"#
    );

    let output = match Command::new("claude")
        .args(["-p", "--model", "haiku", &prompt])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (gathered.iter().collect(), true),
    };

    if !output.status.success() {
        return (gathered.iter().collect(), true);
    }

    let response = String::from_utf8_lossy(&output.stdout);
    let response = response.trim();

    // Strip markdown code fences if present
    let json_str = response
        .strip_prefix("```json")
        .or_else(|| response.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(response);

    #[derive(Deserialize)]
    struct FilterResponse {
        relevant_ids: Vec<String>,
    }

    let parsed: FilterResponse = match serde_json::from_str(json_str) {
        Ok(r) => r,
        Err(_) => return (gathered.iter().collect(), true),
    };

    let relevant_set: std::collections::HashSet<String> = parsed.relevant_ids.into_iter().collect();

    let filtered: Vec<&ContextUnit> = gathered
        .iter()
        .filter(|u| relevant_set.contains(&u.id))
        .collect();

    // If filter returned nothing relevant, fall back to all
    if filtered.is_empty() {
        return (gathered.iter().collect(), true);
    }

    (filtered, false)
}

fn model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "sonnet".to_string())
}

/// Synthesize a response from gathered units using the LLM.
fn synthesize_response(query: &str, units: &[&ContextUnit]) -> Result<String> {
    let mut units_text = String::new();
    for unit in units {
        units_text.push_str(&format!(
            "[{}] type={} source={} tags={}\n{}\n---\n",
            unit.id,
            unit.unit_type,
            unit.source,
            unit.tags.join(", "),
            unit.content
        ));
    }

    let prompt = format!(
        r#"You are a knowledge RETRIEVAL system. You return relevant knowledge — nothing else.

Rules:
- Return ONLY knowledge relevant to the context below
- Do NOT act on the query — you are not doing the work
- Do NOT plan, execute, ask questions, or offer to help
- Do NOT say "I can help with that" or "Here's how to do it"
- Format: concise, dense, factual — procedures as steps, facts as statements
- Include all relevant detail from the knowledge units
- If knowledge is insufficient, state what's missing
- No preamble — just the knowledge

Context: {query}

Knowledge units:
{units_text}"#
    );

    let output = Command::new("claude")
        .args(["-p", "--model", &model(), &prompt])
        .output()
        .context("Failed to run claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed during synthesis: {stderr}");
    }

    let response = String::from_utf8_lossy(&output.stdout);
    Ok(response.trim().to_string())
}
