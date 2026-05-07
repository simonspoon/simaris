//! Anthropic Contextual Retrieval — generate per-unit context preambles.
//!
//! Pipeline: pull a unit + its 1-hop link neighbours, format a context
//! block (parent slug + top-3 `related_to` headlines + top-3 `part_of`
//! children), and ask Haiku 3.5 for a single-sentence preamble that
//! situates the unit in the broader knowledge graph. The preamble is
//! stored in `units.context_preamble` and later prepended to the unit
//! body before embedding (`simaris vec backfill --reembed-with-context`).
//!
//! Modes:
//! - `--dry-run` (default): runs against ≤5 sample units, generates
//!   real preambles when `ANTHROPIC_API_KEY` is set, otherwise mocks
//!   them, and reports a cost projection over the full backlog.
//! - `--execute`: explicit opt-in. Walks the entire backlog (or a
//!   `--limit N` slice) and persists preambles. Idempotent: skips
//!   units where `context_preamble IS NOT NULL` via the upstream
//!   filter in `db::list_unit_ids_without_preamble`.
//!
//! Rate limit: defaults to 50 req/min, override via
//! `SIMARIS_RATE_LIMIT_RPM`. The API key is read from
//! `ANTHROPIC_API_KEY` and never logged or echoed.
//!
//! Authority: Anthropic contextual retrieval (
//! <https://www.anthropic.com/news/contextual-retrieval>),
//! `lotus-m9-intel-picks-2026-05-06`, M9 picks brief m9.5.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::time::{Duration, Instant};

/// Haiku 3.5 model id (per simaris preference atom 019d86b6).
pub const HAIKU_MODEL: &str = "claude-3-5-haiku-20241022";

/// Anthropic Messages API endpoint.
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Haiku 3.5 input token cost (USD per million tokens).
pub const HAIKU_INPUT_USD_PER_MTOK: f64 = 0.80;

/// Haiku 3.5 output token cost (USD per million tokens).
pub const HAIKU_OUTPUT_USD_PER_MTOK: f64 = 4.00;

/// Output cap — preamble must fit one sentence; 100 tokens is generous.
const MAX_OUTPUT_TOKENS: u32 = 100;

/// Default rate limit (requests per minute). Overridden by
/// `SIMARIS_RATE_LIMIT_RPM`.
pub const DEFAULT_RATE_LIMIT_RPM: u32 = 50;

/// One unit-of-work response from the LLM. Token counts are reported by
/// the upstream API and feed the cost estimate.
pub struct PreambleResp {
    pub preamble: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Pluggable LLM client. Production wiring goes through
/// [`AnthropicClient`]; tests inject deterministic mocks via this trait
/// to keep `cargo test` hermetic and offline.
pub trait LlmClient {
    fn generate_preamble(&self, prompt: &str) -> Result<PreambleResp>;
}

/// Production Anthropic client over `reqwest::blocking`. Reads the API
/// key from `ANTHROPIC_API_KEY` at construction; never logs the key.
pub struct AnthropicClient {
    api_key: String,
    http: reqwest::blocking::Client,
}

impl AnthropicClient {
    /// Build a client by reading `ANTHROPIC_API_KEY`. Bails with a
    /// human-readable error when the env var is unset, which surfaces
    /// directly in the `simaris context-enhance` exit message.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY environment variable not set")?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build reqwest client")?;
        Ok(Self { api_key, http })
    }
}

