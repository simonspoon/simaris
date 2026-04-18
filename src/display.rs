use crate::ask::PrimeResult;
use crate::db::{InboxItem, Link, ScanResult, SlugRow, Unit};
use std::path::Path;

/// Short UUID for human-readable display (first 8 chars).
fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
}

pub fn print_unit(unit: &Unit, outgoing: &[Link], incoming: &[Link], json: bool) {
    if json {
        let value = serde_json::json!({
            "unit": unit,
            "links": {
                "outgoing": outgoing,
                "incoming": incoming,
            }
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("[{}] {} ({})", unit.id, unit.unit_type, unit.source);
        println!("{}", unit.content);
        println!(
            "confidence: {}  verified: {}",
            unit.confidence, unit.verified
        );
        if !unit.tags.is_empty() {
            println!("tags: {}", unit.tags.join(", "));
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
