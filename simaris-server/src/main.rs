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
    http::{StatusCode, Uri, header},
    middleware::from_fn,
    response::{IntoResponse, Redirect, Response},
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
        .route("/units/:id/verify", post(routes::units::verify))
        .route("/scan/counts", get(routes::scan::counts))
        .route("/scan/:cat", get(routes::scan::category))
        .route(
            "/consolidation/clusters",
            get(routes::consolidation::clusters),
        )
        .route(
            "/consolidation/action",
            post(routes::consolidation::action),
        )
        .layer(from_fn(middleware::body_size_limit));

    let app = Router::new()
        .route("/healthz", get(routes::healthz::get))
        .nest("/api", api)
        .route("/", get(serve_triage))
        .route("/dashboard", get(serve_index))
        // Triage — scan-first homepage (Layer 4).
        .route("/triage", get(serve_triage))
        .route("/triage/", get(serve_triage))
        // Retired surfaces (2026-05-14): /units, /wiki, /units.html → /browse.
        // Files remain on disk for now; routes redirect at the server.
        .route("/units", get(redirect_to_browse))
        .route("/units/", get(redirect_to_browse))
        .route("/units.html", get(redirect_to_browse))
        .route("/wiki", get(redirect_to_browse))
        .route("/wiki/", get(redirect_to_browse))
        .route("/wiki/*rest", get(redirect_to_browse_path))
        // Browse — two-pane card browser (Layer 3).
        .route("/browse", get(serve_browse))
        .route("/browse/", get(serve_browse))
        // Consolidation — cluster review pane (Phase 1 of consolidation).
        .route("/consolidation", get(serve_consolidation))
        .route("/consolidation/", get(serve_consolidation))
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

/// Serve `triage.html` at `/` and `/triage`.
async fn serve_triage() -> Response {
    asset_response("triage.html")
}

/// Serve `index.html` (Dashboard) at `/dashboard`.
async fn serve_index() -> Response {
    asset_response("index.html")
}

/// Serve `browse.html` at `/browse`.
async fn serve_browse() -> Response {
    asset_response("browse.html")
}

/// Serve `consolidation.html` at `/consolidation`.
async fn serve_consolidation() -> Response {
    asset_response("consolidation.html")
}

/// Permanent redirect for retired surfaces (/units, /wiki, /units.html) → /browse.
async fn redirect_to_browse() -> Redirect {
    Redirect::permanent("/browse")
}

/// Permanent redirect for retired wiki sub-paths (/wiki/<id-or-slug>) → /browse.
/// Path tail is dropped; the user lands on browse welcome state.
async fn redirect_to_browse_path(_uri: Uri) -> Redirect {
    Redirect::permanent("/browse")
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