impl LlmClient for AnthropicClient {
    fn generate_preamble(&self, prompt: &str) -> Result<PreambleResp> {
        let body = serde_json::json!({
            "model": HAIKU_MODEL,
            "max_tokens": MAX_OUTPUT_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
        });
        let resp = self
            .http
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .context("anthropic API request failed")?;
        let status = resp.status();
        let text = resp.text().context("read anthropic response body")?;
        if !status.is_success() {
            anyhow::bail!("anthropic API error ({status})");
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&text).context("parse anthropic response JSON")?;
        let preamble = parsed["content"][0]["text"]
            .as_str()
            .context("anthropic response missing content[0].text")?
            .trim()
            .to_string();
        let input_tokens = parsed["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = parsed["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
        Ok(PreambleResp {
            preamble,
            input_tokens,
            output_tokens,
        })
    }
}

/// 1-hop context block sourced from the link graph.
pub struct ContextBlock {
    /// First `part_of` parent's slug, when one exists. Acts as the
    /// "what bigger thing is this part of" anchor in the prompt.
    pub parent_slug: Option<String>,
    /// Headlines of up to three outbound `related_to` neighbours.
    pub related: Vec<String>,
    /// Headlines of up to three inbound `part_of` children — i.e.
    /// units that declare *this* unit as their parent.
    pub part_of_children: Vec<String>,
}

/// Build the context block for a unit by walking 1-hop link metadata.
pub fn build_context_block(conn: &Connection, unit_id: &str) -> Result<ContextBlock> {
    let outgoing = crate::db::get_links_from_with_meta(conn, unit_id)?;
    let incoming = crate::db::get_links_to_with_meta(conn, unit_id)?;

    let parent_slug = outgoing
        .iter()
        .find(|l| l.relationship == "part_of")
        .and_then(|l| l.slug.clone());

    let related: Vec<String> = outgoing
        .iter()
        .filter(|l| l.relationship == "related_to")
        .take(3)
        .map(|l| l.headline.clone())
        .collect();

    let part_of_children: Vec<String> = incoming
        .iter()
        .filter(|l| l.relationship == "part_of")
        .take(3)
        .map(|l| l.headline.clone())
        .collect();

    Ok(ContextBlock {
        parent_slug,
        related,
        part_of_children,
    })
}

/// Format the prompt sent to Haiku. Stable wording — changing this is a
/// versioning event because preambles already in the DB were produced
/// against the prior format.
pub fn build_prompt(unit: &crate::db::Unit, ctx: &ContextBlock) -> String {
    let mut s = String::new();
    s.push_str(
        "Given this knowledge atom and its context, write a single-sentence preamble \
         (max 30 words) that situates this atom in its broader knowledge graph. \
         Output ONLY the sentence, nothing else.\n\n",
    );
    s.push_str("ATOM:\n");
    s.push_str(&unit.content);
    s.push_str("\n\nCONTEXT:\n");
    if let Some(slug) = &ctx.parent_slug {
        s.push_str(&format!("- parent slug: {slug}\n"));
    }
    if !ctx.related.is_empty() {
        s.push_str("- related: ");
        s.push_str(&ctx.related.join("; "));
        s.push('\n');
    }
    if !ctx.part_of_children.is_empty() {
        s.push_str("- part_of children: ");
        s.push_str(&ctx.part_of_children.join("; "));
        s.push('\n');
    }
    s
}

/// One row of dry-run output — the prompt + generated (or mocked)
/// preamble for a sample unit.
pub struct SampleResult {
    pub id: String,
    pub headline: String,
    pub prompt: String,
    pub preamble: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub mocked: bool,
}

/// Aggregated dry-run report: per-sample detail plus an extrapolated
/// cost projection over the full backlog.
pub struct DryRunOutcome {
    pub samples: Vec<SampleResult>,
    pub total_backlog: u64,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mocked: bool,
}

/// Run the dry-run path. Pulls up to `sample_size` unprocessed units,
/// generates preambles via `client` (or mocks them when `client` is
/// `None`), and projects total cost as `avg_per_unit * backlog`.
pub fn run_dry_run(
    conn: &Connection,
    sample_size: usize,
    client: Option<&dyn LlmClient>,
) -> Result<DryRunOutcome> {
    let total_backlog = crate::db::count_units_without_preamble(conn)?;
    let sample_ids = crate::db::list_unit_ids_without_preamble(conn, Some(sample_size))?;

    let mut samples = Vec::with_capacity(sample_ids.len());
    let mut sum_in: u64 = 0;
    let mut sum_out: u64 = 0;
    let mocked = client.is_none();

    for id in &sample_ids {
        let unit = crate::db::get_unit(conn, id)?;
        let ctx = build_context_block(conn, id)?;
        let prompt = build_prompt(&unit, &ctx);
        let headline = crate::display::derive_headline(&unit.content);

        let (preamble, input_tokens, output_tokens, sample_mocked) = match client {
            Some(c) => {
                let resp = c.generate_preamble(&prompt)?;
                (resp.preamble, resp.input_tokens, resp.output_tokens, false)
            }
            None => {
                // Estimate: ~1 token per ~4 chars of prompt; output cap is the upper bound.
                let est_in = ((prompt.len() / 4).max(1)) as u32;
                (
                    "[mocked: ANTHROPIC_API_KEY unset]".to_string(),
                    est_in,
                    MAX_OUTPUT_TOKENS,
                    true,
                )
            }
        };

        sum_in += input_tokens as u64;
        sum_out += output_tokens as u64;
        samples.push(SampleResult {
            id: id.clone(),
            headline,
            prompt,
            preamble,
            input_tokens,
            output_tokens,
            mocked: sample_mocked,
        });
    }

    let n = sample_ids.len() as u64;
    let avg_in = sum_in.checked_div(n).unwrap_or(0);
    let avg_out = sum_out.checked_div(n).unwrap_or(0);
    let est_in_total = avg_in.saturating_mul(total_backlog);
    let est_out_total = avg_out.saturating_mul(total_backlog);
    let cost = (est_in_total as f64 * HAIKU_INPUT_USD_PER_MTOK
        + est_out_total as f64 * HAIKU_OUTPUT_USD_PER_MTOK)
        / 1_000_000.0;

    Ok(DryRunOutcome {
        samples,
        total_backlog,
        estimated_input_tokens: est_in_total,
        estimated_output_tokens: est_out_total,
        estimated_cost_usd: cost,
        mocked,
    })
}

/// Aggregated execute report — counts + token + cost totals.
pub struct ExecuteOutcome {
    pub processed: u64,
    pub backlog_before: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// Walk the unprocessed backlog (or a `limit`-capped slice) and persist
/// generated preambles. Honours a per-minute rate cap by sleeping
/// between calls. Errors short-circuit the loop — the caller can re-run
/// to resume because the outer query filters on `IS NULL`.
pub fn run_execute(
    conn: &Connection,
    client: &dyn LlmClient,
    limit: Option<usize>,
    rate_limit_rpm: u32,
) -> Result<ExecuteOutcome> {
    let backlog_before = crate::db::count_units_without_preamble(conn)?;
    let ids = crate::db::list_unit_ids_without_preamble(conn, limit)?;

    // Per-call interval needed to honour the per-minute cap.
    let rpm = rate_limit_rpm.max(1);
    let interval = Duration::from_millis(60_000u64 / rpm as u64);

    let mut processed: u64 = 0;
    let mut sum_in: u64 = 0;
    let mut sum_out: u64 = 0;

    for id in &ids {
        let started = Instant::now();
        let unit = crate::db::get_unit(conn, id)?;
        let ctx = build_context_block(conn, id)?;
        let prompt = build_prompt(&unit, &ctx);

        let resp = client.generate_preamble(&prompt)?;
        crate::db::set_context_preamble(conn, id, &resp.preamble)?;

        sum_in += resp.input_tokens as u64;
        sum_out += resp.output_tokens as u64;
        processed += 1;

        let elapsed = started.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }

    let cost = (sum_in as f64 * HAIKU_INPUT_USD_PER_MTOK
        + sum_out as f64 * HAIKU_OUTPUT_USD_PER_MTOK)
        / 1_000_000.0;

    Ok(ExecuteOutcome {
        processed,
        backlog_before,
        input_tokens: sum_in,
        output_tokens: sum_out,
        cost_usd: cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Unit;
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Deterministic mock: returns numbered preambles, fixed token counts.
    struct MockClient {
        calls: AtomicUsize,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl LlmClient for MockClient {
        fn generate_preamble(&self, _prompt: &str) -> Result<PreambleResp> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(PreambleResp {
                preamble: format!("mock preamble #{n}"),
                input_tokens: 200,
                output_tokens: 25,
            })
        }
    }

    /// Build a fresh in-memory db pinned to user_version=5 + seeded with
    /// `n` units. Matches the production path through `connect()` minus
    /// the on-disk side effects.
    fn make_test_db(n: usize) -> Connection {
        // SAFETY: SIMARIS_HOME isolation isn't needed for in-memory dbs;
        // we drive `initialize()` directly to avoid file IO.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // Run the v0->v5 schema setup manually.
        crate::db::initialize(&conn).unwrap();
        for i in 0..n {
            let id = uuid::Uuid::now_v7().to_string();
            conn.execute(
                "INSERT INTO units (id, content, type, source) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, format!("# Unit {i}\n\nbody {i}"), "fact", "test"],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn build_prompt_includes_atom_and_context() {
        let unit = Unit {
            id: "u1".into(),
            content: "# Test Atom\n\nbody".into(),
            unit_type: "fact".into(),
            source: "test".into(),
            confidence: 1.0,
            verified: false,
            tags: vec![],
            conditions: serde_json::Value::Object(Default::default()),
            created: "now".into(),
            updated: "now".into(),
            archived: false,
        };
        let ctx = ContextBlock {
            parent_slug: Some("parent-stub".into()),
            related: vec!["A".into(), "B".into()],
            part_of_children: vec!["C".into()],
        };
        let p = build_prompt(&unit, &ctx);
        assert!(p.contains("ATOM:"));
        assert!(p.contains("# Test Atom"));
        assert!(p.contains("CONTEXT:"));
        assert!(p.contains("parent-stub"));
        assert!(p.contains("A; B"));
        assert!(p.contains("C"));
    }

    #[test]
    fn dry_run_with_mock_writes_no_preambles() {
        let conn = make_test_db(8);
        let mock = MockClient::new();
        let out = run_dry_run(&conn, 5, Some(&mock)).unwrap();
        assert_eq!(out.samples.len(), 5);
        assert_eq!(mock.call_count(), 5);
        assert_eq!(out.total_backlog, 8);
        // No DB writes — backlog unchanged.
        let after = crate::db::count_units_without_preamble(&conn).unwrap();
        assert_eq!(after, 8);
        // Cost projection > 0 with non-zero token counts.
        assert!(out.estimated_cost_usd > 0.0);
        assert!(!out.mocked);
    }

    #[test]
    fn dry_run_without_client_mocks_samples() {
        let conn = make_test_db(3);
        let out = run_dry_run(&conn, 5, None).unwrap();
        assert_eq!(out.samples.len(), 3);
        assert!(out.samples.iter().all(|s| s.mocked));
        assert!(out.mocked);
    }

    #[test]
    fn execute_writes_preambles_and_is_idempotent() {
        let conn = make_test_db(3);
        let mock = MockClient::new();
        // First pass: processes all three.
        let out = run_execute(&conn, &mock, None, 60_000).unwrap();
        assert_eq!(out.processed, 3);
        assert_eq!(out.backlog_before, 3);
        assert_eq!(crate::db::count_units_without_preamble(&conn).unwrap(), 0);

        // Second pass: backlog is empty, so no LLM calls.
        let pre_calls = mock.call_count();
        let out2 = run_execute(&conn, &mock, None, 60_000).unwrap();
        assert_eq!(out2.processed, 0);
        assert_eq!(mock.call_count(), pre_calls);
    }

    #[test]
    fn execute_respects_limit() {
        let conn = make_test_db(10);
        let mock = MockClient::new();
        let out = run_execute(&conn, &mock, Some(4), 60_000).unwrap();
        assert_eq!(out.processed, 4);
        assert_eq!(crate::db::count_units_without_preamble(&conn).unwrap(), 6);
    }

    #[test]
    fn cost_estimate_matches_haiku_pricing() {
        // 1M input tokens at $0.80 + 1M output at $4.00 = $4.80.
        let cost = (1_000_000.0 * HAIKU_INPUT_USD_PER_MTOK
            + 1_000_000.0 * HAIKU_OUTPUT_USD_PER_MTOK)
            / 1_000_000.0;
        assert!((cost - 4.80).abs() < 1e-9);
    }
}
