//! `simaris rewrite <id>` — editor-driven structured-frontmatter authoring.
//!
//! Opens `$EDITOR` (override: `SIMARIS_EDITOR`; fallback: `vi`) with a
//! type-aware skeleton pre-filled with the unit's existing content. After
//! the editor exits, strips leading `#` header-comment lines, validates the
//! frontmatter (if any), and writes the new content back — preserving
//! identity (id, tags, links, marks, slugs, created).
//!
//! Buffer composition rules:
//! | unit state | `--template-only` | buffer |
//! |---|---|---|
//! | structured | no  | existing fm + body verbatim |
//! | prose      | no  | empty skeleton for type + existing body below |
//! | any        | yes | skeleton only |
//!
//! Exit semantics:
//! - empty buffer after strip → abort, unit unchanged, exit 0
//! - no substantive change     → no-op, exit 0
//! - substantive + invalid YAML→ reject, unit unchanged, exit non-zero
//! - substantive + valid       → write, unit updated, exit 0

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::db;
use crate::frontmatter;

/// RAII cleanup guard for the rewrite temp file.
struct TempFile {
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Entry point. Resolves id/slug, composes buffer, spawns editor, validates,
/// writes. See module doc for semantics.
pub fn run(conn: &Connection, id_or_slug: &str, template_only: bool) -> Result<()> {
    let id = db::resolve_id(conn, id_or_slug)?;
    let unit = db::get_unit(conn, &id)?;

    let buffer = compose_buffer(&unit, template_only);
    // Baseline for no-op detection: the body we *wrote* to the temp file,
    // post-header-strip. Compared against the post-edit stripped buffer.
    // Comparing against unit.content is wrong for prose units — the composed
    // buffer prepends a skeleton, so a no-edit cancel would still diff
    // against the DB body-only and trigger a destructive write.
    let initial_stripped = strip_header_comments(&buffer);

    // Temp file path: $TMPDIR/simaris-rewrite-<shortid>-<pid>.md
    let short = if unit.id.len() >= 8 {
        &unit.id[..8]
    } else {
        &unit.id[..]
    };
    let pid = std::process::id();
    let temp_path = std::env::temp_dir().join(format!("simaris-rewrite-{short}-{pid}.md"));
    let _guard = TempFile {
        path: temp_path.clone(),
    };

    // Seed file with composed buffer.
    {
        let mut f = std::fs::File::create(&temp_path)
            .with_context(|| format!("failed to create temp file at {}", temp_path.display()))?;
        f.write_all(buffer.as_bytes())
            .with_context(|| format!("failed to write temp file {}", temp_path.display()))?;
    }

    invoke_editor(&temp_path)?;

    let edited = std::fs::read_to_string(&temp_path)
        .with_context(|| format!("failed to read temp file {}", temp_path.display()))?;
    let stripped = strip_header_comments(&edited);
    let trimmed = stripped.trim();

    // Empty buffer → abort.
    if trimmed.is_empty() {
        eprintln!("rewrite aborted: empty buffer, unit unchanged");
        return Ok(());
    }

    // No substantive change → no-op. Compare final buffer to the baseline we
    // seeded (post-header-strip), not to DB content — see `initial_stripped`.
    if buffers_equal(&initial_stripped, &stripped) {
        eprintln!("rewrite no-op: no changes, unit unchanged");
        return Ok(());
    }

    // Validate frontmatter (if any).
    frontmatter::validate(&stripped, &unit.unit_type)
        .context("rewrite rejected: invalid frontmatter")?;

    // Write through — preserves identity (db::update_unit on same id).
    db::update_unit(conn, &id, Some(&stripped), None, None, None)?;
    eprintln!("rewrote unit {id}");
    Ok(())
}

/// Normalize two candidate buffers for equality: trim trailing whitespace on
/// each line and strip trailing blank lines. Lets editors that touch the
/// file but don't change semantic content count as no-op.
fn buffers_equal(a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        let mut lines: Vec<&str> = s.lines().map(|l| l.trim_end()).collect();
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        while lines.first().map(|l| l.is_empty()).unwrap_or(false) {
            lines.remove(0);
        }
        lines.join("\n")
    };
    norm(a) == norm(b)
}

/// Compose the editor buffer per the spec table. Adds a leading
/// `#`-commented header block (stripped before validation) explaining
/// identity + abort conventions.
pub fn compose_buffer(unit: &db::Unit, template_only: bool) -> String {
    let header = format!(
        "# simaris rewrite -- id: {}  type: {}\n\
         # Save + quit to apply. Empty file to abort.\n\
         # Comment lines starting `#` at top stripped automatically.\n\
         \n",
        unit.id, unit.unit_type
    );

    let body = if template_only {
        skeleton_for(&unit.unit_type)
    } else {
        let parsed = frontmatter::parse(&unit.content);
        if parsed.frontmatter.is_some() {
            // Structured: keep verbatim.
            unit.content.clone()
        } else {
            // Prose: empty skeleton + existing body.
            let skel = skeleton_for(&unit.unit_type);
            if skel.is_empty() {
                unit.content.clone()
            } else {
                format!("{}{}", skel, unit.content)
            }
        }
    };

    format!("{header}{body}")
}

