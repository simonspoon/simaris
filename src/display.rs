use crate::ask::PrimeResult;
use crate::db::{InboxItem, LinkedUnit, ScanResult, SlugRow, Stats, Unit, UnstructuredRow};
use crate::lint::LintReport;
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
    /// Body size in bytes — surfaces split-ruleset thresholds in tooling.
    byte_size: usize,
    /// Surfaced only when an archived unit is present in the result set
    /// (i.e. caller passed `--include-archived`). Skipped from JSON when
    /// false to keep payloads compact for the common live-only case.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    archived: bool,
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
                byte_size: u.content.len(),
                archived: u.archived,
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
            // Archived rows lead with a `[archived]` marker so the soft-
            // deleted state is obvious in `--include-archived` listings.
            let archived_marker = if unit.archived { "[archived] " } else { "" };
            println!(
                "{}[{}] {} ({}) {}  {}  conf={:.2}{}",
                archived_marker,
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

/// Display options for [`print_unit`]. Bundled into a struct to keep the
/// function signature small as flags accrete.
#[derive(Default, Clone, Copy)]
pub struct ShowOpts {
    /// Output as JSON.
    pub json: bool,
    /// Print content verbatim — skip frontmatter parsing/rendering.
    pub raw: bool,
    /// Print only `unit.content`, omit metadata and links.
    pub content_only: bool,
    /// Strip YAML frontmatter from the printed content.
    pub no_frontmatter: bool,
}

pub fn print_unit(
    unit: &Unit,
    outgoing: &[LinkedUnit],
    incoming: &[LinkedUnit],
    slugs: &[String],
    opts: ShowOpts,
) {
    // Resolve content body up-front so `--content` and `--no-frontmatter`
    // can compose with each other and with `--raw`.
    let content_str: String = if opts.no_frontmatter {
        let parsed = frontmatter::parse(&unit.content);
        parsed.body.to_string()
    } else {
        unit.content.clone()
    };

    if opts.content_only {
        // `--content` always prints just the bare content string, regardless
        // of `--json`. Nothing else (no metadata, no links, no wrapper).
        println!("{content_str}");
        return;
    }

    if opts.json {
        let mut unit_value = serde_json::to_value(unit).unwrap();
        if opts.no_frontmatter
            && let Some(obj) = unit_value.as_object_mut()
        {
            obj.insert(
                "content".to_string(),
                serde_json::Value::String(content_str.clone()),
            );
        }
        let value = serde_json::json!({
            "unit": unit_value,
            "links": {
                "outgoing": outgoing,
                "incoming": incoming,
            },
            "slugs": slugs,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("[{}] {} ({})", unit.id, unit.unit_type, unit.source);
        if opts.raw {
            println!("{content_str}");
        } else if opts.no_frontmatter {
            // Frontmatter already stripped — print body verbatim.
            println!("{content_str}");
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
            "confidence: {}  verified: {}{}",
            unit.confidence,
            unit.verified,
            if unit.archived { "  [archived]" } else { "" }
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

        print_relationships_table(outgoing, incoming);
    }
}

/// Render the relationships block of `simaris show` as a fixed-width table.
///
/// Columns: direction (`->`/`<-`), relationship name, short id of the
/// linked unit, and an identifying text — slug if present, else the
/// derived headline. Outgoing first then incoming, each block already
/// ordered (relationship, id) by the SQL queries.
///
/// No-op when both slices are empty.
fn print_relationships_table(outgoing: &[LinkedUnit], incoming: &[LinkedUnit]) {
    if outgoing.is_empty() && incoming.is_empty() {
        return;
    }

    // Build rows up-front so column widths can be measured once.
    struct Row<'a> {
        dir: &'static str,
        rel: &'a str,
        id: String,
        text: String,
    }

    let rows: Vec<Row> = outgoing
        .iter()
        .map(|l| Row {
            dir: "->",
            rel: &l.relationship,
            id: short_id(&l.to_id).to_string(),
            text: identifying_text(l),
        })
        .chain(incoming.iter().map(|l| Row {
            dir: "<-",
            rel: &l.relationship,
            id: short_id(&l.from_id).to_string(),
            text: identifying_text(l),
        }))
        .collect();

    // Column widths: header label is the floor, longest cell sets the ceiling.
    let rel_w = rows.iter().map(|r| r.rel.len()).max().unwrap_or(0).max(3);
    let id_w = rows.iter().map(|r| r.id.len()).max().unwrap_or(0).max(2);

    let blank = "";
    let rel_h = "rel";
    let id_h = "id";
    let text_h = "text";
    println!();
    println!("relationships:");
    println!("  {blank:<2}  {rel_h:<rel_w$}  {id_h:<id_w$}  {text_h}");
    for r in &rows {
        let dir = r.dir;
        let rel = r.rel;
        let id = &r.id;
        let text = &r.text;
        println!("  {dir:<2}  {rel:<rel_w$}  {id:<id_w$}  {text}");
    }
}

/// Identifying text for a relationship row: slug when present, else the
/// linked unit's derived headline. Empty string is preserved (no fallback
/// invention) so callers can see when a unit is unidentifiable.
fn identifying_text(l: &LinkedUnit) -> String {
    if let Some(slug) = &l.slug {
        slug.clone()
    } else {
        l.headline.clone()
    }
}

pub fn print_added(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!("Added unit {id}");
    }
}

/// S1 bridge — refusal output when `--refuse-dup` finds a recent twin.
/// Caller exits non-zero (currently 2). Stderr in text mode so callers can
/// still capture stdout; stdout in JSON mode for machine consumers.
pub fn print_refused_dup(existing_id: &str, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "refused": "duplicate",
                "existing_id": existing_id,
                "window_days": 7,
            })
        );
    } else {
        eprintln!(
            "refused: byte-identical content already added within 7 days as {existing_id}"
        );
    }
}

