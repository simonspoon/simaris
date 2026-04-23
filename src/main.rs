mod ask;
mod db;
mod digest;
mod display;
mod emit;

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

        /// Tags (comma-separated)
        #[arg(long)]
        tags: Option<String>,
    },

    /// Show a knowledge unit
    Show {
        /// Unit ID
        id: String,
    },

    /// Link two knowledge units
    Link {
        /// Source unit ID
        from_id: String,

        /// Target unit ID
        to_id: String,

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
        id: String,

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

        /// Emit full unit bodies (default: lean id/type/slug/headline/tags/source/confidence)
        #[arg(long)]
        full: bool,
    },

    /// Search knowledge units
    Search {
        /// Search query
        query: String,

        /// Filter by type
        #[arg(long, rename_all = "snake_case")]
        r#type: Option<UnitType>,

        /// Emit full unit bodies (default: lean id/type/slug/headline/tags/source/confidence)
        #[arg(long)]
        full: bool,
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
        id: String,
        /// Kind of feedback
        #[arg(long)]
        kind: MarkKind,
    },

    /// Assemble a mindset from the knowledge graph for a task
    Prime {
        /// Task description
        task: String,

        /// Filter strategy for narrowing gathered units
        #[arg(long, default_value = "standard")]
        filter: PrimeFilter,
    },

    /// Ask the knowledge store a question
    Ask {
        /// Your question or context
        query: String,

        /// Run LLM synthesis on results (default: return matched units only)
        #[arg(long)]
        synthesize: bool,

        /// Filter by type
        #[arg(long, rename_all = "snake_case")]
        r#type: Option<UnitType>,
    },

    /// Edit a knowledge unit
    Edit {
        /// Unit ID
        id: String,

        /// New content
        #[arg(long)]
        content: Option<String>,

        /// New type
        #[arg(long, rename_all = "snake_case")]
        r#type: Option<UnitType>,

        /// New source
        #[arg(long)]
        source: Option<String>,

        /// New tags (comma-separated, replaces existing)
        #[arg(long)]
        tags: Option<String>,
    },

    /// Delete a knowledge unit (requires interactive confirmation)
    Delete {
        /// Unit ID to delete
        id: String,
    },

    /// Health-check the knowledge store
    Scan {
        /// Days without marks before a unit is considered stale
        #[arg(long, default_value = "90")]
        stale_days: u32,
    },

    /// Manage human-readable slugs pointing at units
    Slug {
        #[command(subcommand)]
        action: SlugAction,
    },

    /// Emit typed knowledge units as build artifacts for external tools
    Emit {
        /// Target tool format
        #[arg(long)]
        target: EmitTarget,

        /// Type of knowledge unit to emit
        #[arg(long, rename_all = "snake_case")]
        r#type: EmitType,
    },
}

#[derive(Clone, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum EmitTarget {
    ClaudeCode,
}

#[derive(Clone, ValueEnum)]
enum EmitType {
    Aspect,
}

#[derive(Subcommand)]
enum SlugAction {
    /// Bind a slug to a unit id (creates or moves the slug)
    Set {
        /// Slug name
        slug: String,
        /// Target unit id
        id: String,
    },

    /// Remove a slug (no-op if absent)
    Unset {
        /// Slug name
        slug: String,
    },

    /// List every slug
    List,
}

#[derive(Clone, ValueEnum)]
enum UnitType {
    Fact,
    Procedure,
    Principle,
    Preference,
    Lesson,
    Idea,
    Aspect,
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
            UnitType::Aspect => "aspect",
        }
    }
}

#[derive(Clone, ValueEnum)]
#[value(rename_all = "snake_case")]
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
#[value(rename_all = "kebab-case")]
enum PrimeFilter {
    None,
    Standard,
    TagVote,
}

impl PrimeFilter {
    fn to_strategy(&self) -> ask::FilterStrategy {
        match self {
            PrimeFilter::None => ask::FilterStrategy::None,
            PrimeFilter::Standard => ask::FilterStrategy::Standard,
            PrimeFilter::TagVote => ask::FilterStrategy::TagVote,
        }
    }

