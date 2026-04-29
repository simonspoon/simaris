//! Body-size middleware.
//!
//! Implements the recipe in simaris procedure 019d93e1: read the request
//! body up to a hard limit using `axum::body::to_bytes`, distinguish
//! `LengthLimitError` from other body-read errors via `Error::source` +
//! `downcast_ref` (the only public way — `axum_core::Error` does not expose
//! `downcast_ref`), and turn an exceeded limit into 413 instead of letting
//! the handler see a silently-empty body.

use std::error::Error as _;

use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::LengthLimitError;

/// 1 MiB. Admin endpoints take small JSON payloads; bigger requests are
/// almost certainly mistakes or abuse.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Buffer the request body up to `MAX_BODY_BYTES`, then re-attach it to the
/// request before forwarding. Returns 413 on size overflow, 400 on any
/// other body-read failure.
pub async fn body_size_limit(req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => {
            let req = Request::from_parts(parts, Body::from(bytes));
            next.run(req).await
        }
        Err(err) => {
            // axum_core::Error wraps the underlying error; the only public
            // way to detect LengthLimitError is via Error::source + downcast.
            let too_large = err
                .source()
                .and_then(|s| s.downcast_ref::<LengthLimitError>())
                .is_some();
            if too_large {
                tracing::warn!(limit = MAX_BODY_BYTES, "request body exceeded limit");
                (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response()
            } else {
                tracing::warn!(error = %err, "body read failed");
                (StatusCode::BAD_REQUEST, "request body read failed").into_response()
            }
        }
    }
}