pub fn print_cloned(from_id: &str, new_id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": new_id, "from": from_id }));
    } else {
        println!("Cloned {from_id} -> {new_id}");
    }
}

pub fn print_deleted(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "deleted": id }));
    } else {
        println!("Deleted unit {id}");
    }
}

pub fn print_archived(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "archived": id }));
    } else {
        println!("Archived unit {id}");
    }
}

pub fn print_unarchived(id: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "unarchived": id }));
    } else {
        println!("Unarchived unit {id}");
    }
}

pub fn print_stats(stats: &Stats, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(stats).unwrap());
        return;
    }
    let scope = if stats.include_archived {
        "all units (including archived)"
    } else {
        "live units (archived excluded)"
    };
    println!("simaris stats — {scope}");
    println!("  total:            {}", stats.total);
    println!("  archived:         {}", stats.archived_count);
    println!("  inbox:            {}", stats.inbox_size);
    println!("  superseded:       {}", stats.superseded_count);
    println!("\nby type:");
    if stats.by_type.is_empty() {
        println!("  (none)");
    } else {
        for (t, n) in &stats.by_type {
            println!("  {t:<12} {n}");
        }
    }
    println!("\nconfidence:");
    println!("  low      (<0.60): {}", stats.confidence.low);
    println!("  medium   (<0.80): {}", stats.confidence.medium);
    println!("  high     (<0.95): {}", stats.confidence.high);
    println!("  verified (≥0.95): {}", stats.confidence.verified);
    println!("\nmarks:");
    if stats.marks.is_empty() {
        println!("  (none)");
    } else {
        for (kind, n) in &stats.marks {
            println!("  {kind:<10} {n}");
        }
    }
    println!(
        "\ntags ({} unique, top {}):",
        stats.by_tag.total_unique,
        stats.by_tag.top.len()
    );
    if stats.by_tag.top.is_empty() {
        println!("  (none)");
    } else {
        for tc in &stats.by_tag.top {
            println!("  {:<24} {}", tc.tag, tc.count);
        }
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

    let mut any_directory = false;

    for (i, section) in result.sections.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("# {}", section.label);
        for unit in &section.units {
            println!();
            if unit.full {
                println!("{}", unit.content);
            } else {
                any_directory = true;
                let preview = first_line_preview(&unit.content);
                let tag_part = if unit.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", unit.tags.join(", "))
                };
                println!("- [{}] {}{}", unit.id, preview, tag_part);
            }
        }
    }

    if any_directory {
        println!();
        println!("# Loading");
        println!();
        println!("Directory entries above are stubs. Load any full body with: `simaris show <id>`");
    }
}

