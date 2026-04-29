//! simaris-server: HTTP admin dashboard for the simaris knowledge store.
//!
//! Bind 0.0.0.0:3535. JSON API mounted under `/api`; static files from
//! `web/` served at `/`. All data flows through the `simaris` CLI — see
//! `cli::run_simaris`.

mod cli;
mod middleware;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{
    Router,
    middleware::from_fn,
    routing::{get, post},
};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

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
        .fallback_service(ServeDir::new(web_dir()))
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

/// Resolve where to serve static files from. `SIMARIS_WEB_DIR` env var wins;
/// otherwise the workspace-root `web/` directory (one level above this
/// crate's manifest) is used.
fn web_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SIMARIS_WEB_DIR") {
        return PathBuf::from(d);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(|p| p.join("web"))
        .unwrap_or_else(|| PathBuf::from("web"))
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
