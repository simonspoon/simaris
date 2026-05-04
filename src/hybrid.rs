//! Hybrid search bridge — wires the simaris-vec lance leg into `simaris search`.
//!
//! M5.2 contract:
//! - Default: lance KNN ∪ tantivy text → RRF (k=60) → top-N (default 10).
//! - `--no-vec`: caller skips this module entirely; existing FTS5 path runs.
//! - Lance dataset absent: caller logs a warning to stderr and falls back to
//!   FTS5. We DO NOT crash on missing dataset — the production sanctuary
//!   ships without lance until the user runs the M5.3 backfill subcommand.
//!
//! Path discovery:
//! - Env override: `SIMARIS_VEC_DIR` points at the directory that holds
//!   `units.lance` AND a `tantivy/` subdir (matches the M3-redo-2 layout).
//! - Default: `$SIMARIS_HOME/vec/<model>` where `<model>` defaults to
//!   `bge-m3` (the M5 ratified backend). `SIMARIS_HOME` resolves to
//!   `~/.simaris` if unset, mirroring the rest of the CLI.
//!
//! Embedding model: bge-m3 via local ollama. `SIMARIS_OLLAMA_URL` overrides
//! the base URL (default `http://localhost:11434`).

use crate::db::{self, Unit};
use anyhow::{Context, Result};
use rusqlite::Connection;
use simaris_vec::ask::hybrid_search;
use simaris_vec::embed::{BGE_M3_MODEL, OLLAMA_DEFAULT_URL, OllamaEmbedClient};
use std::path::{Path, PathBuf};

/// Resolved hybrid configuration. Returned by [`HybridConfig::discover`] so
/// the caller can decide between the hybrid path and the FTS5 fallback before
/// touching ollama.
pub struct HybridConfig {
    pub lance_dir: PathBuf,
    pub tantivy_dir: PathBuf,
    pub model: String,
    pub ollama_url: String,
}

impl HybridConfig {
    /// Discover paths from env + defaults. Returns `Ok(Some(cfg))` when both
    /// `units.lance` and `tantivy/` are present. Returns `Ok(None)` when the
    /// dataset is absent (caller falls back). Errors only on environmental
    /// problems (no $HOME, etc.).
    pub fn discover() -> Result<Option<Self>> {
        let model = std::env::var("SIMARIS_VEC_MODEL").unwrap_or_else(|_| BGE_M3_MODEL.to_string());
        let ollama_url =
            std::env::var("SIMARIS_OLLAMA_URL").unwrap_or_else(|_| OLLAMA_DEFAULT_URL.to_string());

        let base_dir = if let Ok(v) = std::env::var("SIMARIS_VEC_DIR") {
            PathBuf::from(v)
        } else {
            let home_root = if let Ok(h) = std::env::var("SIMARIS_HOME") {
                PathBuf::from(h)
            } else {
                dirs::home_dir()
                    .context("no $HOME for vec dir discovery")?
                    .join(".simaris")
            };
            home_root.join("vec").join(&model)
        };

        let lance_dir = base_dir.clone();
        let tantivy_dir = base_dir.join("tantivy");
        if !lance_dir.join("units.lance").exists() || !tantivy_dir.exists() {
            return Ok(None);
        }
        Ok(Some(Self {
            lance_dir,
            tantivy_dir,
            model,
            ollama_url,
        }))
    }
}

/// Run hybrid retrieval and resolve unit ids back to `Unit` rows from sqlite.
///
/// `top_n` is the fused result count returned to the caller. Per-leg
/// candidate pool is fixed at 50 to match the M3-redo-2 cell.
///
/// Filtering policy:
/// - `type_filter`: applied post-fusion (lance/tantivy carry no FTS schema
///   awareness; cheaper to filter the small fused set than re-shape both
///   legs).
/// - `include_archived`: same — filter post-fusion. The sanctuary's archived
///   units are present in lance/tantivy because the bench corpus snapshots
///   them too; honour the same default-hide rule the FTS5 path enforces.
pub fn run_hybrid(
    conn: &Connection,
    cfg: &HybridConfig,
    query: &str,
    top_n: usize,
    type_filter: Option<&str>,
    include_archived: bool,
) -> Result<Vec<Unit>> {
    let client = OllamaEmbedClient::new(&cfg.ollama_url, &cfg.model);
    let qvec = client
        .embed(query)
        .with_context(|| format!("embed query via ollama ({})", cfg.model))?;

    // Over-request from fusion so we still have `top_n` after type/archived
    // filters drop ineligible rows.
    let candidate_pool = 50usize;
    let fused_pool = (top_n * 5).max(50);

    let fused = run_async(hybrid_search(
        &cfg.lance_dir,
        &cfg.tantivy_dir,
        query,
        &qvec,
        fused_pool,
        candidate_pool,
    ))?;

    let mut out: Vec<Unit> = Vec::with_capacity(top_n);
    for (id, _score) in fused {
        let Ok(u) = db::get_unit(conn, &id) else {
            continue;
        };
        if !include_archived && u.archived {
            continue;
        }
        if let Some(t) = type_filter
            && u.unit_type != t
        {
            continue;
        }
        out.push(u);
        if out.len() >= top_n {
            break;
        }
    }
    Ok(out)
}

/// Spin up a one-shot tokio current-thread runtime. Lance is async-only; the
/// simaris CLI is sync. We avoid plumbing tokio through the whole binary by
/// localising the runtime here — fresh per call, dropped on return.
fn run_async<F: std::future::Future<Output = Result<T>>, T>(fut: F) -> Result<T> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(fut)
}

/// Helper for tests that want to know what discovery would resolve to without
/// requiring the dataset to actually exist.
#[allow(dead_code)]
pub fn debug_paths() -> (PathBuf, PathBuf) {
    let model = std::env::var("SIMARIS_VEC_MODEL").unwrap_or_else(|_| BGE_M3_MODEL.to_string());
    let base = if let Ok(v) = std::env::var("SIMARIS_VEC_DIR") {
        PathBuf::from(v)
    } else {
        let home = std::env::var("SIMARIS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| Path::new(".").to_path_buf())
                    .join(".simaris")
            });
        home.join("vec").join(model)
    };
    (base.clone(), base.join("tantivy"))
}
