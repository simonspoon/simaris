use crate::ask::PrimeResult;
use crate::db::{InboxItem, Link, ScanResult, SlugRow, Unit, UnstructuredRow};
use crate::emit::EmitResult;
use crate::frontmatter;
use serde::Serialize;
use std::path::Path;

/// Short UUID for human-readable display (first 8 chars).
fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
}

/// Headline for lean list/search rows: first non-empty line of `content`,
/// trimmed and truncated at 120 chars (ellipsis appended if cut). Char-aware
/// to avoid splitting a UTF-8 codepoint.
///
/// For structured units (frontmatter present), the fence `---` is not a
/// useful headline — the hook would show just `---`. Parse the frontmatter
/// off and use the body's first non-empty line. If the body is empty, fall
/// back to a representative frontmatter scalar (`role` / `scope` / `trigger`
/// / `context` / `tension`) so the hook still surfaces something meaningful.
pub fn derive_headline(content: &str) -> String {
    let parsed = frontmatter::parse(content);
    let source = if parsed.frontmatter.is_some() {
        parsed.body
    } else {
        content
    };

    let first = source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");

    // Fall back to a frontmatter scalar if the body is empty.
    let pick = if first.is_empty() {
        parsed
            .frontmatter
            .as_ref()
            .and_then(|fm| fm.as_mapping())
            .and_then(|m| {
                for key in ["role", "scope", "trigger", "context", "tension"] {
                    if let Some(v) = m.get(serde_yml::Value::String(key.to_string()))
                        && let Some(s) = v.as_str()
                        && !s.is_empty()
                    {
                        return Some(s.to_string());
                    }
                }
                None
            })
            .unwrap_or_default()
    } else {
        first.to_string()
    };

    const MAX: usize = 120;
    if pick.chars().count() > MAX {
        let end = pick
            .char_indices()
            .nth(MAX)
            .map(|(i, _)| i)
            .unwrap_or(pick.len());
        format!("{}...", &pick[..end])
    } else {
        pick
    }
}

/// Lean row shape for default `search` / `list` output. Omits body to keep
/// agent call sizes under the bash output cap. Full body still available via
/// `simaris show <id>` or by passing `--full` to search/list.
#[derive(Serialize)]
struct LeanUnit<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    unit_type: &'a str,
    slug: Option<&'a str>,
    headline: String,
    tags: &'a [String],
    source: &'a str,
    confidence: f64,
}

pub fn print_units_lean(units: &[Unit], slug_map: &[Option<String>], json: bool) {
    if json {
        let rows: Vec<LeanUnit> = units
            .iter()
            .zip(slug_map.iter())
            .map(|(u, s)| LeanUnit {
                id: &u.id,
                unit_type: &u.unit_type,
                slug: s.as_deref(),
                headline: derive_headline(&u.content),
                tags: &u.tags,
                source: &u.source,
                confidence: u.confidence,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
    } else if units.is_empty() {
        println!("No units found.");
    } else {
        for (unit, slug) in units.iter().zip(slug_map.iter()) {
            let slug_disp = slug.as_deref().unwrap_or("-");
            let tags_str = if unit.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", unit.tags.join(", "))
            };
            println!(
                "[{}] {} ({}) {}  {}  conf={:.2}{}",
                short_id(&unit.id),
                unit.unit_type,
                unit.source,
                slug_disp,
                derive_headline(&unit.content),
                unit.confidence,
                tags_str,
            );
        }
    }
}

pub fn print_unit(
    unit: &Unit,
    outgoing: &[Link],
    incoming: &[Link],
    slugs: &[String],
    json: bool,
    raw: bool,
) {
    if json {
        let value = serde_json::json!({
            "unit": unit,
            "links": {
                "outgoing": outgoing,
                "incoming": incoming,
            },
            "slugs": slugs,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("[{}] {} ({})", unit.id, unit.unit_type, unit.source);
        if raw {
            println!("{}", unit.content);
        } else {
            let parsed = frontmatter::parse(&unit.content);
            if let Some(ref fm) = parsed.frontmatter {
                let md = frontmatter::render_markdown(fm);
                if !md.is_empty() {
                    print!("{md}");
                    println!();
                }
                println!("{}", parsed.body);
            } else {
                println!("{}", unit.content);
            }
        }
        println!(
            "confidence: {}  verified: {}",
            unit.confidence, unit.verified
        );
        if !unit.tags.is_empty() {
            println!("tags: {}", unit.tags.join(", "));
        }
        if !slugs.is_empty() {
            println!("Slugs: {}", slugs.join(", "));
        }
        if unit.conditions != serde_json::json!({}) {
            println!("conditions: {}", unit.conditions);
        }
        println!("created: {}  updated: {}", unit.created, unit.updated);

        if !outgoing.is_empty() || !incoming.is_empty() {
            println!();
        }
        for link in outgoing {
            println!("  -> {} ({})", link.to_id, link.relationship);
        }
        for link in incoming {
            println!("  <- {} ({})", link.from_id, link.relationship);
        }
    }
}

pub fn print_added(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!("Added unit {id}");
    }
}

pub fn print_deleted(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "deleted": id }));
    } else {
        println!("Deleted unit {id}");
    }
}

