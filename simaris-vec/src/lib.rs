//! simaris-vec — vec subsystem for simaris hybrid retrieval.
//!
//! M5.1 milestone: subsystem ports the M3.2 lance prototype into mainline as a
//! workspace-member library. The `simaris` CLI does NOT yet consume this
//! subsystem — that wiring is M5.2 scope (`simaris search` hybrid leg) and
//! M5.3 scope (`simaris vec backfill` subcommand).
//!
//! Components:
//! - [`migrate`] — sqlite → arrow → lance dataset writer
//! - [`ask`]     — lance KNN ∪ tantivy text RRF (k=60) hybrid query
//! - [`verify`]  — round-trip integrity checks
//! - [`embed`]   — bge-m3 ollama HTTP embedding client
//!
//! Direct-write Python fallback for the embedding path lives in `tools/` per
//! `simaris-m3-redo-2-verdict-2026-05-04` deadlock workaround caveat.
//!
//! References:
//! - `simaris-m5-impl-plan-2026-05-04`
//! - `memory-architect-sitrep-2026-05-03-m3-2`
//! - `simaris-m3-redo-2-results-2026-05-03`

pub mod ask;
pub mod backfill;
pub mod embed;
pub mod migrate;
pub mod verify;

/// Reciprocal rank fusion constant. Matches sqlite-vec sibling and the
/// M3-redo-2 measured cell at lance × bge-m3 r@5=0.4708 / MRR=0.6936.
pub const RRF_K: usize = 60;

/// Default bge-m3 embedding dimension.
pub const BGE_M3_DIM: usize = 1024;

/// Default nomic-embed-text-v1.5 dimension.
pub const NOMIC_DIM: usize = 768;
