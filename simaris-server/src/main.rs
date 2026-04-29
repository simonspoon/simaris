//! simaris-server: HTTP admin dashboard for the simaris knowledge store.
//!
//! Bind 0.0.0.0:3535. JSON API mounted under `/api`; static files for the
//! dashboard UI are embedded at compile time via `rust_embed` and served at
//! `/`. All data flows through the `simaris` CLI — see `cli::run_simaris`.

mod cli;
mod middleware;
mod routes;

use std::net::SocketAddr;

use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{StatusCode, header},
    middleware::from_fn,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use tower_http::trace::TraceLayer;

/// Static dashboard assets baked into the binary at build time. In debug
/// builds rust_embed reads from disk on every request (hot reload); in
/// release builds files are embedded verbatim.
#[derive(RustEmbed)]
#[folder = "../web/"]
struct WebAssets;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let api = Router::new()
        .route("/stats", get(routes::stats::get))
        .route("/search", get(routes::search::get))
        .route(
            "/units/:id",
            get(routes::units::get_unit).post(routes::units::edit),
        )
        .route("/units/:id/clone", post(routes::units::clone))
        .route("/units/:id/archive", post(routes::units::archive))
        .route("/units/:id/unarchive", post(routes::units::unarchive))
        .layer(from_fn(middleware::body_size_limit));

    let app = Router::new()
        .route("/healthz", get(routes::healthz::get))
        .nest("/api", api)
        .route("/", get(serve_index))
        .route("/*path", get(serve_asset))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = "0.0.0.0:3535".parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "simaris-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}

/// Serve `index.html` at `/`.
async fn serve_index() -> Response {
    asset_response("index.html")
}

/// Serve any other embedded asset by path.
async fn serve_asset(Path(path): Path<String>) -> Response {
    asset_response(&path)
}

fn asset_response(path: &str) -> Response {
    match WebAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.into_owned()))
                .expect("build asset response")
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