pub fn print_linked(from_id: &str, to_id: &str, relationship: &str, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "from": from_id,
                "to": to_id,
                "relationship": relationship,
            })
        );
    } else {
        println!("Linked {from_id} -> {to_id} ({relationship})");
    }
}

pub fn print_dropped(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!("Dropped item {id}");
    }
}

pub fn print_units(units: &[Unit], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(units).unwrap());
    } else if units.is_empty() {
        println!("No units found.");
    } else {
        for unit in units {
            let content = if unit.content.chars().count() > 80 {
                let end = unit
                    .content
                    .char_indices()
                    .nth(80)
                    .map(|(i, _)| i)
                    .unwrap_or(unit.content.len());
                format!("{}...", &unit.content[..end])
            } else {
                unit.content.clone()
            };
            println!(
                "[{}] {} ({})  {}",
                short_id(&unit.id),
                unit.unit_type,
                unit.source,
                content
            );
        }
    }
}

pub fn print_inbox(items: &[InboxItem], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(items).unwrap());
    } else if items.is_empty() {
        println!("Inbox is empty.");
    } else {
        for item in items {
            let content = if item.content.chars().count() > 80 {
                let end = item
                    .content
                    .char_indices()
                    .nth(80)
                    .map(|(i, _)| i)
                    .unwrap_or(item.content.len());
                format!("{}...", &item.content[..end])
            } else {
                item.content.clone()
            };
            println!(
                "[{}] {} ({})  {}",
                short_id(&item.id),
                item.created,
                item.source,
                content
            );
        }
    }
}

pub fn print_backup_created(path: &Path, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({ "path": path.to_str().unwrap_or("") })
        );
    } else {
        println!("Backup created: {}", path.display());
    }
}

pub fn print_backups(names: &[String], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(names).unwrap());
    } else if names.is_empty() {
        println!("No backups found.");
    } else {
        for name in names {
            println!("{name}");
        }
    }
}

pub fn print_restored(filename: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "restored": filename }));
    } else {
        println!("Restored from: {filename}");
    }
}

pub fn print_marked(id: &str, kind: &str, confidence: f64, json: bool) {
    if json {
        let out = serde_json::json!({
            "id": id,
            "mark": kind,
            "confidence": (confidence * 100.0).round() / 100.0,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!(
            "Marked unit {} as {} (confidence: {:.2})",
            id, kind, confidence
        );
    }
}

pub fn print_prime(result: &PrimeResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(result).unwrap());
        return;
    }

    if result.sections.is_empty() {
        println!("No relevant knowledge found for: {}", result.task);
        return;
    }

    for (i, section) in result.sections.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("# {}", section.label);
        for unit in &section.units {
            println!();
            println!("{}", unit.content);
        }
    }
}

fn truncate_content(content: &str) -> String {
    if content.chars().count() > 80 {
        let end = content
            .char_indices()
            .nth(80)
            .map(|(i, _)| i)
            .unwrap_or(content.len());
        format!("{}...", &content[..end])
    } else {
        content.to_string()
    }
}

pub fn print_scan(result: &ScanResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(result).unwrap());
        return;
    }

    let mut found_issues = false;

    if !result.low_confidence.is_empty() {
        found_issues = true;
        println!("Low confidence:");
        for unit in &result.low_confidence {
            println!(
                "  [{}] ({:.2}) {}",
                short_id(&unit.id),
                unit.confidence,
                truncate_content(&unit.content)
            );
        }
        println!();
    }

    if !result.negative_marks.is_empty() {
        found_issues = true;
        println!("Negative marks:");
        for unit in &result.negative_marks {
            println!(
                "  [{}] {}",
                short_id(&unit.id),
                truncate_content(&unit.content)
            );
        }
        println!();
    }

    if !result.contradictions.is_empty() {
        found_issues = true;
        println!("Contradictions:");
        for pair in &result.contradictions {
            println!(
                "  [{}] {} <-> [{}] {}",
                short_id(&pair.from_id),
                truncate_content(&pair.from_content),
                short_id(&pair.to_id),
                truncate_content(&pair.to_content)
            );
        }
        println!();
    }

    if !result.orphans.is_empty() {
        found_issues = true;
        println!("Orphans:");
        for unit in &result.orphans {
            println!(
                "  [{}] {}",
                short_id(&unit.id),
                truncate_content(&unit.content)
            );
        }
        println!();
    }

    if !result.stale.is_empty() {
        found_issues = true;
        println!("Stale:");
        for unit in &result.stale {
            println!(
                "  [{}] ({}) {}",
                short_id(&unit.id),
                unit.created,
                truncate_content(&unit.content)
            );
        }
        println!();
    }

    if !found_issues {
        println!("No issues found.");
    }
}

