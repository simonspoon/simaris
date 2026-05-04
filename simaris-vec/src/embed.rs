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
}
