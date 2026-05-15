//! bge-m3 ollama HTTP embedding client.
//!
//! Talks to a locally-running ollama instance (default `http://localhost:11434`)
//! via the `/api/embeddings` endpoint. Synchronous one-shot per call — callers
//! batch externally if needed.
//!
//! Direct-write Python fallback (`tools/direct_backfill_ollama.py`) preserved
//! per `simaris-m3-redo-2-verdict-2026-05-04` deadlock-workaround caveat. Use
//! the Python path when the Rust subprocess pipe-buffer deadlock recurs (full
//! corpus backfill via simaris EMBED_CMD pipeline).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";
pub const BGE_M3_MODEL: &str = "bge-m3";

#[derive(Debug, Clone)]
pub struct OllamaEmbedClient {
    base_url: String,
    model: String,
}

impl OllamaEmbedClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    /// bge-m3 against a local ollama with default URL.
    pub fn bge_m3() -> Self {
        Self::new(OLLAMA_DEFAULT_URL, BGE_M3_MODEL)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// POST one prompt, return its embedding.
    pub fn embed(&self, prompt: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.base_url);
        let req = EmbedRequest {
            model: &self.model,
            prompt,
        };
        let resp: EmbedResponse = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(&req)?)
            .with_context(|| format!("ollama POST {url}"))?
            .into_json()
            .context("decode ollama embedding response")?;
        if resp.embedding.is_empty() {
            bail!("ollama returned empty embedding (model={})", self.model);
        }
        Ok(resp.embedding)
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

/// Build the text used for semantic embedding from a unit's stored content.
///
/// Strips the leading YAML frontmatter envelope (`---\n...\n---\n`). Frontmatter
/// carries bookkeeping fields — most notably `refs:` (UUID lists) — that
/// pollute embeddings and cause false clusters between unrelated units sharing
/// a refs payload (see task ppjs / pilot smqs sk-C2). The `scope:` value, when
/// present, is preserved and prepended to the body because it's a useful
/// one-line domain summary.
///
/// Pure-prose units (no frontmatter) return content unchanged. Malformed
/// frontmatter (opening fence without closing) is treated as pure prose —
/// embeddings stay stable rather than silently truncating.
///
/// String-based parsing keeps simaris-vec free of a YAML dependency. The
/// `simaris` main crate parses frontmatter properly elsewhere; for the embed
/// pre-processing path the conservative text parser is sufficient and matches
/// the on-disk shape every simaris write emits.
pub fn embed_input(content: &str) -> String {
    const FENCE: &str = "---\n";
    if !content.starts_with(FENCE) {
        return content.to_string();
    }
    let after_open = &content[FENCE.len()..];

    let (yaml_src, body) = if let Some(idx) = after_open.find("\n---\n") {
        (&after_open[..idx], &after_open[idx + "\n---\n".len()..])
    } else if let Some(idx) = after_open.rfind("\n---") {
        let tail = &after_open[idx + "\n---".len()..];
        if tail.is_empty() || tail == "\n" {
            (&after_open[..idx], "")
        } else {
            // Embedded `\n---` is not a closing fence — treat as malformed.
            return content.to_string();
        }
    } else {
        return content.to_string();
    };

    match extract_top_level_scalar(yaml_src, "scope") {
        Some(s) if !s.is_empty() => format!("{s}\n\n{body}"),
        _ => body.to_string(),
    }
}

/// Pull a top-level scalar field value out of a YAML mapping using a
/// line-level scan. Handles plain, double-quoted, and single-quoted scalars.
/// Indented lines and block-scalar (`|` / `>`) values are skipped — they
/// don't appear for `scope:` in practice (always a one-line label).
fn extract_top_level_scalar(yaml: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in yaml.lines() {
        // Top-level keys have no leading whitespace.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let value = rest.trim();
        if value.is_empty() {
            return None;
        }
        // Strip matched outer quotes.
        let unquoted = if value.len() >= 2
            && value.starts_with('"')
            && value.ends_with('"')
        {
            &value[1..value.len() - 1]
        } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
            &value[1..value.len() - 1]
        } else {
            value
        };
        return Some(unquoted.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_holds_config() {
        let c = OllamaEmbedClient::bge_m3();
        assert_eq!(c.model(), BGE_M3_MODEL);
        assert_eq!(c.base_url(), OLLAMA_DEFAULT_URL);
    }

    #[test]
    fn custom_url_and_model() {
        let c = OllamaEmbedClient::new("http://example:1234", "nomic-embed-text-v1.5");
        assert_eq!(c.base_url(), "http://example:1234");
        assert_eq!(c.model(), "nomic-embed-text-v1.5");
    }

    #[test]
    fn embed_input_pure_prose_unchanged() {
        let content = "Just a prose body with no frontmatter.\n";
        assert_eq!(embed_input(content), content);
    }

    #[test]
    fn embed_input_strips_frontmatter_keeps_body() {
        let content = "---\nrefs:\n  - 019d-aaaa\n  - 019d-bbbb\n---\nthe actual prose";
        assert_eq!(embed_input(content), "the actual prose");
    }

    #[test]
    fn embed_input_prepends_scope_when_present() {
        let content = "---\nscope: \"skill reflection — analysis\"\nrefs:\n  - 019d-aaaa\n---\nbody text\n";
        assert_eq!(
            embed_input(content),
            "skill reflection — analysis\n\nbody text\n"
        );
    }

    #[test]
    fn embed_input_scope_unquoted_supported() {
        let content = "---\nscope: plain scope value\nrefs:\n  - x\n---\nbody";
        assert_eq!(embed_input(content), "plain scope value\n\nbody");
    }

    #[test]
    fn embed_input_scope_single_quoted_supported() {
        let content = "---\nscope: 'single quoted'\n---\nbody";
        assert_eq!(embed_input(content), "single quoted\n\nbody");
    }

    #[test]
    fn embed_input_no_scope_returns_body_only() {
        let content = "---\nrefs:\n  - a\n  - b\nevidence: \"some doc\"\n---\nbody only\n";
        assert_eq!(embed_input(content), "body only\n");
    }

    #[test]
    fn embed_input_trailing_fence_without_newline() {
        let content = "---\nscope: trailing\n---";
        // No body, scope preserved.
        assert_eq!(embed_input(content), "trailing\n\n");
    }

    #[test]
    fn embed_input_unclosed_fence_falls_back_to_content() {
        let content = "---\nscope: never closed\nstill yaml-ish";
        assert_eq!(embed_input(content), content);
    }

    #[test]
    fn embed_input_indented_scope_ignored() {
        // `scope:` only inside a nested mapping — not a top-level key, so
        // ignored. Body still returned cleanly.
        let content = "---\nnested:\n  scope: inner\n---\nthe body\n";
        assert_eq!(embed_input(content), "the body\n");
    }

    #[test]
    fn embed_input_refs_uuids_dropped() {
        // The motivating pilot case: 30+ UUIDs in refs no longer reach the
        // embedding input, only scope + prose body do.
        let content = concat!(
            "---\n",
            "scope: \"skill reflection\"\n",
            "refs:\n",
            "  - 019d-aaaa\n",
            "  - 019d-bbbb\n",
            "  - 019d-cccc\n",
            "---\n",
            "real prose here\n",
        );
        let out = embed_input(content);
        assert!(out.starts_with("skill reflection"), "scope missing: {out}");
        assert!(out.contains("real prose here"), "body missing: {out}");
        assert!(!out.contains("019d-aaaa"), "refs leaked: {out}");
        assert!(!out.contains("019d-bbbb"), "refs leaked: {out}");
    }
}
