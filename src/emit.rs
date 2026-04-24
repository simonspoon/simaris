use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::db;

const MANAGED_MARKER: &str = "simaris-managed: true";

#[derive(Debug, Default, Serialize)]
pub struct EmitResult {
    pub written: Vec<String>,
    pub swept: Vec<String>,
    pub skipped_uuids: Vec<String>,
}

/// Resolve the target directory for Claude Code agent files.
///
/// Honors `SIMARIS_CLAUDE_AGENTS_DIR` when set (used by tests to redirect
/// writes away from the real `~/.claude/agents/`). Falls back to
/// `$HOME/.claude/agents/`.
pub fn claude_agents_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("SIMARIS_CLAUDE_AGENTS_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".claude").join("agents"))
}

/// Extract the first paragraph of a markdown body.
///
/// A paragraph here is the first non-empty run of text up to a blank line.
/// Internal line breaks are collapsed to spaces so the result fits on one
/// YAML line.
fn first_paragraph(content: &str) -> String {
    let mut paragraph_lines: Vec<&str> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            if !paragraph_lines.is_empty() {
                break;
            }
            continue;
        }
        paragraph_lines.push(line.trim());
    }
    paragraph_lines.join(" ")
}

/// Wrap a string in single quotes for safe inclusion as a YAML scalar.
/// Any embedded single quote is doubled per the YAML single-quoted style.
fn yaml_single_quote(s: &str) -> String {
    let escaped = s.replace('\'', "''");
    format!("'{escaped}'")
}

/// Render a Claude Code agent markdown file: YAML frontmatter + verbatim body.
fn render_agent_file(slug: &str, content: &str) -> String {
    let description = yaml_single_quote(&first_paragraph(content));
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {slug}\n"));
    out.push_str(&format!("description: {description}\n"));
    out.push_str(&format!("{MANAGED_MARKER}\n"));
    out.push_str("---\n");
    out.push_str(content);
    if !content.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Parse a file's YAML frontmatter and return its `name` value when the
/// `simaris-managed: true` marker is present. Returns `None` for unmanaged
/// files, malformed frontmatter, or read errors.
fn parse_managed_slug(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name: Option<String> = None;
    let mut managed = false;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            name = Some(rest.trim().to_string());
        } else if line.trim() == MANAGED_MARKER {
            managed = true;
        }
    }
    if managed { name } else { None }
}

/// Emit every aspect unit with at least one slug to the given target directory.
/// Overwrites existing managed files, sweeps stale managed files, leaves
/// hand-authored files untouched. Missing directories are created.
pub fn emit_claude_code_aspects(conn: &Connection, target_dir: &Path) -> Result<EmitResult> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create {}", target_dir.display()))?;

    let aspects = db::list_units(conn, Some("aspect"))?;

    let mut written: Vec<String> = Vec::new();
    let mut skipped_uuids: Vec<String> = Vec::new();
    let mut live_slugs: HashSet<String> = HashSet::new();

    for aspect in &aspects {
        let slugs = db::get_slugs_for_unit(conn, &aspect.id)?;
        if slugs.is_empty() {
            skipped_uuids.push(aspect.id.clone());
            continue;
        }
        for slug in slugs {
            let file = target_dir.join(format!("{slug}.md"));
            let body = render_agent_file(&slug, &aspect.content);
            fs::write(&file, body)
                .with_context(|| format!("Failed to write {}", file.display()))?;
            live_slugs.insert(slug.clone());
            written.push(slug);
        }
    }
    written.sort();
    skipped_uuids.sort();

    let mut swept: Vec<String> = Vec::new();
    for entry in fs::read_dir(target_dir)
        .with_context(|| format!("Failed to read {}", target_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(managed_slug) = parse_managed_slug(&path) else {
            continue;
        };
        if live_slugs.contains(&managed_slug) {
            continue;
        }
        fs::remove_file(&path).with_context(|| format!("Failed to remove {}", path.display()))?;
        swept.push(managed_slug);
    }
    swept.sort();

    Ok(EmitResult {
        written,
        swept,
        skipped_uuids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_paragraph_single_line() {
        assert_eq!(first_paragraph("hello world"), "hello world");
    }

    #[test]
    fn first_paragraph_stops_at_blank() {
        let s = "one\ntwo\n\nthree";
        assert_eq!(first_paragraph(s), "one two");
    }

    #[test]
    fn first_paragraph_skips_leading_blanks() {
        let s = "\n\nhello\n";
        assert_eq!(first_paragraph(s), "hello");
    }

    #[test]
    fn first_paragraph_empty() {
        assert_eq!(first_paragraph(""), "");
    }

    #[test]
    fn yaml_single_quote_escapes_inner() {
        assert_eq!(yaml_single_quote("it's fine"), "'it''s fine'");
    }

    #[test]
    fn render_agent_file_shape() {
        let out = render_agent_file("my-agent", "# Title\n\nBody line");
        assert!(out.starts_with("---\n"));
        assert!(out.contains("\nname: my-agent\n"));
        assert!(out.contains("\ndescription: '# Title'\n"));
        assert!(out.contains("\nsimaris-managed: true\n"));
        assert!(out.contains("\n---\n# Title\n\nBody line\n"));
    }

    #[test]
    fn parse_managed_slug_detects_marker() {
        let dir = std::env::temp_dir().join(format!(
            "simaris-emit-test-{}-{}",
            std::process::id(),
            "managed"
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("managed.md");
        fs::write(
            &path,
            "---\nname: managed\ndescription: 'x'\nsimaris-managed: true\n---\nbody\n",
        )
        .unwrap();
        assert_eq!(parse_managed_slug(&path), Some("managed".to_string()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_managed_slug_ignores_unmanaged() {
        let dir = std::env::temp_dir().join(format!(
            "simaris-emit-test-{}-{}",
            std::process::id(),
            "unmanaged"
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hand.md");
        fs::write(&path, "---\nname: hand\n---\nbody\n").unwrap();
        assert_eq!(parse_managed_slug(&path), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_managed_slug_ignores_no_frontmatter() {
        let dir = std::env::temp_dir().join(format!(
            "simaris-emit-test-{}-{}",
            std::process::id(),
            "nofm"
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.md");
        fs::write(&path, "just some content\n").unwrap();
        assert_eq!(parse_managed_slug(&path), None);
        fs::remove_dir_all(&dir).ok();
    }
}