/// Return a type-aware empty skeleton (YAML frontmatter only, body goes
/// below). Types without a schema (preference, idea) return an empty string.
pub fn skeleton_for(unit_type: &str) -> String {
    match unit_type {
        "procedure" => "---\ntrigger: \"\"\ncheck: \"\"\ncadence: \"\"\ncaveat: \"\"\nprereq: []\nrefs: []\n---\n\n".to_string(),
        "aspect" => "---\nrole: \"\"\ndispatches_to: []\nhandles_directly: []\nrefs: []\n---\n\n".to_string(),
        "fact" => "---\nscope: \"\"\nevidence: \"\"\nrefs: []\n---\n\n".to_string(),
        "principle" => "---\ntension: \"\"\nrefs: []\n---\n\n".to_string(),
        "lesson" => "---\ncontext: \"\"\nscope: \"\"\nrefs: []\n---\n\n".to_string(),
        // preference, idea — no schema
        _ => String::new(),
    }
}

/// Strip leading `#` comment lines from a buffer, plus the separator blank
/// lines between the header block and the real content. Stops at the first
/// non-blank, non-`#` line. Leaves body `#` headers (`# Heading`) that sit
/// after the blank line untouched.
///
/// The separator-blank strip matters: frontmatter detection requires the
/// `---` fence at byte 0 of stored content, so any stray `\n` prefix from
/// the header separator would cause `has_frontmatter` to miss the block.
pub fn strip_header_comments(content: &str) -> String {
    let mut byte_skip = 0usize;
    let mut in_header = true;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if in_header && trimmed.starts_with('#') && !trimmed.starts_with("---") {
            byte_skip += line.len() + 1; // +1 for the '\n' that .lines() strips
        } else if in_header && trimmed.is_empty() {
            // Consume separator blank lines between header and body.
            byte_skip += line.len() + 1;
        } else {
            // First non-header, non-blank line ends the strip.
            break;
        }
        // Once we see a blank line, any further `#` lines are body headers,
        // not header comments — stop treating them as strippable.
        if line.trim_start().is_empty() {
            in_header = false;
        }
    }
    if byte_skip >= content.len() {
        return String::new();
    }
    content[byte_skip..].to_string()
}

