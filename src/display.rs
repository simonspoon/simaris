use crate::db::{Link, Unit};

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
