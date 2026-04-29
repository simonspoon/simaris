use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::cli::run_simaris;

/// Pass-through to `simaris stats --json`. The body is whatever the CLI
/// emitted; the server adds no shape guarantees beyond "valid JSON".
pub async fn get() -> Response {
    match run_simaris(&["stats", "--json"]) {
        Ok(value) => Json(value).into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "simaris stats --json failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("simaris stats failed: {err}"),
            )
                .into_response()
        }
    }
}

type Response = axum::response::Response;