/// First non-empty line of a unit's content body (frontmatter skipped),
/// stripped of leading markdown header marks and trimmed to a max width.
/// Used for LOD-1 directory previews.
fn first_line_preview(content: &str) -> String {
    let body = frontmatter::parse(content).body;
    let raw = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim();

    const MAX: usize = 80;
    if raw.chars().count() > MAX {
        let end = raw
            .char_indices()
            .nth(MAX)
            .map(|(i, _)| i)
            .unwrap_or(raw.len());
        format!("{}…", &raw[..end])
    } else {
        raw.to_string()
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

    #[test]
    fn first_line_preview_skips_frontmatter() {
        // LOD-1 directory entries strip the YAML frontmatter so callers
        // see the body's first non-empty line, not `---` and not the first
        // frontmatter key.
        let with_fm = "---\nrole: \"r\"\n---\n\nfirst body line\n\nsecond para\n";
        assert_eq!(first_line_preview(with_fm), "first body line");

        // Markdown header marks are stripped from the preview.
        let heading_first = "---\ntags: [x]\n---\n\n# Body Heading\n\npara\n";
        assert_eq!(first_line_preview(heading_first), "Body Heading");

        // No frontmatter — first non-empty line still wins, leading
        // whitespace trimmed.
        let plain = "\n\n  plain start\nrest\n";
        assert_eq!(first_line_preview(plain), "plain start");
    }
}


/// Render a `simaris lint` report.
///
/// JSON mode emits the full structured report (no truncation, all findings
/// per category). Text mode prints grouped sections with counts and the top
/// 5 examples per category. Always advisory — caller exits 0.
pub fn print_lint(report: &LintReport, json: bool, fix_suggest: bool, by_aspect: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(report).unwrap());
        return;
    }

    let mut printed_any = false;

    if !report.procedure_no_trigger.is_empty() {
        printed_any = true;
        println!(
            "PROCEDURE_NO_TRIGGER: {} unit(s)",
            report.procedure_no_trigger.len()
        );
        for f in report.procedure_no_trigger.iter().take(5) {
            println!(
                "  [{}] ({}) {}",
                short_id(&f.id),
                f.unit_type,
                truncate_content(&f.headline)
            );
            println!("      → {}", f.reason);
        }
        if report.procedure_no_trigger.len() > 5 {
            println!(
                "  … and {} more",
                report.procedure_no_trigger.len() - 5
            );
        }
        println!();
    }

    if !report.orphan.is_empty() {
        printed_any = true;
        println!("ORPHAN: {} unit(s)", report.orphan.len());
        for f in report.orphan.iter().take(5) {
            println!(
                "  [{}] ({}) {}",
                short_id(&f.id),
                f.unit_type,
                truncate_content(&f.headline)
            );
            println!("      → {}", f.reason);
        }
        if report.orphan.len() > 5 {
            println!("  … and {} more", report.orphan.len() - 5);
        }
        println!();
    }

    if !report.dupe.is_empty() {
        printed_any = true;
        println!("DUPE: {} pair(s)", report.dupe.len());
        for f in report.dupe.iter().take(5) {
            println!(
                "  [{}] <-> [{}] (sim={:.2}, type={})",
                short_id(&f.a_id),
                short_id(&f.b_id),
                f.similarity,
                f.unit_type
            );
            println!("      → {}", f.reason);
            println!("        a: {}", truncate_content(&f.a_headline));
            println!("        b: {}", truncate_content(&f.b_headline));
        }
        if report.dupe.len() > 5 {
            println!("  … and {} more", report.dupe.len() - 5);
        }
        println!();
    }

    if !report.dual_parent_divergence.is_empty() {
        printed_any = true;
        println!(
            "DUAL_PARENT_DIVERGENCE: {} unit(s)",
            report.dual_parent_divergence.len()
        );
        for f in report.dual_parent_divergence.iter().take(5) {
            println!(
                "  [{}] ({}) {}",
                short_id(&f.id),
                f.unit_type,
                truncate_content(&f.headline)
            );
            println!("      → {}", f.reason);
            println!(
                "        parents: [{}] @ {}  ↔  [{}] @ {}",
                short_id(&f.parent_a_id),
                f.parent_a_updated,
                short_id(&f.parent_b_id),
                f.parent_b_updated
            );
        }
        if report.dual_parent_divergence.len() > 5 {
            println!(
                "  … and {} more",
                report.dual_parent_divergence.len() - 5
            );
        }
        println!();
    }

    // M1: TAG_VARIANT block (printed alongside content categories).
    if !report.tag_variant.is_empty() {
        printed_any = true;
        println!("TAG_VARIANT: {} group(s)", report.tag_variant.len());
        for f in report.tag_variant.iter().take(5) {
            println!(
                "  `{}` ({} variant(s), {} total uses)",
                f.canonical,
                f.variants.len(),
                f.total_uses
            );
            println!("      → {}", f.reason);
        }
        if report.tag_variant.len() > 5 {
            println!("  … and {} more", report.tag_variant.len() - 5);
        }
        println!();
    }

    if !printed_any {
        println!("No lint issues found.");
    } else {
        println!(
            "Total findings: {} (procedure_no_trigger={}, orphan={}, dupe={}, dual_parent_divergence={}, tag_variant={})",
            report.total(),
            report.procedure_no_trigger.len(),
            report.orphan.len(),
            report.dupe.len(),
            report.dual_parent_divergence.len(),
            report.tag_variant.len()
        );
    }

    // M1: bulk tag entropy stats (always printed — fast diagnostic).
    let ts = &report.tag_stats;
    if ts.distinct > 0 {
        println!(
            "tag_stats: distinct={}, total_uses={}, singletons={} ({:.1}%), low_use(<3)={} ({:.1}%)",
            ts.distinct,
            ts.total_uses,
            ts.singletons,
            (ts.singletons as f64 / ts.distinct as f64) * 100.0,
            ts.low_use,
            (ts.low_use as f64 / ts.distinct as f64) * 100.0
        );
        println!();
    }

    // M1: per-aspect rollup table (gated on --by-aspect to keep default
    // output skim-friendly). Top 10.
    if by_aspect && !report.by_aspect.is_empty() {
        println!("by_aspect (top 10 by total findings):");
        println!(
            "  {:<10}  {:<32}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}",
            "id", "owner", "PNT", "ORPH", "DUPE", "DPD", "TOTAL"
        );
        for r in report.by_aspect.iter().take(10) {
            let owner = match &r.slug {
                Some(s) => format!("{} ({})", s, truncate_content(&r.headline)),
                None => truncate_content(&r.headline),
            };
            let id_short = if r.aspect_id == "(unowned)" {
                "(unowned)".to_string()
            } else {
                short_id(&r.aspect_id).to_string()
            };
            println!(
                "  {:<10}  {:<32}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}",
                id_short,
                truncate_owner(&owner, 32),
                r.procedure_no_trigger,
                r.orphan,
                r.dupe,
                r.dual_parent_divergence,
                r.total
            );
        }
        if report.by_aspect.len() > 10 {
            println!("  … and {} more aspect(s)", report.by_aspect.len() - 10);
        }
        println!();
    }

    if fix_suggest && report.total() > 0 {
        println!();
        println!("Fix suggestions:");
        if !report.procedure_no_trigger.is_empty() {
            println!(
                "  PROCEDURE_NO_TRIGGER → simaris edit <id> --trigger '<machine-detectable event>'"
            );
        }
        if !report.orphan.is_empty() {
            println!(
                "  ORPHAN → simaris link <id> <parent-aspect-id> --rel part_of   (or)   simaris slug set <name> <id>"
            );
        }
        if !report.dupe.is_empty() {
            println!(
                "  DUPE → review pairs; simaris archive <loser-id>, or simaris link winner loser --rel supersedes"
            );
        }
        if !report.dual_parent_divergence.is_empty() {
            println!(
                "  DUAL_PARENT_DIVERGENCE → review parents; drop the stale link or refresh the lagging branch"
            );
        }
        if !report.tag_variant.is_empty() {
            println!(
                "  TAG_VARIANT → pick a canonical, then `simaris edit <id> --tags ...` on each unit; archive obsolete variant"
            );
        }
    }
}