/// Render rows from `scan --unstructured`. JSON mode serializes the vector
/// as-is; text mode prints a compact table with short id, type, first slug
/// (or `-`), mark count, confidence, and the leading line of the body.
pub fn print_scan_unstructured(rows: &[UnstructuredRow], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(rows).unwrap());
        return;
    }
    if rows.is_empty() {
        println!("No unstructured units found.");
        return;
    }
    println!(
        "{:<8}  {:<10}  {:<16}  {:>5}  {:>5}  first-line",
        "id", "type", "slug", "marks", "conf"
    );
    for row in rows {
        let slug = row.slugs.first().map(String::as_str).unwrap_or("-");
        let slug_trim = if slug.chars().count() > 16 {
            let end = slug
                .char_indices()
                .nth(16)
                .map(|(i, _)| i)
                .unwrap_or(slug.len());
            format!("{}…", &slug[..end])
        } else {
            slug.to_string()
        };
        let first = truncate_content(&row.first_line);
        println!(
            "{:<8}  {:<10}  {:<16}  {:>5}  {:>5.2}  {}",
            short_id(&row.id),
            row.unit_type,
            slug_trim,
            row.marks,
            row.confidence,
            first
        );
    }
}

pub fn print_slug_set(slug: &str, id: &str, json: bool) {
    if json {
        let value = serde_json::json!({ "slug": slug, "unit_id": id });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Set slug '{slug}' -> {id}");
    }
}

pub fn print_slug_unset(slug: &str, removed: bool, json: bool) {
    if json {
        let value = serde_json::json!({ "unset": slug, "removed": removed });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else if removed {
        println!("Unset slug '{slug}'.");
    } else {
        println!("No slug '{slug}' set.");
    }
}

pub fn print_emit_result(result: &EmitResult, target_dir: &Path, json: bool) {
    if json {
        let value = serde_json::json!({
            "target_dir": target_dir.to_string_lossy(),
            "written": result.written,
            "swept": result.swept,
            "skipped_uuids": result.skipped_uuids,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return;
    }
    println!("Emit target: {}", target_dir.display());
    println!(
        "Written: {}  Swept: {}  Skipped: {}",
        result.written.len(),
        result.swept.len(),
        result.skipped_uuids.len()
    );
    if !result.written.is_empty() {
        println!("Written slugs:");
        for slug in &result.written {
            println!("  {slug}");
        }
    }
    if !result.swept.is_empty() {
        println!("Swept slugs:");
        for slug in &result.swept {
            println!("  {slug}");
        }
    }
    if !result.skipped_uuids.is_empty() {
        println!("Skipped (no slug):");
        for id in &result.skipped_uuids {
            println!("  {id}");
        }
    }
}

pub fn print_slug_list(rows: &[SlugRow], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(rows).unwrap());
    } else if rows.is_empty() {
        println!("No slugs.");
    } else {
        for row in rows {
            println!("{} -> {}", row.slug, row.unit_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headline_prose_uses_first_line() {
        let c = "first line of prose\n\nsecond para\n";
        assert_eq!(derive_headline(c), "first line of prose");
    }

    #[test]
    fn headline_prose_skips_blank_lines() {
        let c = "\n\n  real content here\n";
        assert_eq!(derive_headline(c), "real content here");
    }

    #[test]
    fn headline_structured_skips_fence_picks_body_heading() {
        // Regression: hook surfaced `---` as the headline for units with
        // frontmatter. Body's first line should win.
        let c = "---\nrole: \"r\"\n---\n\n# Body Heading\n\nparagraph\n";
        assert_eq!(derive_headline(c), "# Body Heading");
    }

    #[test]
    fn headline_structured_no_body_falls_back_to_role() {
        let c = "---\nrole: \"skeptical reviewer\"\n---\n";
        assert_eq!(derive_headline(c), "skeptical reviewer");
    }

    #[test]
    fn headline_structured_no_body_falls_back_to_trigger() {
        let c = "---\ntrigger: \"new target found\"\n---\n";
        assert_eq!(derive_headline(c), "new target found");
    }

    #[test]
    fn headline_structured_no_body_no_fallback_key_returns_empty() {
        let c = "---\nfoo: bar\n---\n";
        assert_eq!(derive_headline(c), "");
    }

    #[test]
    fn headline_truncates_at_120_chars() {
        let long: String = "x".repeat(150);
        let out = derive_headline(&long);
        // 120 chars + "..." = 123
        assert_eq!(out.chars().count(), 123);
        assert!(out.ends_with("..."));
    }
}
