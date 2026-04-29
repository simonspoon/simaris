use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct DigestUnit {
    #[serde(rename = "type")]
    pub unit_type: String,
    pub content: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_overview: bool,
}

#[derive(Debug, Deserialize)]
pub struct DigestResult {
    pub units: Vec<DigestUnit>,
}

/// Get the model to use (env override or default)
fn model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "sonnet".to_string())
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
        r#"You are a knowledge extraction system. Break the following content into discrete knowledge units.

Rules:
- First unit MUST be an overview: 1-3 sentences summarizing the whole thing. Mark it "is_overview": true.
- Then extract each logical section, rule, principle, or distinct procedure as its own unit.
- Each unit must be CONCISE — no prose, no filler. Distill to the essential information.
- Each unit gets its own type: "fact", "procedure", "principle", "preference", "lesson", "idea", or "aspect".
- Procedures should be step-by-step, not paragraphs.
- Principles/rules should be standalone statements that make sense without context.
- 2-5 tags per unit (lowercase).
- Aim for 3-8 units total. Don't over-split — group related small items.

Return ONLY a JSON object (no markdown, no code fences):
{{
  "units": [
    {{
      "type": "procedure",
      "content": "concise overview here",
      "tags": ["tag1", "tag2"],
      "is_overview": true
    }},
    {{
      "type": "principle",
      "content": "extracted principle or rule",
      "tags": ["tag1", "tag2"]
    }}
  ]
}}

Content to process:
---
{content}
---

Return ONLY valid JSON. No other text."#
    );

    // --output-format json wraps the model reply in a structured envelope
    // {"result": "...", ...}. The envelope itself is always well-formed JSON,
    // which makes parsing robust against any preamble the UserPromptSubmit
    // hook injects into stdout (e.g. "## Simaris procedures..."). The model's
    // reply lives in `.result` and may still arrive with code fences or
    // surrounding prose — we extract the JSON object below.
    //
    // We deliberately do NOT pass --bare here. --bare disables OAuth/keychain
    // auth, which is how this binary's user authenticates. Falling back to
    // ANTHROPIC_API_KEY would silently start charging real API spend.
    let output = Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "json",
            "--model",
            &model(),
            &prompt,
        ])
        .output()
        .context("Failed to run claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed: {stderr}");
    }

    let envelope_raw = String::from_utf8_lossy(&output.stdout);

    // Outer envelope: {"result": "<inner content>", ...}. Inner content is the
    // model's reply — should be JSON {"units": [...]} but tolerate fence
    // wrappers and surrounding prose.
    #[derive(Deserialize)]
    struct Envelope {
        result: String,
    }

    let envelope: Envelope = serde_json::from_str(envelope_raw.trim()).with_context(|| {
        let preview: String = envelope_raw.chars().take(200).collect();
        format!("Failed to parse claude --output-format json envelope; stdout preview: {preview}")
    })?;

    let inner = envelope.result.trim();

    // Step 1: drop common fence wrappers. Step 2: locate the first `{` and the
    // last `}` and extract that span — survives "Here's the JSON: {...}\nDone"
    // shaped responses without needing a real parser.
    let defenced = inner
        .strip_prefix("```json")
        .or_else(|| inner.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(inner);

    let json_str = match (defenced.find('{'), defenced.rfind('}')) {
        (Some(lo), Some(hi)) if hi >= lo => &defenced[lo..=hi],
        _ => defenced,
    };

    let result: DigestResult = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse LLM response: {json_str}"))?;

    // Validate all unit types
    let valid_types = [
        "fact",
        "procedure",
        "principle",
        "preference",
        "lesson",
        "idea",
        "aspect",
    ];
    for unit in &result.units {
        if !valid_types.contains(&unit.unit_type.as_str()) {
            anyhow::bail!("LLM returned invalid type: {}", unit.unit_type);
        }
    }

    if result.units.is_empty() {
        anyhow::bail!("LLM returned no units");
    }

    Ok(result)
}
