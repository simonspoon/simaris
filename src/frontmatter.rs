//! Frontmatter parser — read-side only (P0).
//!
//! Recognises a leading YAML frontmatter block delimited by `---\n` fences at
//! byte 0. On malformed YAML, falls back to treating the entire content as
//! body so `show` never crashes.

use serde_yml::Value;

const FENCE: &str = "---\n";

/// Parsed view of unit content.
pub struct Parsed<'a> {
    pub frontmatter: Option<Value>,
    pub body: &'a str,
}

/// Parse content into optional frontmatter + body.
///
/// Rules:
/// - must start with exactly `---\n` at byte 0
/// - closing `\n---\n` ends the block (or trailing `\n---` at EOF)
/// - malformed YAML → `frontmatter = None`, body = full content
/// - absent fences → `frontmatter = None`, body = full content
pub fn parse(content: &str) -> Parsed<'_> {
    if !content.starts_with(FENCE) {
        return Parsed {
            frontmatter: None,
            body: content,
        };
    }
    let after_open = &content[FENCE.len()..];

    // Find closing fence. Accept either "\n---\n" mid-document or a trailing
    // "\n---" at end-of-file (no terminating newline).
    let (yaml_src, body) = if let Some(idx) = after_open.find("\n---\n") {
        let yaml = &after_open[..idx];
        let body_start = idx + "\n---\n".len();
        (yaml, &after_open[body_start..])
    } else if let Some(idx) = after_open.rfind("\n---") {
        // Only treat as closing fence if it sits at EOF (nothing after but
        // optional newline). This avoids matching "\n---" embedded in body.
        let tail = &after_open[idx + "\n---".len()..];
        if tail.is_empty() || tail == "\n" {
            (&after_open[..idx], "")
        } else {
            return Parsed {
                frontmatter: None,
                body: content,
            };
        }
    } else {
        return Parsed {
            frontmatter: None,
            body: content,
        };
    };

    match serde_yml::from_str::<Value>(yaml_src) {
        Ok(v) if v.is_mapping() => Parsed {
            frontmatter: Some(v),
            body,
        },
        // Non-mapping YAML (scalar, sequence) is not a useful frontmatter
        // shape for rendering — treat as malformed, fall through to body.
        _ => Parsed {
            frontmatter: None,
            body: content,
        },
    }
}

/// Render top-level frontmatter keys as markdown lines.
///
/// - scalars → `**key:** value`
/// - short sequences (all scalars, ≤3 items) → `**key:** a, b, c`
/// - longer or mixed sequences → bulleted list under header
/// - nested mappings → skipped in P0
pub fn render_markdown(fm: &Value) -> String {
    let Some(map) = fm.as_mapping() else {
        return String::new();
    };

    let mut out = String::new();
    for (k, v) in map {
        let key = match k.as_str() {
            Some(s) => s,
            None => continue,
        };
        match v {
            Value::Null => {
                out.push_str(&format!("**{key}:**\n"));
            }
            Value::Bool(b) => {
                out.push_str(&format!("**{key}:** {b}\n"));
            }
            Value::Number(n) => {
                out.push_str(&format!("**{key}:** {n}\n"));
            }
            Value::String(s) => {
                out.push_str(&format!("**{key}:** {s}\n"));
            }
            Value::Sequence(seq) => {
                let all_scalar = seq.iter().all(is_simple_scalar);
                if all_scalar && seq.len() <= 3 {
                    let joined = seq
                        .iter()
                        .map(scalar_to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("**{key}:** {joined}\n"));
                } else if all_scalar {
                    out.push_str(&format!("**{key}:**\n"));
                    for item in seq {
                        out.push_str(&format!("- {}\n", scalar_to_string(item)));
                    }
                } else {
                    // mixed: render simple scalars, skip nested
                    out.push_str(&format!("**{key}:**\n"));
                    for item in seq {
                        if is_simple_scalar(item) {
                            out.push_str(&format!("- {}\n", scalar_to_string(item)));
                        }
                    }
                }
            }
            // nested mappings skipped in P0
            Value::Mapping(_) => {}
            Value::Tagged(_) => {}
        }
    }
    out
}

fn is_simple_scalar(v: &Value) -> bool {
    matches!(
        v,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_frontmatter() {
        let p = parse("just prose");
        assert!(p.frontmatter.is_none());
        assert_eq!(p.body, "just prose");
    }

    #[test]
    fn parse_roundtrip() {
        let src = "---\ntitle: hello\ntags: [a, b]\n---\nbody text\n";
        let p = parse(src);
        assert!(p.frontmatter.is_some());
        assert_eq!(p.body, "body text\n");
    }

    #[test]
    fn parse_malformed_falls_back() {
        let src = "---\n: : invalid yaml :: :\n---\nbody\n";
        let p = parse(src);
        assert!(p.frontmatter.is_none());
        assert_eq!(p.body, src);
    }

    #[test]
    fn render_scalar() {
        let src = "---\ntitle: hello\n---\n";
        let p = parse(src);
        let md = render_markdown(p.frontmatter.as_ref().unwrap());
        assert!(md.contains("**title:** hello"));
    }

    #[test]
    fn render_short_list_inline() {
        let src = "---\ntags: [a, b, c]\n---\n";
        let p = parse(src);
        let md = render_markdown(p.frontmatter.as_ref().unwrap());
        assert!(md.contains("**tags:** a, b, c"));
    }

    #[test]
    fn render_long_list_bulleted() {
        let src = "---\nitems: [a, b, c, d]\n---\n";
        let p = parse(src);
        let md = render_markdown(p.frontmatter.as_ref().unwrap());
        assert!(md.contains("**items:**"));
        assert!(md.contains("- a"));
        assert!(md.contains("- d"));
    }
}
