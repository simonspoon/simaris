mod ask;
mod db;
mod digest;
mod display;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "simaris", about = "Knowledge unit storage")]
struct Cli {
    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Show debug trace of internal processing
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a knowledge unit
    Add {
        /// Content of the unit
        content: String,

        /// Type of knowledge unit
        #[arg(long, rename_all = "snake_case")]
        r#type: UnitType,

        /// Source of the unit
        #[arg(long, default_value = "inbox")]
        source: String,
    },

    /// Show a knowledge unit
    Show {
        /// Unit ID
        id: i64,
    },

    /// Link two knowledge units
    Link {
        /// Source unit ID
        from_id: i64,

        /// Target unit ID
        to_id: i64,

        /// Relationship type
        #[arg(long)]
        rel: Relationship,
    },

    /// Drop raw knowledge into the inbox
    Drop {
        /// Content to capture
        content: String,

        /// Source of the capture
        #[arg(long, default_value = "cli")]
        source: String,
    },

    /// Promote an inbox item to a typed knowledge unit
    Promote {
        /// Inbox item ID
        id: i64,

        /// Type for the new unit
        #[arg(long, rename_all = "snake_case")]
        r#type: UnitType,
    },

    /// List pending inbox items
    Inbox,

    /// List knowledge units
    List {
        /// Filter by type
        #[arg(long, rename_all = "snake_case")]
        r#type: Option<UnitType>,
    },

    /// Search knowledge units
    Search {
        /// Search query
        query: String,
    },

    /// Create a backup of the knowledge store
    Backup,

    /// Restore from a backup (no args = list backups)
    Restore {
        /// Backup filename to restore
        filename: Option<String>,
    },

    /// Digest inbox items through LLM classification
    Digest,

    /// Record feedback on a knowledge unit
    Mark {
        /// Unit ID to mark
        id: i64,
        /// Kind of feedback
        #[arg(long)]
        kind: MarkKind,
    },

    /// Ask the knowledge store a question
    Ask {
        /// Your question or context
        query: String,

        /// Run LLM synthesis on results (default: return matched units only)
        #[arg(long)]
        synthesize: bool,
    },
}

#[derive(Clone, ValueEnum)]
enum UnitType {
    Fact,
    Procedure,
    Principle,
    Preference,
    Lesson,
    Idea,
}

impl UnitType {
    fn as_str(&self) -> &'static str {
        match self {
            UnitType::Fact => "fact",
            UnitType::Procedure => "procedure",
            UnitType::Principle => "principle",
            UnitType::Preference => "preference",
            UnitType::Lesson => "lesson",
            UnitType::Idea => "idea",
        }
    }
}

#[derive(Clone, ValueEnum)]
enum Relationship {
    RelatedTo,
    PartOf,
    DependsOn,
    Contradicts,
    Supersedes,
    SourcedFrom,
}

impl Relationship {
    fn as_str(&self) -> &'static str {
        match self {
            Relationship::RelatedTo => "related_to",
            Relationship::PartOf => "part_of",
            Relationship::DependsOn => "depends_on",
            Relationship::Contradicts => "contradicts",
            Relationship::Supersedes => "supersedes",
            Relationship::SourcedFrom => "sourced_from",
        }
    }
}

#[derive(Clone, ValueEnum)]
enum MarkKind {
    Used,
    Wrong,
    Outdated,
    Helpful,
}

impl MarkKind {
    fn as_str(&self) -> &str {
        match self {
            MarkKind::Used => "used",
            MarkKind::Wrong => "wrong",
            MarkKind::Outdated => "outdated",
            MarkKind::Helpful => "helpful",
        }
    }

