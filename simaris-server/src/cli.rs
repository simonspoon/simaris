//! Shell out to the simaris CLI.
//!
//! Source of truth for data lives in the simaris binary. The server resolves
//! the binary via `SIMARIS_BIN` if set, else falls back to `simaris` on PATH.
//! Every helper captures stdout and parses `--json` output.

use std::ffi::OsString;
use std::process::Command;

use anyhow::{Context, Result};

/// Resolve the simaris binary path. `SIMARIS_BIN` env var wins; otherwise
/// the bare command `simaris` is returned and the OS PATH search is used.
pub fn simaris_bin() -> OsString {
    std::env::var_os("SIMARIS_BIN").unwrap_or_else(|| OsString::from("simaris"))
}

/// Spawn `simaris <args...>`, capture stdout, parse it as JSON.
///
/// Caller is responsible for adding `--json` to `args` when the underlying
/// command needs it. Stderr is forwarded into the error context on non-zero
/// exit.
pub fn run_simaris(args: &[&str]) -> Result<serde_json::Value> {
    run_simaris_inner(args)
}

/// Owned-string variant. Convenience for callers that build args dynamically
/// (route handlers stitching CLI flags from query/body params).
pub fn run_simaris_owned(args: &[String]) -> Result<serde_json::Value> {
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_simaris_inner(&refs)
}

fn run_simaris_inner(args: &[&str]) -> Result<serde_json::Value> {
    let bin = simaris_bin();
    let output = Command::new(&bin)
        .args(args)
        .output()
        .with_context(|| format!("spawn {:?} {:?}", bin, args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "simaris {:?} exited {}: {}",
            args,
            output.status,
            stderr.trim()
        );
    }

    let stdout = std::str::from_utf8(&output.stdout)
        .with_context(|| format!("simaris {:?} stdout not utf-8", args))?;

    serde_json::from_str(stdout)
        .with_context(|| format!("simaris {:?} stdout not json: {}", args, stdout))
}
