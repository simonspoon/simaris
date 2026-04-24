//! Frontmatter parser — read-side (P0) + builder (P1).
//!
//! Recognises a leading YAML frontmatter block delimited by `---\n` fences at
//! byte 0. On malformed YAML, falls back to treating the entire content as
//! body so `show` never crashes.
//!
//! The P1 builder emits deterministic YAML from CLI flags. Field order matches
//! the per-type schema order in the frontmatter-p1 spec — scalar before list,
//! trigger before check before caveat, etc. Ordering matters for diff
//! readability.

use anyhow::{Result, bail};
use serde_yml::Value;

const FENCE: &str = "---\n";

/// A single frontmatter field value — either a scalar string or an ordered
/// list of strings. Empty-string scalars and empty lists are skipped by the
/// builder.
pub enum FieldValue {
    Scalar(String),
    List(Vec<String>),
}

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

/// Build a YAML frontmatter block (fences included) from an ordered list of
/// `(key, value)` fields. Returns `None` if every field was absent/empty.
///
/// Output shape:
/// ```text
/// ---
/// key: value
/// list_key:
///   - item1
///   - item2
/// ---
/// ```
///
/// Closing fence ends with `\n` so callers can concat a body directly.
/// Scalars are emitted as YAML plain-or-quoted strings via serde_yml so
/// awkward values (leading `#`, colons, etc.) stay legal.
pub fn build_frontmatter(fields: &[(&str, FieldValue)]) -> Option<String> {
    let mut mapping = serde_yml::Mapping::new();
    for (key, val) in fields {
        match val {
            FieldValue::Scalar(s) if !s.is_empty() => {
                mapping.insert(Value::String((*key).to_string()), Value::String(s.clone()));
            }
            FieldValue::List(items) if !items.is_empty() => {
                let seq: Vec<Value> = items.iter().cloned().map(Value::String).collect();
                mapping.insert(Value::String((*key).to_string()), Value::Sequence(seq));
            }
            _ => {} // skip empty
        }
    }
    if mapping.is_empty() {
        return None;
    }
    let yaml = serde_yml::to_string(&Value::Mapping(mapping)).ok()?;
    // serde_yml always terminates with '\n'.
    Some(format!("{FENCE}{yaml}{}", "---\n"))
}

/// Validate that a user-supplied `--from-file` payload either has no
/// frontmatter fences (pure prose is fine) or has a valid YAML mapping
/// frontmatter block. Returns Err with a diagnostic on malformed YAML or
/// non-mapping roots.
pub fn validate_from_file(content: &str) -> Result<()> {
    if !content.starts_with(FENCE) {
        return Ok(()); // pure prose — fine
    }
    let after_open = &content[FENCE.len()..];
    let yaml_src = if let Some(idx) = after_open.find("\n---\n") {
        &after_open[..idx]
    } else if let Some(idx) = after_open.rfind("\n---") {
        let tail = &after_open[idx + "\n---".len()..];
        if tail.is_empty() || tail == "\n" {
            &after_open[..idx]
        } else {
            bail!("malformed frontmatter: opening `---` fence without closing `---` fence");
        }
    } else {
        bail!("malformed frontmatter: opening `---` fence without closing `---` fence");
    };

    match serde_yml::from_str::<Value>(yaml_src) {
        Ok(v) if v.is_mapping() => Ok(()),
        Ok(_) => bail!("malformed frontmatter: YAML root must be a mapping, got scalar or list"),
        Err(e) => bail!("malformed frontmatter YAML: {e}"),
    }
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

    #[test]
    fn build_fm_all_empty_returns_none() {
        let fields: Vec<(&str, FieldValue)> = vec![
            ("trigger", FieldValue::Scalar(String::new())),
            ("prereq", FieldValue::List(vec![])),
        ];
        assert!(build_frontmatter(&fields).is_none());
    }

    #[test]
    fn build_fm_scalar_preserves_order() {
        let fields = vec![
            ("trigger", FieldValue::Scalar("weekly".into())),
            ("check", FieldValue::Scalar("report".into())),
            ("caveat", FieldValue::Scalar("edge case".into())),
        ];
        let s = build_frontmatter(&fields).unwrap();
        assert!(s.starts_with("---\n"), "opening fence: {s}");
        assert!(s.ends_with("---\n"), "closing fence: {s}");
        let tp = s.find("trigger").unwrap();
        let cp = s.find("check").unwrap();
        let xp = s.find("caveat").unwrap();
        assert!(tp < cp && cp < xp, "ordering broken: {s}");
    }

    #[test]
    fn build_fm_list_becomes_yaml_sequence() {
        let fields = vec![
            (
                "prereq",
                FieldValue::List(vec!["@one".into(), "@two".into()]),
            ),
            ("refs", FieldValue::List(vec!["@proposal".into()])),
        ];
        let s = build_frontmatter(&fields).unwrap();
        // parse roundtrip — confirms the builder emits legal YAML mapping
        let p = parse(&s);
        assert!(p.frontmatter.is_some(), "roundtrip: {s}");
    }

    #[test]
    fn build_fm_skips_empty_fields_but_keeps_populated() {
        let fields = vec![
            ("trigger", FieldValue::Scalar("weekly".into())),
            ("check", FieldValue::Scalar(String::new())),
            ("prereq", FieldValue::List(vec![])),
            ("refs", FieldValue::List(vec!["@proposal".into()])),
        ];
        let s = build_frontmatter(&fields).unwrap();
        assert!(s.contains("trigger:"), "has trigger: {s}");
        assert!(s.contains("refs:"), "has refs: {s}");
        assert!(!s.contains("check:"), "skipped check: {s}");
        assert!(!s.contains("prereq:"), "skipped prereq: {s}");
    }

    #[test]
    fn validate_from_file_pure_prose_ok() {
        assert!(validate_from_file("just prose\n").is_ok());
    }

    #[test]
    fn validate_from_file_valid_fm_ok() {
        assert!(validate_from_file("---\ntitle: hello\n---\nbody\n").is_ok());
    }

    #[test]
    fn validate_from_file_malformed_yaml_errs() {
        let err = validate_from_file("---\n: : bad yaml :: :\n---\nbody\n").unwrap_err();
        assert!(
            format!("{err}").contains("malformed frontmatter"),
            "msg: {err}"
        );
    }

    #[test]
    fn validate_from_file_unclosed_fence_errs() {
        let err = validate_from_file("---\ntitle: hello\nbody with no close\n").unwrap_err();
        assert!(format!("{err}").contains("malformed"), "msg: {err}");
    }

    #[test]
    fn validate_from_file_non_mapping_root_errs() {
        let err = validate_from_file("---\n- just a list\n- of items\n---\nbody\n").unwrap_err();
        assert!(format!("{err}").contains("mapping"), "msg: {err}");
    }
}
