use crate::db;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Serialize)]
pub struct AskResult {
    pub query: String,
    pub response: String,
    pub units_used: Vec<i64>,
}

#[derive(Debug, Serialize)]
struct ContextUnit {
    id: i64,
    content: String,
    unit_type: String,
    tags: Vec<String>,
    source: String,
    links_to: Vec<LinkInfo>,
    links_from: Vec<LinkInfo>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_direct_match: bool,
}

#[derive(Debug, Serialize)]
struct LinkInfo {
    unit_id: i64,
    relationship: String,
}

#[derive(Debug, Deserialize)]
struct SteeringResponse {
    explore: Vec<i64>,
    #[serde(default = "default_true")]
    sufficient: bool,
}

fn default_true() -> bool {
    true
}

fn model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "sonnet".to_string())
}

/// Main entry point: search the knowledge graph and synthesize a response.
pub fn ask(conn: &Connection, query: &str) -> Result<AskResult> {
    // Phase 1: gather initial matches + 1-hop links
    let mut gathered = gather_initial(conn, query)?;

    if gathered.is_empty() {
        return Ok(AskResult {
            query: query.to_string(),
            response: "No knowledge found for that query.".to_string(),
            units_used: vec![],
        });
    }

    // Phase 2: LLM steering — ask which units need deeper exploration
    let steering = steer(query, &gathered)?;

    // Phase 3: fetch additional units if steering says more is needed
    if !steering.sufficient && !steering.explore.is_empty() {
        gather_more(conn, &steering.explore, &mut gathered)?;
    }

    // Phase 4: synthesize a response from all gathered units
    let units_used: Vec<i64> = gathered.iter().map(|u| u.id).collect();
    let response = synthesize(query, &gathered)?;

    Ok(AskResult {
        query: query.to_string(),
        response,
        units_used,
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
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            // Strip characters that are FTS5 operators/syntax
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
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
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
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

/// Phase 1: FTS5 search + 1-hop link expansion.
fn gather_initial(conn: &Connection, query: &str) -> Result<Vec<ContextUnit>> {
    let fts_query = sanitize_fts_query(query);
    let matches = if fts_query.is_empty() {
        vec![]
    } else {
        db::search_units(conn, &fts_query).unwrap_or_default()
    };
    let matches: Vec<_> = matches.into_iter().take(10).collect();

    let mut units_by_id: HashMap<i64, ContextUnit> = HashMap::new();

    for unit in &matches {
        let linked = db::get_linked_unit_ids(conn, unit.id)?;
        let mut links_to = vec![];
        let mut links_from = vec![];

        for (linked_id, relationship, direction) in &linked {
            match direction.as_str() {
                "outgoing" => links_to.push(LinkInfo {
                    unit_id: *linked_id,
                    relationship: relationship.clone(),
                }),
                "incoming" => links_from.push(LinkInfo {
                    unit_id: *linked_id,
                    relationship: relationship.clone(),
                }),
                _ => {}
            }
        }

        units_by_id.insert(
            unit.id,
            ContextUnit {
                id: unit.id,
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
            if let Ok(linked_unit) = db::get_unit(conn, *linked_id) {
                let linked_links = db::get_linked_unit_ids(conn, *linked_id)?;
                let mut lt = vec![];
                let mut lf = vec![];
                for (lid, rel, dir) in &linked_links {
                    match dir.as_str() {
                        "outgoing" => lt.push(LinkInfo {
                            unit_id: *lid,
                            relationship: rel.clone(),
                        }),
                        "incoming" => lf.push(LinkInfo {
                            unit_id: *lid,
                            relationship: rel.clone(),
                        }),
                        _ => {}
                    }
                }
                units_by_id.insert(
                    *linked_id,
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
    Ok(result)
}

/// Phase 2: Ask the LLM which units need deeper exploration.
fn steer(query: &str, gathered: &[ContextUnit]) -> Result<SteeringResponse> {
    let mut units_summary = String::new();
    for unit in gathered {
        let preview: String = unit.content.chars().take(100).collect();
        let link_ids: Vec<String> = unit
            .links_to
            .iter()
            .map(|l| format!("{} ({})", l.unit_id, l.relationship))
            .chain(
                unit.links_from
                    .iter()
                    .map(|l| format!("{} ({})", l.unit_id, l.relationship)),
            )
            .collect();
        units_summary.push_str(&format!(
            "[{}] ({}) {}... links: [{}]\n",
            unit.id,
            unit.unit_type,
            preview,
            link_ids.join(", ")
        ));
    }

    let prompt = format!(
        r#"You are a knowledge graph navigator. Given a query and retrieved knowledge units, decide if more exploration is needed.

Query: {query}

Retrieved units:
{units_summary}
Return ONLY JSON (no markdown):
{{
  "explore": [list of unit IDs that need deeper exploration],
  "sufficient": true/false
}}

If the retrieved units contain enough information, set sufficient=true and explore=[]."#
    );

    let output = Command::new("claude")
        .args(["-p", "--model", &model(), &prompt])
        .output()
        .context("Failed to run claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed during steering: {stderr}");
    }

    let response = String::from_utf8_lossy(&output.stdout);
    let response = response.trim();

    // Strip markdown code fences if present
    let json_str = response
        .strip_prefix("```json")
        .or_else(|| response.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(response);

    let result: SteeringResponse = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse steering response: {json_str}"))?;

    Ok(result)
}

/// Phase 3: Fetch additional units requested by steering.
fn gather_more(
    conn: &Connection,
    explore_ids: &[i64],
    gathered: &mut Vec<ContextUnit>,
) -> Result<()> {
    let existing_ids: std::collections::HashSet<i64> = gathered.iter().map(|u| u.id).collect();

    for id in explore_ids {
        if existing_ids.contains(id) {
            continue;
        }
        if let Ok(unit) = db::get_unit(conn, *id) {
            let linked = db::get_linked_unit_ids(conn, *id)?;
            let mut links_to = vec![];
            let mut links_from = vec![];

            for (linked_id, relationship, direction) in &linked {
                match direction.as_str() {
                    "outgoing" => links_to.push(LinkInfo {
                        unit_id: *linked_id,
                        relationship: relationship.clone(),
                    }),
                    "incoming" => links_from.push(LinkInfo {
                        unit_id: *linked_id,
                        relationship: relationship.clone(),
                    }),
                    _ => {}
                }
            }

            gathered.push(ContextUnit {
                id: unit.id,
                content: unit.content.clone(),
                unit_type: unit.unit_type.clone(),
                tags: unit.tags.clone(),
                source: unit.source.clone(),
                links_to,
                links_from,
                is_direct_match: false,
            });

            // Also fetch the linked units of explored units (1 more hop)
            for (linked_id, _rel, _dir) in &linked {
                let all_ids: std::collections::HashSet<i64> =
                    gathered.iter().map(|u| u.id).collect();
                if all_ids.contains(linked_id) {
                    continue;
                }
                if let Ok(linked_unit) = db::get_unit(conn, *linked_id) {
                    let ll = db::get_linked_unit_ids(conn, *linked_id)?;
                    let mut lt = vec![];
                    let mut lf = vec![];
                    for (lid, rel, dir) in &ll {
                        match dir.as_str() {
                            "outgoing" => lt.push(LinkInfo {
                                unit_id: *lid,
                                relationship: rel.clone(),
                            }),
                            "incoming" => lf.push(LinkInfo {
                                unit_id: *lid,
                                relationship: rel.clone(),
                            }),
                            _ => {}
                        }
                    }
                    gathered.push(ContextUnit {
                        id: linked_unit.id,
                        content: linked_unit.content.clone(),
                        unit_type: linked_unit.unit_type.clone(),
                        tags: linked_unit.tags.clone(),
                        source: linked_unit.source.clone(),
                        links_to: lt,
                        links_from: lf,
                        is_direct_match: false,
                    });
                }
            }
        }
    }

    Ok(())
}

/// Phase 4: Synthesize a response from all gathered units.
fn synthesize(query: &str, units: &[ContextUnit]) -> Result<String> {
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
        r#"You are a knowledge system. Using ONLY the knowledge units below, respond to the query.

Rules:
- Be concise but include all relevant detail
- Don't summarize -- reconstruct from the knowledge
- Match format to intent: steps for how-to, facts for what-is, lists for what-are
- If the knowledge is insufficient, say what's missing
- No preamble, no "Based on the knowledge..." -- just answer

Query: {query}

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
