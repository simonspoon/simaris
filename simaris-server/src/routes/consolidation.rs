//! Consolidation review pane endpoints.
//!
//! Surfaces the `simaris cluster --all` output as a reviewable list of
//! clusters and lets the user resolve each cluster with one of:
//!
//! - `archive` — keep `canonical_id`, archive every other `member_ids` entry
//!   and add a `supersedes` link from the archived unit to the canonical.
//!   Also tags the archived unit with `merge-source-<canonical-prefix>` so
//!   provenance survives the archive.
//! - `retype` — set `--type new_type` on every `member_ids` entry.
//! - `merge`  — phase-1 punt; returns 501 with a hint.
//! - `skip`   — record `cluster_id` in `skipped.json` so it disappears on
//!   the next cluster fetch.
//!
//! ## Endpoints
//!
//! - `GET  /api/consolidation/clusters?refresh=true&tag=skills`
//!     - With cache hit (and `refresh` absent/false): returns the cached
//!       report from `<state_dir>/clusters.cache.json`. With `refresh=true`
//!       or a cache miss: shells out to `simaris cluster --all --json`
//!       (or `--tag <tag>` when supplied), writes the cache, returns it.
//!     - Clusters whose `cluster_id` appears in `skipped.json` are filtered
//!       out before returning, and the `summary.cluster_count` is adjusted.
//! - `POST /api/consolidation/action` — body shape above. Sequential
//!   shell-outs; per-member result vector returned in `results`.
//!
//! ## State files (under `$HOME/.simaris-server/`)
//!
//! - `clusters.cache.json` — last cluster report (full ClusterReport JSON).
//! - `skipped.json`        — `{ "cluster_ids": ["abc12345", ...] }`.
//! - `audit.log`           — newline-delimited JSON, one entry per POST.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::run_simaris_owned;

// ---------------------------------------------------------------------------
// State directory + file helpers.
// ---------------------------------------------------------------------------

/// `$HOME/.simaris-server/`. Created lazily on first write. Falls back to
/// `./.simaris-server/` if `$HOME` is unset (unusual but possible in CI).
fn state_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".simaris-server")
}

fn cache_path() -> PathBuf {
    state_dir().join("clusters.cache.json")
}

fn skipped_path() -> PathBuf {
    state_dir().join("skipped.json")
}

fn audit_path() -> PathBuf {
    state_dir().join("audit.log")
}

fn ensure_state_dir() -> std::io::Result<()> {
    fs::create_dir_all(state_dir())
}

// ---------------------------------------------------------------------------
// Skipped-cluster persistence.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct SkippedFile {
    #[serde(default)]
    cluster_ids: Vec<String>,
}

fn read_skipped() -> SkippedFile {
    fs::read_to_string(skipped_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_skipped(s: &SkippedFile) -> std::io::Result<()> {
    ensure_state_dir()?;
    let body = serde_json::to_string_pretty(s).unwrap_or_else(|_| "{}".into());
    fs::write(skipped_path(), body)
}

fn add_skipped(cluster_id: &str) -> std::io::Result<()> {
    let mut s = read_skipped();
    if !s.cluster_ids.iter().any(|c| c == cluster_id) {
        s.cluster_ids.push(cluster_id.to_string());
    }
    write_skipped(&s)
}

// ---------------------------------------------------------------------------
// Audit log (append-only JSONL).
// ---------------------------------------------------------------------------

fn append_audit(entry: &Value) {
    if let Err(e) = ensure_state_dir() {
        tracing::warn!(error = ?e, "audit: ensure_state_dir failed");
        return;
    }
    let line = match serde_json::to_string(entry) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, "audit: serialize failed");
            return;
        }
    };
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path());
    match file {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                tracing::warn!(error = ?e, "audit: write failed");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "audit: open failed"),
    }
}