    fn delta(&self) -> f64 {
        match self {
            MarkKind::Used => 0.05,
            MarkKind::Wrong => -0.2,
            MarkKind::Outdated => -0.1,
            MarkKind::Helpful => 0.1,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that don't need a connection
    if let Command::Restore { filename } = &cli.command {
        match filename {
            Some(f) => {
                db::restore_backup(f)?;
                display::print_restored(f, cli.json);
            }
            None => {
                let backups = db::list_backups()?;
                display::print_backups(&backups, cli.json);
            }
        }
        return Ok(());
    }

    // All other commands need a connection
    let conn = db::connect()?;

    match cli.command {
        Command::Add {
            content,
            r#type,
            source,
        } => {
            let id = db::add_unit(&conn, &content, r#type.as_str(), &source)?;
            display::print_added(id, cli.json);
        }
        Command::Show { id } => {
            let unit = db::get_unit(&conn, id)?;
            let outgoing = db::get_links_from(&conn, id)?;
            let incoming = db::get_links_to(&conn, id)?;
            display::print_unit(&unit, &outgoing, &incoming, cli.json);
        }
        Command::Link {
            from_id,
            to_id,
            rel,
        } => {
            db::add_link(&conn, from_id, to_id, rel.as_str())?;
            display::print_linked(from_id, to_id, rel.as_str(), cli.json);
        }
        Command::Drop { content, source } => {
            let id = db::drop_item(&conn, &content, &source)?;
            display::print_dropped(id, cli.json);
        }
        Command::Promote { id, r#type } => {
            let unit_id = db::promote_item(&conn, id, r#type.as_str())?;
            display::print_added(unit_id, cli.json);
        }
        Command::Inbox => {
            let items = db::list_inbox(&conn)?;
            display::print_inbox(&items, cli.json);
        }
        Command::List { r#type } => {
            let filter = r#type.as_ref().map(|t| t.as_str());
            let units = db::list_units(&conn, filter)?;
            display::print_units(&units, cli.json);
        }
        Command::Search { query } => {
            let units = db::search_units(&conn, &query)?;
            display::print_units(&units, cli.json);
        }
        Command::Backup => {
            let path = db::create_backup(&conn)?;
            display::print_backup_created(&path, cli.json);
        }
        Command::Mark { id, kind } => {
            let confidence = db::add_mark(&conn, id, kind.as_str(), kind.delta())?;
            display::print_marked(id, kind.as_str(), confidence, cli.json);
        }
        Command::Digest => {
            let items = db::list_inbox(&conn)?;
            if items.is_empty() {
                println!("Inbox is empty. Nothing to digest.");
                return Ok(());
            }
            digest::check_claude()?;
            println!("Processing {} inbox item(s)...\n", items.len());
            let mut success = 0;
            let mut failed = 0;
            let mut total_units = 0;
            for item in &items {
                println!(
                    "[{}] {}...",
                    item.id,
                    &item.content.chars().take(50).collect::<String>()
                );
                match digest::classify(&item.content) {
                    Ok(result) => {
                        match db::digest_inbox_item_multi(
                            &conn,
                            item.id,
                            &result.units,
                            &item.source,
                        ) {
                            Ok(ids) => {
                                for (id, unit) in ids.iter().zip(result.units.iter()) {
                                    let marker = if unit.is_overview { "*" } else { " " };
                                    println!(
                                        "  {marker} -> unit {} ({}) [{}]",
                                        id,
                                        unit.unit_type,
                                        unit.tags.join(", ")
                                    );
                                }
                                total_units += ids.len();
                                success += 1;
                            }
                            Err(e) => {
                                println!("  DB ERROR: {e}");
                                failed += 1;
                            }
                        }
                    }
                    Err(e) => {
                        println!("  SKIP: {e}");
                        failed += 1;
                    }
                }
            }
            println!(
                "\nDigested: {} items -> {} units, Skipped: {}",
                success, total_units, failed
            );
        }
        Command::Ask { query, synthesize } => {
            digest::check_claude()?;
            let result = ask::ask(&conn, &query, synthesize, cli.debug)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if let Some(ref response) = result.response {
                println!("{}", response);
            } else if result.units.is_empty() {
                println!("No knowledge found for that query.");
            } else {
                println!("Found {} relevant unit(s):\n", result.units.len());
                for unit in &result.units {
                    let tags_str = if unit.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" (tags: {})", unit.tags.join(", "))
                    };
                    println!("[{}] {}{}", unit.id, unit.unit_type, tags_str);
                    for line in unit.content.lines() {
                        println!("  {}", line);
                    }
                    if !unit.links.is_empty() {
                        let link_strs: Vec<String> = unit
                            .links
                            .iter()
                            .map(|l| format!("{} {} ({})", l.unit_id, l.title, l.relationship))
                            .collect();
                        println!("  Links: {}", link_strs.join(", "));
                    }
                    println!();
                }
            }
        }
        Command::Restore { .. } => unreachable!(),
    }

    Ok(())
}
