//! `GET /api/search` — units listing + search.
//!
//! Query string:
//! - `q` — search query. Empty/missing → list view via `simaris list --json`.
//! - `type` — filter to one of the seven knowledge types.
//! - `include_archived` — `1`/`true` to include archived units.
//!
//! Response shape (normalized across both code paths):
//! ```json
//! { "kind": "ask"|"list", "query": "...", "units": [
//!     { "id", "type", "tags", "source", "snippet",
//!       "confidence"?, "is_direct_match"? }
//! ] }
//! ```
//!
//! Strategy:
//! - q empty → `simaris list --json [--type T] [--include-archived]`.
//! - q non-empty → `simaris ask <q> --json [--type T] [--include-archived]`,
//!   falling back to `simaris search <q> --json [...]` if ask fails.

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cli::run_simaris_owned;

const SNIPPET_CHARS: usize = 240;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub include_archived: Option<String>,
}

fn parse_bool(s: &Option<String>) -> bool {
    match s.as_deref() {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => false,
    }
}

pub async fn get(Query(p): Query<SearchParams>) -> Response {
    let q_raw = p.q.unwrap_or_default();
    let q = q_raw.trim().to_string();
    let include_archived = parse_bool(&p.include_archived);
    let type_filter = p.type_.as_ref().filter(|t| !t.is_empty()).cloned();

    if q.is_empty() {
        return list_path(&type_filter, include_archived);
    }

    // Primary path: simaris ask. On failure fall back to simaris search.
    match run_ask(&q, &type_filter, include_archived) {
        Ok(value) => Json(normalize_ask(&q, &value)).into_response(),
        Err(ask_err) => {
            tracing::warn!(error = ?ask_err, "simaris ask failed, falling back to search");
            match run_search(&q, &type_filter, include_archived) {
                Ok(value) => Json(normalize_list(Some(q), &value)).into_response(),
                Err(err) => err_response(err),
            }
        }
    }
}

fn list_path(type_filter: &Option<String>, include_archived: bool) -> Response {
    let mut args: Vec<String> = vec!["list".into(), "--json".into()];
    if let Some(t) = type_filter {
        args.push("--type".into());
        args.push(t.clone());
    }
    if include_archived {
        args.push("--include-archived".into());
    }
    match run_simaris_owned(&args) {
        Ok(v) => Json(normalize_list(None, &v)).into_response(),
        Err(err) => err_response(err),
    }
}

fn run_ask(q: &str, type_filter: &Option<String>, include_archived: bool) -> anyhow::Result<Value> {
    let mut args: Vec<String> = vec!["ask".into(), q.to_string(), "--json".into()];
    if let Some(t) = type_filter {
        args.push("--type".into());
        args.push(t.clone());
    }
    if include_archived {
        args.push("--include-archived".into());
    }
    run_simaris_owned(&args)
}

fn run_search(
    q: &str,
    type_filter: &Option<String>,
    include_archived: bool,
) -> anyhow::Result<Value> {
    let mut args: Vec<String> = vec!["search".into(), q.to_string(), "--json".into()];
    if let Some(t) = type_filter {
        args.push("--type".into());
        args.push(t.clone());
    }
    if include_archived {
        args.push("--include-archived".into());
    }
    run_simaris_owned(&args)
}

fn normalize_ask(q: &str, v: &Value) -> Value {
    let units = v
        .get("units")
        .and_then(|u| u.as_array())
        .cloned()
        .unwrap_or_default();
    let normalized: Vec<Value> = units.iter().map(unit_from_ask).collect();
    json!({
        "kind": "ask",
        "query": q,
        "units": normalized,
    })
}

fn normalize_list(query: Option<String>, v: &Value) -> Value {
    let arr = v.as_array().cloned().unwrap_or_default();
    let normalized: Vec<Value> = arr.iter().map(unit_from_list).collect();
    json!({
        "kind": "list",
        "query": query.unwrap_or_default(),
        "units": normalized,
    })
}

fn unit_from_ask(u: &Value) -> Value {
    let content = u.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let snippet = truncate(content);
    json!({
        "id": u.get("id").cloned().unwrap_or(Value::Null),
        "type": u.get("unit_type").cloned().unwrap_or(Value::Null),
        "tags": u.get("tags").cloned().unwrap_or(json!([])),
        "source": u.get("source").cloned().unwrap_or(Value::Null),
        "slug": u.get("slug").cloned().unwrap_or(Value::Null),
        "byte_size": u.get("byte_size").cloned().unwrap_or(Value::Null),
        "snippet": snippet,
        "is_direct_match": u.get("is_direct_match").cloned().unwrap_or(json!(false)),
    })
}

fn unit_from_list(u: &Value) -> Value {
    let snippet = u
        .get("headline")
        .and_then(|c| c.as_str())
        .map(truncate)
        .unwrap_or_default();
    json!({
        "id": u.get("id").cloned().unwrap_or(Value::Null),
        "type": u.get("type").cloned().unwrap_or(Value::Null),
        "tags": u.get("tags").cloned().unwrap_or(json!([])),
        "source": u.get("source").cloned().unwrap_or(Value::Null),
        "slug": u.get("slug").cloned().unwrap_or(Value::Null),
        "byte_size": u.get("byte_size").cloned().unwrap_or(Value::Null),
        "confidence": u.get("confidence").cloned().unwrap_or(Value::Null),
        "snippet": snippet,
    })
}

fn truncate(s: &str) -> String {
    let mut iter = s.chars();
    let head: String = iter.by_ref().take(SNIPPET_CHARS).collect();
    if iter.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn err_response(err: anyhow::Error) -> Response {
    tracing::error!(error = ?err, "simaris search failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("simaris search failed: {err}"),
    )
        .into_response()
}
