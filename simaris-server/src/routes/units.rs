//! Per-unit endpoints. All routes shell out to the simaris CLI.
//!
//! - `GET  /api/units/:id` — `simaris show <id> --json`
//! - `POST /api/units/:id` — `simaris edit <id> [--content/--type/--tags/--source] --json`
//! - `POST /api/units/:id/clone` — `simaris clone <id> --json`
//! - `POST /api/units/:id/archive` — `simaris archive <id> --json`
//! - `POST /api/units/:id/unarchive` — `simaris unarchive <id> --json`

use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::Value;

use crate::cli::run_simaris_owned;

#[derive(Debug, Deserialize, Default)]
pub struct EditBody {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

pub async fn get_unit(Path(id): Path<String>) -> Response {
    let args = vec!["show".to_string(), id, "--json".to_string()];
    run_or_error(&args)
}

pub async fn edit(Path(id): Path<String>, body: Option<Json<EditBody>>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let mut args: Vec<String> = vec!["edit".into(), id, "--json".into()];

    let mut any = false;
    if let Some(c) = &body.content {
        args.push("--content".into());
        args.push(c.clone());
        any = true;
    }
    if let Some(t) = &body.type_
        && !t.is_empty()
    {
        args.push("--type".into());
        args.push(t.clone());
        any = true;
    }
    if let Some(t) = &body.tags {
        args.push("--tags".into());
        args.push(t.clone());
        any = true;
    }
    if let Some(s) = &body.source {
        args.push("--source".into());
        args.push(s.clone());
        any = true;
    }
    if !any {
        return (
            StatusCode::BAD_REQUEST,
            "edit body must set at least one of: content, type, tags, source",
        )
            .into_response();
    }
    run_or_error(&args)
}

pub async fn clone(Path(id): Path<String>) -> Response {
    let args = vec!["clone".into(), id, "--json".into()];
    run_or_error(&args)
}

pub async fn archive(Path(id): Path<String>) -> Response {
    let args = vec!["archive".into(), id, "--json".into()];
    run_or_error(&args)
}

pub async fn unarchive(Path(id): Path<String>) -> Response {
    let args = vec!["unarchive".into(), id, "--json".into()];
    run_or_error(&args)
}

fn run_or_error(args: &[String]) -> Response {
    match run_simaris_owned(args) {
        Ok(value) => Json::<Value>(value).into_response(),
        Err(err) => {
            tracing::error!(args = ?args, error = ?err, "simaris CLI failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("simaris failed: {err}"),
            )
                .into_response()
        }
    }
}