/// Helper: truncate the owner column without splitting a UTF-8 boundary.
fn truncate_owner(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let take: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{take}…")
}

/// Render `simaris lint --history` output.
pub fn print_lint_history(rows: &[crate::db::LintSnapshotRow], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(rows).unwrap());
        return;
    }
    if rows.is_empty() {
        println!("No lint snapshots recorded yet. Run `simaris lint --snapshot` first.");
        return;
    }
    println!(
        "{:<10}  {:<19}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}  {:>6}  note",
        "id", "created", "PNT", "ORPH", "DUPE", "DPD", "TAG", "TOTAL"
    );
    for r in rows {
        let t = &r.totals;
        let id_short = short_id(&r.id);
        println!(
            "{:<10}  {:<19}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}  {:>6}  {}",
            id_short,
            r.created,
            t.procedure_no_trigger,
            t.orphan,
            t.dupe,
            t.dual_parent_divergence,
            t.tag_variant,
            t.total,
            r.note
        );
    }
}

/// Render `simaris lint --ci` output. Compares current totals to `prev`
/// and emits a structured delta. Returns `true` if any category regressed
/// (count strictly increased) — caller exits non-zero.
pub fn print_lint_ci(
    report: &LintReport,
    prev: Option<&crate::db::LintSnapshotRow>,
    json: bool,
) -> bool {
    let curr = report.totals();
    let prev_totals = prev.map(|p| p.totals.clone()).unwrap_or(crate::db::LintTotals {
        procedure_no_trigger: 0,
        orphan: 0,
        dupe: 0,
        dual_parent_divergence: 0,
        tag_variant: 0,
        total: 0,
    });

    // Per-category delta (signed).
    let d = |a: usize, b: usize| -> i64 { a as i64 - b as i64 };
    let pnt_d = d(curr.procedure_no_trigger, prev_totals.procedure_no_trigger);
    let orph_d = d(curr.orphan, prev_totals.orphan);
    let dupe_d = d(curr.dupe, prev_totals.dupe);
    let dpd_d = d(curr.dual_parent_divergence, prev_totals.dual_parent_divergence);
    let tag_d = d(curr.tag_variant, prev_totals.tag_variant);
    let total_d = d(curr.total, prev_totals.total);

    let mut regressions: Vec<&'static str> = Vec::new();
    if pnt_d > 0 {
        regressions.push("procedure_no_trigger");
    }
    if orph_d > 0 {
        regressions.push("orphan");
    }
    if dupe_d > 0 {
        regressions.push("dupe");
    }
    if dpd_d > 0 {
        regressions.push("dual_parent_divergence");
    }
    if tag_d > 0 {
        regressions.push("tag_variant");
    }
    let regressed = !regressions.is_empty();

    if json {
        let payload = serde_json::json!({
            "previous": prev.map(|p| serde_json::json!({
                "id": p.id,
                "created": p.created,
                "totals": p.totals,
                "note": p.note,
            })),
            "current": curr,
            "delta": {
                "procedure_no_trigger": pnt_d,
                "orphan": orph_d,
                "dupe": dupe_d,
                "dual_parent_divergence": dpd_d,
                "tag_variant": tag_d,
                "total": total_d,
            },
            "regressions": regressions,
            "regressed": regressed,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else {
        if let Some(p) = prev {
            println!("CI compare vs snapshot {} ({})", short_id(&p.id), p.created);
        } else {
            println!("CI compare vs no prior snapshot (treating baseline as zero)");
        }
        println!(
            "  procedure_no_trigger: {} -> {} ({:+})",
            prev_totals.procedure_no_trigger, curr.procedure_no_trigger, pnt_d
        );
        println!(
            "  orphan:               {} -> {} ({:+})",
            prev_totals.orphan, curr.orphan, orph_d
        );
        println!(
            "  dupe:                 {} -> {} ({:+})",
            prev_totals.dupe, curr.dupe, dupe_d
        );
        println!(
            "  dual_parent_divergence: {} -> {} ({:+})",
            prev_totals.dual_parent_divergence, curr.dual_parent_divergence, dpd_d
        );
        println!(
            "  tag_variant:          {} -> {} ({:+})",
            prev_totals.tag_variant, curr.tag_variant, tag_d
        );
        println!(
            "  total:                {} -> {} ({:+})",
            prev_totals.total, curr.total, total_d
        );
        if regressed {
            println!();
            println!("REGRESSED: {}", regressions.join(", "));
        }
    }

    regressed
}
