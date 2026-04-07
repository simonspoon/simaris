use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct DigestResult {
    #[serde(rename = "type")]
    pub unit_type: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub content: Option<String>,
}

/// Get the model to use (env override or default)
fn model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "haiku".to_string())
}

/// Check if claude CLI is available
pub fn check_claude() -> Result<()> {
    let output = Command::new("which")
        .arg("claude")
        .output()
        .context("Failed to check for claude CLI")?;
    if !output.status.success() {
        anyhow::bail!("claude CLI not found. Install it to use digest.");
    }
    Ok(())
}

/// Process a single piece of content through the LLM
pub fn classify(content: &str) -> Result<DigestResult> {
    let prompt = format!(
        r#"You are a knowledge classification system. Analyze the following content and return ONLY a JSON object (no markdown, no explanation, no code fences) with these fields:

- "type": one of: "fact", "procedure", "principle", "preference", "lesson", "idea"
- "tags": array of 2-5 relevant keyword tags (lowercase, single words or short phrases)
- "content": optionally rewrite the content to be clearer and more concise, or null to keep the original

Content to classify:
---
{content}
---

Return ONLY valid JSON. No other text."#
    );

    let output = Command::new("claude")
        .args(["-p", "--model", &model(), &prompt])
        .output()
        .context("Failed to run claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed: {stderr}");
    }

    let response = String::from_utf8_lossy(&output.stdout);
    let response = response.trim();

    // Try to parse, stripping markdown code fences if present
    let json_str = response
        .strip_prefix("```json")
        .or_else(|| response.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(response);

    let result: DigestResult = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse LLM response: {json_str}"))?;

    // Validate type
    let valid_types = [
        "fact",
        "procedure",
        "principle",
        "preference",
        "lesson",
        "idea",
    ];
    if !valid_types.contains(&result.unit_type.as_str()) {
        anyhow::bail!("LLM returned invalid type: {}", result.unit_type);
    }

    Ok(result)
}