    fn needs_claude(&self) -> bool {
        matches!(self, PrimeFilter::Standard)
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

/// Build a per-unit slug hint for lean output. Each entry is the first slug
/// bound to that unit id (alphabetical order), or `None` when the unit has no
/// slug. N small in practice (list/search return tens of rows); N+1 queries
/// acceptable until corpora grow.
fn build_slug_map(conn: &rusqlite::Connection, units: &[db::Unit]) -> Result<Vec<Option<String>>> {
    let mut out = Vec::with_capacity(units.len());
    for u in units {
        let slugs = db::get_slugs_for_unit(conn, &u.id)?;
        out.push(slugs.into_iter().next());
    }
    Ok(out)
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
            tags,
        } => {
            let id = if let Some(tag_str) = tags {
                let tag_vec: Vec<String> = tag_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                db::add_unit_full(&conn, &content, r#type.as_str(), &source, &tag_vec)?
            } else {
                db::add_unit(&conn, &content, r#type.as_str(), &source)?
            };
            display::print_added(&id, cli.json);
            let linked = db::auto_link(&conn, &id)?;
            if linked > 0 && !cli.json {
                println!("  auto-linked to {linked} existing unit(s)");
            }
        }
        Command::Show { id } => {
            let id = db::resolve_id(&conn, &id)?;
            let unit = db::get_unit(&conn, &id)?;
            let outgoing = db::get_links_from(&conn, &id)?;
            let incoming = db::get_links_to(&conn, &id)?;
            let slugs = db::get_slugs_for_unit(&conn, &id)?;
            display::print_unit(&unit, &outgoing, &incoming, &slugs, cli.json);
        }
        Command::Link {
            from_id,
            to_id,
            rel,
        } => {
            let from_id = db::resolve_id(&conn, &from_id)?;
            let to_id = db::resolve_id(&conn, &to_id)?;
            db::add_link(&conn, &from_id, &to_id, rel.as_str())?;
            display::print_linked(&from_id, &to_id, rel.as_str(), cli.json);
        }
        Command::Drop { content, source } => {
            let id = db::drop_item(&conn, &content, &source)?;
            display::print_dropped(&id, cli.json);
        }
        Command::Promote { id, r#type } => {
            let unit_id = db::promote_item(&conn, &id, r#type.as_str())?;
            display::print_added(&unit_id, cli.json);
        }
        Command::Inbox => {
            let items = db::list_inbox(&conn)?;
            display::print_inbox(&items, cli.json);
        }
        Command::List { r#type, full } => {
            let filter = r#type.as_ref().map(|t| t.as_str());
            let units = db::list_units(&conn, filter)?;
            if full {
                display::print_units(&units, cli.json);
            } else {
                let slug_map = build_slug_map(&conn, &units)?;
                display::print_units_lean(&units, &slug_map, cli.json);
            }
        }
        Command::Search {
            query,
            r#type,
            full,
        } => {
            let filter = r#type.as_ref().map(|t| t.as_str());
            let units = db::search_units(&conn, &query, filter)?;
            if full {
                display::print_units(&units, cli.json);
            } else {
                let slug_map = build_slug_map(&conn, &units)?;
                display::print_units_lean(&units, &slug_map, cli.json);
            }
        }
        Command::Backup => {
            let path = db::create_backup(&conn)?;
            display::print_backup_created(&path, cli.json);
        }
        Command::Mark { id, kind } => {
            let id = db::resolve_id(&conn, &id)?;
            let confidence = db::add_mark(&conn, &id, kind.as_str(), kind.delta())?;
            display::print_marked(&id, kind.as_str(), confidence, cli.json);
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
                            &item.id,
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
        Command::Prime { task, filter } => {
            if filter.needs_claude() {
                digest::check_claude()?;
            }
            let result = ask::prime(&conn, &task, filter.to_strategy(), cli.debug)?;
            display::print_prime(&result, cli.json);
        }
        Command::Ask {
            query,
            synthesize,
            r#type,
        } => {
            if synthesize {
                digest::check_claude()?;
            }
            let filter = r#type.as_ref().map(|t| t.as_str());
            let result = ask::ask(&conn, &query, synthesize, cli.debug, filter)?;
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
        Command::Edit {
            id,
            content,
            r#type,
            source,
            tags,
        } => {
            let id = db::resolve_id(&conn, &id)?;
            let tag_vec: Option<Vec<String>> = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let unit = db::update_unit(
                &conn,
                &id,
                content.as_deref(),
                r#type.as_ref().map(|t| t.as_str()),
                source.as_deref(),
                tag_vec.as_deref(),
            )?;
            let outgoing = db::get_links_from(&conn, &id)?;
            let incoming = db::get_links_to(&conn, &id)?;
            let slugs = db::get_slugs_for_unit(&conn, &id)?;
            display::print_unit(&unit, &outgoing, &incoming, &slugs, cli.json);
        }
        Command::Delete { id } => {
            let id = db::resolve_id(&conn, &id)?;
            db::delete_unit(&conn, &id)?;
            display::print_deleted(&id, cli.json);
        }
        Command::Scan { stale_days } => {
            let result = db::scan(&conn, stale_days)?;
            display::print_scan(&result, cli.json);
        }
        Command::Slug { action } => match action {
            SlugAction::Set { slug, id } => {
                db::set_slug(&conn, &slug, &id)?;
                display::print_slug_set(&slug, &id, cli.json);
            }
            SlugAction::Unset { slug } => {
                let removed = db::unset_slug(&conn, &slug)?;
                display::print_slug_unset(&slug, removed, cli.json);
            }
            SlugAction::List => {
                let rows = db::list_slugs(&conn)?;
                display::print_slug_list(&rows, cli.json);
            }
        },
        Command::Emit { target, r#type } => {
            let EmitTarget::ClaudeCode = target;
            let EmitType::Aspect = r#type;
            let target_dir = emit::claude_agents_dir()?;
            let result = emit::emit_claude_code_aspects(&conn, &target_dir)?;
            display::print_emit_result(&result, &target_dir, cli.json);
        }
        Command::Restore { .. } => unreachable!(),
    }

    Ok(())
}
