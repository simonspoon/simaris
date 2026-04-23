//! Write-time body-size signal.
//!
//! Measures the UTF-8 byte length of unit body content at `add` / `edit` and:
//! - Warns (stderr) above `SIMARIS_WARN_BYTES` (default 2048).
//! - Rejects (non-zero exit via `anyhow::bail!`) above `SIMARIS_HARD_BYTES`
//!   (default 8192), unless the caller passed `--force`.
//!
//! Flow-sequence escape hatches:
//! - Tag `flow` (shell recipes, ordered procedures) bypasses all checks.
//! - `--flow` CLI flag bypasses all checks.
//!
//! Thresholds are defaults today; Story 4 (task `yyvb`) calibrates them from
//! measured corpora. Authors can always override via env vars.
//!
//! Warnings cite the `split-ruleset` slug (Story 2 / task `khvc` / `rrkx`).

use anyhow::{Result, bail};

/// Default warn threshold — body bytes above this trigger stderr warning.
/// Placeholder pending Story 4 calibration.
pub const DEFAULT_WARN_BYTES: usize = 2048;

/// Default hard threshold — body bytes above this reject without `--force`.
/// Placeholder pending Story 4 calibration.
pub const DEFAULT_HARD_BYTES: usize = 8192;

/// Slug cited in warnings. Points at the atomicity/split ruleset procedure
/// (Story 2). Cited as a literal string — works even before the slug is bound.
const CITE_SLUG: &str = "split-ruleset";

/// Read warn threshold from `SIMARIS_WARN_BYTES` env var, falling back to
/// `DEFAULT_WARN_BYTES` on unset or unparseable values.
pub fn warn_threshold() -> usize {
    std::env::var("SIMARIS_WARN_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_WARN_BYTES)
}

/// Read hard threshold from `SIMARIS_HARD_BYTES` env var, falling back to
/// `DEFAULT_HARD_BYTES` on unset or unparseable values.
pub fn hard_threshold() -> usize {
    std::env::var("SIMARIS_HARD_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_HARD_BYTES)
}

/// Check a body against write-time size thresholds.
///
/// Returns `Ok(())` when the write should proceed. Emits an `eprintln!`
/// warning when the body is oversized but allowed. Returns `Err` (non-zero
/// exit) when the body exceeds the hard threshold and `force` is false.
///
/// `flow == true` OR `tags` containing the literal `"flow"` short-circuits
/// all checks (flow sequences legitimately exceed the budget).
pub fn check_size<S: AsRef<str>>(
    content: &str,
    tags: &[S],
    flow: bool,
    force: bool,
) -> Result<()> {
    if flow || tags.iter().any(|t| t.as_ref() == "flow") {
        return Ok(());
    }

    let bytes = content.len();
    let warn = warn_threshold();
    let hard = hard_threshold();

    if bytes > hard {
        if force {
            eprintln!(
                "simaris: warning — body {bytes} bytes exceeds hard threshold {hard} bytes \
                 (override via --force); see `{CITE_SLUG}` for decomposition rules"
            );
            return Ok(());
        }
        bail!(
            "body {bytes} bytes exceeds hard threshold {hard} bytes; \
             see `{CITE_SLUG}` for decomposition rules — re-run with --force to override, \
             or tag `flow` for legitimate flow sequences"
        );
    }

    if bytes > warn {
        eprintln!(
            "simaris: warning — body {bytes} bytes exceeds warn threshold {warn} bytes; \
             see `{CITE_SLUG}` for decomposition rules"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: these tests touch process env vars. They run single-threaded by
    // default under `cargo test` on the same binary only when isolated; we
    // avoid collisions by only setting vars in tests that strictly need them
    // and by restoring defaults afterwards. For cross-test safety, prefer
    // the integration tests (separate process per run).

    #[test]
    fn empty_below_warn() {
        let tags: Vec<&str> = vec![];
        assert!(check_size("", &tags, false, false).is_ok());
    }

    #[test]
    fn small_below_warn() {
        let tags: Vec<&str> = vec![];
        assert!(check_size("short body", &tags, false, false).is_ok());
    }

    #[test]
    fn flow_tag_bypasses_hard_reject() {
        let body = "x".repeat(DEFAULT_HARD_BYTES + 500);
        let tags = vec!["flow".to_string()];
        assert!(check_size(&body, &tags, false, false).is_ok());
    }

    #[test]
    fn flow_flag_bypasses_hard_reject() {
        let body = "x".repeat(DEFAULT_HARD_BYTES + 500);
        let tags: Vec<&str> = vec![];
        assert!(check_size(&body, &tags, true, false).is_ok());
    }

    #[test]
    fn force_overrides_hard_reject() {
        let body = "x".repeat(DEFAULT_HARD_BYTES + 500);
        let tags: Vec<&str> = vec![];
        assert!(check_size(&body, &tags, false, true).is_ok());
    }

    #[test]
    fn hard_reject_bails_without_force() {
        let body = "x".repeat(DEFAULT_HARD_BYTES + 500);
        let tags: Vec<&str> = vec![];
        let err = check_size(&body, &tags, false, false).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("hard threshold"), "msg={msg}");
        assert!(msg.contains("split-ruleset"), "msg={msg}");
    }
}
