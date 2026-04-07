mod db;
mod display;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "simaris", about = "Knowledge unit storage")]
struct Cli {
    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

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

fn main() -> Result<()> {
    let cli = Cli::parse();
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
    }

    Ok(())
}