// ---------------------------------------------------------------------------
// GET /api/consolidation/clusters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ClustersQuery {
    /// When true, ignore the cache and re-run `simaris cluster`.
    #[serde(default)]
    pub refresh: bool,
    /// Optional tag filter. Forwarded as `--tag <tag>` to the CLI when set;
    /// otherwise `--all` is used.
    #[serde(default)]
    pub tag: Option<String>,
    /// Optional type filter. Forwarded as `--type <type>` to the CLI.
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
}

pub async fn clusters(Query(q): Query<ClustersQuery>) -> Response {
    // Cache hit path — only when no filters and no explicit refresh.
    let cached = if !q.refresh && q.tag.is_none() && q.type_.is_none() {
        fs::read_to_string(cache_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    } else {
        None
    };

    let report = match cached {
        Some(v) => v,
        None => {
            let mut args: Vec<String> = vec!["cluster".into(), "--json".into()];
            match (&q.tag, &q.type_) {
                (Some(t), _) => {
                    args.push("--tag".into());
                    args.push(t.clone());
                }
                (None, _) => args.push("--all".into()),
            }
            if let Some(t) = &q.type_ {
                args.push("--type".into());
                args.push(t.clone());
            }
            match run_simaris_owned(&args) {
                Ok(v) => {
                    // Only persist the unfiltered baseline (no tag/type).
                    if q.tag.is_none() && q.type_.is_none() {
                        if let Err(e) = ensure_state_dir() {
                            tracing::warn!(error = ?e, "clusters: ensure_state_dir failed");
                        } else if let Ok(body) = serde_json::to_string_pretty(&v) {
                            let _ = fs::write(cache_path(), body);
                        }
                    }
                    v
                }
                Err(err) => {
                    tracing::error!(error = ?err, "simaris cluster failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("simaris cluster failed: {err}"),
                    )
                        .into_response();
                }
            }
        }
    };

    // Filter out skipped cluster_ids.
    let skipped: HashSet<String> = read_skipped().cluster_ids.into_iter().collect();
    let filtered = filter_skipped(report, &skipped);
    Json(filtered).into_response()
}

/// Remove clusters whose `cluster_id` is in `skipped`; re-derive
/// `summary.cluster_count` and `summary.by_pattern` from the survivors.
fn filter_skipped(mut report: Value, skipped: &HashSet<String>) -> Value {
    let Some(clusters) = report.get_mut("clusters").and_then(|c| c.as_array_mut()) else {
        return report;
    };
    clusters.retain(|c| {
        c.get("cluster_id")
            .and_then(|v| v.as_str())
            .map(|id| !skipped.contains(id))
            .unwrap_or(true)
    });
    let surviving_count = clusters.len();
    // Recompute by_pattern over surviving clusters.
    let mut by_pattern = serde_json::Map::new();
    for c in clusters.iter() {
        if let Some(pats) = c.get("patterns").and_then(|v| v.as_array()) {
            for p in pats {
                if let Some(s) = p.as_str() {
                    let v = by_pattern
                        .entry(s.to_string())
                        .or_insert_with(|| json!(0));
                    if let Some(n) = v.as_u64() {
                        *v = json!(n + 1);
                    }
                }
            }
        }
    }
    if let Some(summary) = report.get_mut("summary").and_then(|s| s.as_object_mut()) {
        summary.insert("cluster_count".into(), json!(surviving_count));
        summary.insert("by_pattern".into(), Value::Object(by_pattern));
    }
    report
}

// ---------------------------------------------------------------------------
// POST /api/consolidation/action
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ActionBody {
    pub cluster_id: String,
    pub action: String,
    #[serde(default)]
    pub canonical_id: Option<String>,
    #[serde(default)]
    pub member_ids: Vec<String>,
    #[serde(default)]
    pub new_type: Option<String>,
}

/// Per-action result. Each shell-out yields one entry.
#[derive(Debug, Serialize)]
struct StepResult {
    /// Subject unit id (target of the operation).
    id: String,
    /// Sub-step label (e.g. "archive", "supersedes-link", "tag", "retype").
    step: String,
    /// True on shell-out success.
    ok: bool,
    /// On failure: the error message; on success: empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    error: String,
}

/// Cap to avoid runaway shell-outs. Submits larger than this 400.
const MAX_MEMBERS_PER_SUBMIT: usize = 50;

pub async fn action(Json(body): Json<ActionBody>) -> Response {
    let started_at = chrono_like_now();
    let action_lc = body.action.to_lowercase();

    // Pre-audit the request body so we have a paper trail even on early returns.
    let request_audit = json!({
        "ts": started_at,
        "kind": "request",
        "body": body_to_value(&body),
    });
    append_audit(&request_audit);

    if body.member_ids.len() > MAX_MEMBERS_PER_SUBMIT {
        let msg = format!(
            "member_ids count {} exceeds cap {}",
            body.member_ids.len(),
            MAX_MEMBERS_PER_SUBMIT
        );
        let resp = json!({ "ok": false, "error": msg });
        append_audit(&json!({
            "ts": started_at,
            "kind": "response",
            "cluster_id": body.cluster_id,
            "action": action_lc,
            "result": resp,
        }));
        return (StatusCode::BAD_REQUEST, Json(resp)).into_response();
    }

    let result: Response = match action_lc.as_str() {
        "archive" => do_archive(&body),
        "retype" => do_retype(&body),
        "merge" => {
            let resp = json!({
                "ok": false,
                "error": "merge action not implemented in phase 1 — use the manual content-merge workflow",
            });
            (StatusCode::NOT_IMPLEMENTED, Json(resp.clone())).into_response()
        }
        "skip" => do_skip(&body),
        other => {
            let resp = json!({
                "ok": false,
                "error": format!("unknown action: {other}"),
            });
            (StatusCode::BAD_REQUEST, Json(resp.clone())).into_response()
        }
    };

    // Best-effort audit of the response. We can't read the response body
    // (axum::Response consumes it), so we re-record a coarse audit entry
    // capturing status code only. Detailed per-step results are in the
    // sub-handlers' own audit writes.
    let status = result.status().as_u16();
    append_audit(&json!({
        "ts": started_at,
        "kind": "response",
        "cluster_id": body.cluster_id,
        "action": action_lc,
        "status": status,
    }));

    result
}

fn body_to_value(b: &ActionBody) -> Value {
    json!({
        "cluster_id": b.cluster_id,
        "action": b.action,
        "canonical_id": b.canonical_id,
        "member_ids": b.member_ids,
        "new_type": b.new_type,
    })
}

/// Archive every member that isn't the canonical, link them with
/// `supersedes` → canonical, and tag them with a merge-source marker.
fn do_archive(body: &ActionBody) -> Response {
    let Some(canonical) = body.canonical_id.as_deref() else {
        return bad_request("archive action requires canonical_id");
    };
    if canonical.is_empty() {
        return bad_request("canonical_id must not be empty");
    }

    let tag = format!("merge-source-{}", &canonical.chars().take(8).collect::<String>());

    let mut steps: Vec<StepResult> = Vec::new();
    let mut any_failure = false;

    for member in &body.member_ids {
        if member == canonical {
            // Don't archive the canonical itself.
            continue;
        }
        // 1. archive
        let res = run_simaris_owned(&[
            "archive".into(),
            member.clone(),
            "--json".into(),
        ]);
        let archived_ok = record(&mut steps, member, "archive", res, &mut any_failure);
        if !archived_ok {
            continue;
        }

        // 2. supersedes link: member -> canonical
        let res = run_simaris_owned(&[
            "link".into(),
            member.clone(),
            canonical.to_string(),
            "--rel".into(),
            "supersedes".into(),
            "--json".into(),
        ]);
        record(&mut steps, member, "supersedes-link", res, &mut any_failure);

        // 3. tag merge-source-<canonical-prefix>. `simaris edit --tags`
        // replaces the tag set wholesale, so read existing tags via
        // `simaris show --json` first and append. The show payload is
        // shaped `{ unit: { tags: [...] }, links: ..., slugs: ... }`,
        // so dig into `unit.tags`. Fall back to overwrite if read fails.
        let existing = run_simaris_owned(&[
            "show".into(),
            member.clone(),
            "--json".into(),
        ]);
        let tags_arg = match &existing {
            Ok(v) => {
                let mut tags: Vec<String> = v
                    .get("unit")
                    .and_then(|u| u.get("tags"))
                    .and_then(|t| t.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if !tags.iter().any(|t| t == &tag) {
                    tags.push(tag.clone());
                }
                tags.join(",")
            }
            Err(_) => tag.clone(),
        };

        let res = run_simaris_owned(&[
            "edit".into(),
            member.clone(),
            "--tags".into(),
            tags_arg,
            "--json".into(),
        ]);
        record(&mut steps, member, "tag", res, &mut any_failure);
    }

    let resp = json!({
        "ok": !any_failure,
        "cluster_id": body.cluster_id,
        "action": "archive",
        "canonical_id": canonical,
        "results": steps,
    });

    // Invalidate the cluster cache — actions mutate the store, so the
    // cached cluster report no longer reflects reality.
    invalidate_cache();

    Json(resp).into_response()
}

/// Set `--type new_type` on every `member_ids` entry.
fn do_retype(body: &ActionBody) -> Response {
    let Some(new_type) = body.new_type.as_deref() else {
        return bad_request("retype action requires new_type");
    };
    if new_type.is_empty() {
        return bad_request("new_type must not be empty");
    }

    let mut steps: Vec<StepResult> = Vec::new();
    let mut any_failure = false;
    for member in &body.member_ids {
        let res = run_simaris_owned(&[
            "edit".into(),
            member.clone(),
            "--type".into(),
            new_type.to_string(),
            "--json".into(),
        ]);
        record(&mut steps, member, "retype", res, &mut any_failure);
    }

    let resp = json!({
        "ok": !any_failure,
        "cluster_id": body.cluster_id,
        "action": "retype",
        "new_type": new_type,
        "results": steps,
    });

    invalidate_cache();
    Json(resp).into_response()
}

/// Persist the cluster_id in skipped.json so the cluster disappears on
/// the next fetch. No store mutation.
fn do_skip(body: &ActionBody) -> Response {
    if let Err(e) = add_skipped(&body.cluster_id) {
        tracing::error!(error = ?e, "skip: write failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("skip persist failed: {e}") })),
        )
            .into_response();
    }
    Json(json!({
        "ok": true,
        "cluster_id": body.cluster_id,
        "action": "skip",
    }))
    .into_response()
}

fn invalidate_cache() {
    let _ = fs::remove_file(cache_path());
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn record(
    steps: &mut Vec<StepResult>,
    id: &str,
    step: &str,
    res: anyhow::Result<Value>,
    any_failure: &mut bool,
) -> bool {
    match res {
        Ok(_) => {
            steps.push(StepResult {
                id: id.to_string(),
                step: step.to_string(),
                ok: true,
                error: String::new(),
            });
            true
        }
        Err(e) => {
            *any_failure = true;
            let msg = format!("{e}");
            tracing::warn!(id = %id, step = %step, error = %msg, "consolidation step failed");
            steps.push(StepResult {
                id: id.to_string(),
                step: step.to_string(),
                ok: false,
                error: msg,
            });
            false
        }
    }
}

/// Unix epoch seconds — audit timestamp. Plenty of precision for a log and
/// avoids pulling in `chrono` for one call. The audit reader (humans + grep)
/// can convert with `date -r <secs>`.
fn chrono_like_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