/// Spawn the editor on `path`. Resolves `SIMARIS_EDITOR` > `EDITOR` > `vi`.
/// The resolved command is run via `sh -c "<cmd> <path>"` so multi-word
/// editor commands (`nvim --clean`) and test harnesses
/// (`sh -c 'cat fixture > $0'`) both work.
pub fn invoke_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("SIMARIS_EDITOR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());

    // Quote the path with single quotes and escape any embedded single quotes.
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp file path is not valid UTF-8"))?;
    let quoted = format!("'{}'", path_str.replace('\'', "'\\''"));
    let full_cmd = format!("{editor} {quoted}");

    let status = Command::new("sh")
        .arg("-c")
        .arg(&full_cmd)
        .status()
        .with_context(|| format!("failed to spawn editor `{editor}`"))?;

    if !status.success() {
        bail!(
            "editor `{editor}` exited with status {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string())
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_unit(id: &str, ty: &str, content: &str) -> db::Unit {
        db::Unit {
            id: id.to_string(),
            content: content.to_string(),
            unit_type: ty.to_string(),
            source: "test".to_string(),
            confidence: 0.5,
            verified: false,
            tags: vec![],
            conditions: serde_json::Value::Null,
            created: "2026-01-01".to_string(),
            updated: "2026-01-01".to_string(),
            archived: false,
        }
    }

    #[test]
    fn skeleton_procedure_has_trigger() {
        assert!(skeleton_for("procedure").contains("trigger:"));
        assert!(skeleton_for("procedure").contains("prereq:"));
    }

    #[test]
    fn skeleton_aspect_has_role() {
        assert!(skeleton_for("aspect").contains("role:"));
    }

    #[test]
    fn skeleton_preference_empty() {
        assert!(skeleton_for("preference").is_empty());
    }

    #[test]
    fn compose_structured_keeps_verbatim() {
        let u = mk_unit(
            "019abc00-0000-7000-8000-000000000000",
            "procedure",
            "---\ntrigger: x\n---\nbody\n",
        );
        let buf = compose_buffer(&u, false);
        // Buffer: header comments + blank line + existing content.
        // Stripped: blank line (from header trailing '\n') + existing content.
        let stripped = strip_header_comments(&buf);
        assert!(
            stripped.contains(&u.content),
            "structured verbatim: {stripped}"
        );
        // Trimming the leading separator yields the original unit content.
        assert_eq!(stripped.trim_start_matches('\n'), u.content);
    }

    #[test]
    fn compose_prose_prepends_skeleton() {
        let u = mk_unit(
            "019abc00-0000-7000-8000-000000000000",
            "procedure",
            "plain body\n",
        );
        let buf = compose_buffer(&u, false);
        let stripped = strip_header_comments(&buf);
        assert!(
            stripped.trim_start().starts_with("---\n"),
            "has skeleton fence: {stripped}"
        );
        assert!(
            stripped.contains("plain body"),
            "body preserved: {stripped}"
        );
    }

    #[test]
    fn compose_template_only_skeleton_no_body() {
        let u = mk_unit(
            "019abc00-0000-7000-8000-000000000000",
            "procedure",
            "plain body\n",
        );
        let buf = compose_buffer(&u, true);
        let stripped = strip_header_comments(&buf);
        assert!(
            !stripped.contains("plain body"),
            "body excluded: {stripped}"
        );
        assert!(stripped.contains("trigger:"), "has skeleton: {stripped}");
    }

    #[test]
    fn strip_removes_leading_hash_lines_until_blank() {
        let input = "# hello\n# world\n\nreal content\n";
        // Separator blank also consumed so body starts cleanly.
        assert_eq!(strip_header_comments(input), "real content\n");
    }

    #[test]
    fn strip_stops_at_fence() {
        let input = "# comment\n---\ntitle: x\n---\nbody\n";
        let out = strip_header_comments(input);
        assert!(out.starts_with("---\n"), "stops at fence: {out}");
    }

    #[test]
    fn strip_preserves_fence_when_separator_present() {
        // Regression: compose_buffer prefixes `#` header then blank then
        // `---`. The stored content must begin at `---`, not at the blank,
        // otherwise `has_frontmatter` misses the block and `scan
        // --unstructured` re-surfaces the unit.
        let input = "# simaris rewrite -- id: x  type: aspect\n# Save + quit to apply.\n\n---\nrole: \"\"\n---\nbody\n";
        let out = strip_header_comments(input);
        assert!(out.starts_with("---\n"), "fence at byte 0: {out:?}");
    }

    #[test]
    fn strip_preserves_body_hash_headers() {
        let input = "# header comment\n\n# Actual Heading\nbody\n";
        let out = strip_header_comments(input);
        // Separator blank consumed; body `#` heading preserved.
        assert_eq!(out, "# Actual Heading\nbody\n");
    }

    #[test]
    fn strip_all_comments_returns_empty() {
        let input = "# only\n# comments\n";
        assert_eq!(strip_header_comments(input), "");
    }

    #[test]
    fn buffers_equal_ignores_trailing_newline() {
        assert!(buffers_equal("hello\n", "hello\n\n"));
        assert!(buffers_equal("hello\n", "hello"));
    }

    #[test]
    fn buffers_equal_ignores_leading_blank() {
        // Header-strip leaves a leading blank line; it should count as no-op
        // against the stored content without leading blanks.
        assert!(buffers_equal("\nhello\n", "hello\n"));
    }

    #[test]
    fn buffers_equal_detects_content_change() {
        assert!(!buffers_equal("hello\n", "world\n"));
    }

    /// R1 mitigation (task `onqm`): the no-op check relies on
    /// `strip_header_comments(compose_buffer(..))` yielding a stable baseline
    /// that equals the composed buffer minus its `#` header block. If the
    /// header format drifts and stripping fails to remove it, the no-op
    /// check silently breaks. Lock the contract.
    #[test]
    fn compose_then_strip_removes_only_header() {
        let u = mk_unit(
            "019abc00-0000-7000-8000-000000000000",
            "procedure",
            "plain body\n",
        );
        let buf = compose_buffer(&u, false);
        let stripped = strip_header_comments(&buf);
        // The stripped result must not contain any of the `#`-prefixed
        // header lines compose_buffer writes.
        assert!(
            !stripped.contains("simaris rewrite -- id:"),
            "header gone: {stripped}"
        );
        assert!(
            !stripped.contains("Save + quit to apply"),
            "header gone: {stripped}"
        );
        // The body we seeded must survive.
        assert!(
            stripped.contains("plain body"),
            "body preserved: {stripped}"
        );
        // A no-edit cancel compares this baseline to itself → equal.
        assert!(buffers_equal(&stripped, &stripped));
    }
}
