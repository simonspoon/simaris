//! Triage scan endpoints.
//!
//! Each endpoint shells out to `simaris scan --cat <name> --stale-days <n>`.
//!
//! - `GET /api/scan/counts?stale_days=30`  — all category counts
//! - `GET /api/scan/degraded`
//! - `GET /api/scan/contradictions`
//! - `GET /api/scan/oversized`
//! - `GET /api/scan/orphaned`
//! - `GET /api/scan/stale?stale_days=30`

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::Value;

use crate::cli::run_simaris_owned;

#[derive(Debug, Deserialize)]
pub struct ScanQuery {
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
}

fn default_stale_days() -> u32 {
    30
}

pub async fn counts(Query(q): Query<ScanQuery>) -> Response {
    let args = vec![
        "scan".into(),
        "--cat".into(),
        "counts".into(),
        "--stale-days".into(),
        q.stale_days.to_string(),
    ];
    run_or_error(&args)
}

pub async fn category(Path(cat): Path<String>, Query(q): Query<ScanQuery>) -> Response {
    let mut args = vec!["scan".into(), "--cat".into(), cat.clone()];
    if cat == "stale" {
        args.push("--stale-days".into());
        args.push(q.stale_days.to_string());
    }
    run_or_error(&args)
}

fn run_or_error(args: &[String]) -> Response {
    match run_simaris_owned(args) {
        Ok(value) => Json::<Value>(value).into_response(),
        Err(err) => {
            tracing::error!(args = ?args, error = ?err, "simaris scan failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("scan failed: {err}"),
            )
                .into_response()
        }
    }
}
