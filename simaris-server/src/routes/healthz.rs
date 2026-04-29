use axum::http::StatusCode;

/// Liveness probe. No external calls — succeeds as long as the server
/// process is running and tokio is healthy.
pub async fn get() -> StatusCode {
    StatusCode::OK
}
