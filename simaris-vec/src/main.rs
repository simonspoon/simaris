//! simaris-vec-dev — ad-hoc developer binary for the vec subsystem.
//!
//! Branch-only and not invoked by the `simaris` CLI. Used for migrating a
//! sqlite snapshot, building a tantivy index, running smoke-test queries.
//! Production wiring lands in M5.2 (`simaris search`) and M5.3
//! (`simaris vec backfill`).
//!
//! Subcommands:
//!   migrate       sqlite -> arrow -> lance dataset (units + links + aspects + slugs + marks)
//!   verify-links  load lance dataset back, count links by relationship, diff vs source
//!   size-report   measure on-disk size of dataset + tantivy index
//!   ask           hybrid query (lance KNN + tantivy + RRF)
//!
//! Embeddings: dev binary uses placeholder zero-vectors at the requested dim.
//! Real embedding ingestion lives in M5.3 backfill tooling.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use simaris_vec::{ask, migrate, verify};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "simaris-vec-dev", about = "simaris vec subsystem dev tool")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// path to source sqlite db (default: ~/.simaris/sanctuary.db)
    #[arg(long, global = true)]
    sqlite: Option<PathBuf>,

    /// lance dataset directory (default: ~/.simaris/vec/)
    #[arg(long, global = true)]
    lance_dir: Option<PathBuf>,

    /// tantivy index directory (default: ~/.simaris/vec-tantivy/)
    #[arg(long, global = true)]
    tantivy_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// migrate sqlite -> arrow -> lance + build tantivy index
    Migrate {
        /// embedding dimension (768 = nomic, 1024 = BGE-M3)
        #[arg(long, default_value = "1024")]
        dim: usize,
        /// embedding model label stored in dataset metadata
        #[arg(long, default_value = "placeholder")]
        model: String,
    },
    /// verify all units/links/aspects round-tripped from sqlite to lance
    VerifyLinks,
    /// report on-disk size of lance dataset + tantivy index
    SizeReport,
    /// hybrid query: lance KNN UNION tantivy text search, RRF fusion (k=60)
    Ask {
        query: String,
        #[arg(long, default_value = "10")]
        n: usize,
    },
}

fn default_paths(cli: &Cli) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let home = dirs::home_dir().context("no $HOME")?;
    let sqlite = cli
        .sqlite
        .clone()
        .unwrap_or_else(|| home.join(".simaris/sanctuary.db"));
    let lance = cli
        .lance_dir
        .clone()
        .unwrap_or_else(|| home.join(".simaris/vec"));
    let tantivy = cli
        .tantivy_dir
        .clone()
        .unwrap_or_else(|| home.join(".simaris/vec-tantivy"));
    Ok((sqlite, lance, tantivy))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (sqlite, lance, tantivy) = default_paths(&cli)?;

    if !sqlite.exists() {
        bail!("source sqlite not found: {}", sqlite.display());
    }

    match &cli.cmd {
        Cmd::Migrate { dim, model } => migrate::run(&sqlite, &lance, &tantivy, *dim, model).await,
        Cmd::VerifyLinks => verify::run(&sqlite, &lance).await,
        Cmd::SizeReport => verify::size_report(&lance, &tantivy),
        Cmd::Ask { query, n } => ask::run(&lance, &tantivy, query, *n).await,
    }
}
