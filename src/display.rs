use crate::db::{InboxItem, Link, Unit};

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

pub fn print_added(id: i64, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!("Added unit {id}");
    }
}

pub fn print_linked(from_id: i64, to_id: i64, relationship: &str, json: bool) {
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

pub fn print_dropped(id: i64, json: bool) {
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
                unit.id, unit.unit_type, unit.source, content
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
                item.id, item.created, item.source, content
            );
        }
    }
}
