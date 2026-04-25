mod ask;
mod db;
mod digest;
mod display;
mod emit;
mod frontmatter;
mod rewrite;
mod size_guard;

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
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Add a knowledge unit
    Add {
        /// Content of the unit (optional when `--from-file` is used)
        content: Option<String>,

        /// Type of knowledge unit
        #[arg(long, rename_all = "snake_case")]
        r#type: UnitType,

        /// Source of the unit
        #[arg(long, default_value = "inbox")]
        source: String,

        /// Tags (comma-separated)
        #[arg(long)]
        tags: Option<String>,

        /// Override hard size threshold (still warns)
        #[arg(long)]
        force: bool,

        /// Treat body as a flow sequence — bypass size warning
        #[arg(long)]
        flow: bool,

        /// Read content verbatim from file path (mutex with any field flag)
        #[arg(long, value_name = "PATH")]
        from_file: Option<String>,

        // --- procedure-only -----------------------------------------------
        /// procedure: condition that fires the procedure
        #[arg(long)]
        trigger: Option<String>,

        /// procedure: verification condition
        #[arg(long)]
        check: Option<String>,

        /// procedure: edge case or caveat
        #[arg(long)]
        caveat: Option<String>,

        /// procedure: prerequisite (repeatable)
        #[arg(long)]
        prereq: Vec<String>,

        /// procedure: how often the procedure runs
        #[arg(long)]
        cadence: Option<String>,

        // --- aspect-only --------------------------------------------------
        /// aspect: role the aspect plays
        #[arg(long)]
        role: Option<String>,

        /// aspect: subagent this aspect dispatches to (repeatable)
        #[arg(long = "dispatches-to")]
        dispatches_to: Vec<String>,

        /// aspect: task this aspect handles directly (repeatable)
        #[arg(long = "handles-directly")]
        handles_directly: Vec<String>,

        // --- fact + lesson ------------------------------------------------
        /// fact / lesson: scope where the unit applies
        #[arg(long)]
        scope: Option<String>,

        // --- fact-only ----------------------------------------------------
        /// fact: evidence supporting the claim
        #[arg(long)]
        evidence: Option<String>,

        // --- principle-only -----------------------------------------------
        /// principle: underlying design tension
        #[arg(long)]
        tension: Option<String>,

        // --- lesson-only --------------------------------------------------
        /// lesson: surrounding context that gave rise to the lesson
        #[arg(long)]
        context: Option<String>,

        // --- shared -------------------------------------------------------
        /// Reference (repeatable) — valid on procedure / aspect / fact / principle / lesson
        #[arg(long = "ref")]
        refs: Vec<String>,
    },

    /// Show a knowledge unit
    Show {
        /// Unit ID
        id: String,

        /// Print content verbatim (including `---` fences) — skip parsing
        #[arg(long)]
        raw: bool,
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

        /// Override hard size threshold (still warns)
        #[arg(long)]
        force: bool,

        /// Treat body as a flow sequence — bypass size warning
        #[arg(long)]
        flow: bool,

        /// Replace entire content verbatim — skip frontmatter merge (legacy,
        /// destructive on schema'd units).
        #[arg(long)]
        replace_all: bool,

        /// Read content verbatim from file path (mutex with field flags + --replace-all)
        #[arg(long, value_name = "PATH")]
        from_file: Option<String>,

        // --- procedure-only -----------------------------------------------
        /// procedure: condition that fires the procedure
        #[arg(long)]
        trigger: Option<String>,

        /// procedure: verification condition
        #[arg(long)]
        check: Option<String>,

        /// procedure: edge case or caveat
        #[arg(long)]
        caveat: Option<String>,

        /// procedure: prerequisite (repeatable)
        #[arg(long)]
        prereq: Vec<String>,

        /// procedure: how often the procedure runs
        #[arg(long)]
        cadence: Option<String>,

        // --- aspect-only --------------------------------------------------
        /// aspect: role the aspect plays
        #[arg(long)]
        role: Option<String>,

        /// aspect: subagent this aspect dispatches to (repeatable)
        #[arg(long = "dispatches-to")]
        dispatches_to: Vec<String>,

        /// aspect: task this aspect handles directly (repeatable)
        #[arg(long = "handles-directly")]
        handles_directly: Vec<String>,

        // --- fact + lesson ------------------------------------------------
        /// fact / lesson: scope where the unit applies
        #[arg(long)]
        scope: Option<String>,

        // --- fact-only ----------------------------------------------------
        /// fact: evidence supporting the claim
        #[arg(long)]
        evidence: Option<String>,

        // --- principle-only -----------------------------------------------
        /// principle: underlying design tension
        #[arg(long)]
        tension: Option<String>,

        // --- lesson-only --------------------------------------------------
        /// lesson: surrounding context that gave rise to the lesson
        #[arg(long)]
        context: Option<String>,

        // --- shared -------------------------------------------------------
        /// Reference (repeatable) — valid on procedure / aspect / fact / principle / lesson
        #[arg(long = "ref")]
        refs: Vec<String>,
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

        /// List units without a frontmatter block, ranked for rewrite
        /// priority (aspect first, then mark count, then confidence).
        /// Overrides stale-days.
        #[arg(long)]
        unstructured: bool,

        /// Narrow `--unstructured` to a single unit type.
        #[arg(long = "type", rename_all = "snake_case")]
        type_filter: Option<UnitType>,
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

    /// Rewrite a unit in `$EDITOR` with a type-aware skeleton (P3a) or LLM
    /// pre-fill (P3b via `--suggest`).
    Rewrite {
        /// Unit id or slug
        id: String,

        /// Open buffer with skeleton only, no existing body
        #[arg(long, conflicts_with = "suggest")]
        template_only: bool,

        /// Pre-fill the editor buffer with an LLM-drafted unit (P3b)
        #[arg(long)]
        suggest: bool,

        /// Print the LLM draft to stdout instead of opening the editor.
        /// Only valid with `--suggest`. No DB change.
        #[arg(long, requires = "suggest")]
        dry_run: bool,

        /// Override hard size threshold on the LLM draft (still warns)
        #[arg(long, requires = "suggest")]
        force: bool,

        /// Treat LLM-drafted body as a flow sequence — bypass size warning
        #[arg(long, requires = "suggest")]
        flow: bool,
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

/// Parse a database-stored type string back into the `UnitType` enum.
/// Used by `edit` to determine the effective type when no `--type` flag is
/// passed. Unknown values error rather than defaulting silently.
fn parse_unit_type(s: &str) -> Result<UnitType> {
    Ok(match s {
        "fact" => UnitType::Fact,
        "procedure" => UnitType::Procedure,
        "principle" => UnitType::Principle,
        "preference" => UnitType::Preference,
        "lesson" => UnitType::Lesson,
        "idea" => UnitType::Idea,
        "aspect" => UnitType::Aspect,
        other => anyhow::bail!("unknown unit type `{other}` stored in db"),
    })
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

/// Borrowed view of all per-type frontmatter flags. Used by both `add` and
/// `edit` (P1.5) for validation + serialization.
struct TypeFlags<'a> {
    trigger: &'a Option<String>,
    check: &'a Option<String>,
    caveat: &'a Option<String>,
    prereq: &'a [String],
    cadence: &'a Option<String>,
    role: &'a Option<String>,
    dispatches_to: &'a [String],
    handles_directly: &'a [String],
    scope: &'a Option<String>,
    evidence: &'a Option<String>,
    tension: &'a Option<String>,
    context: &'a Option<String>,
    refs: &'a [String],
}

impl TypeFlags<'_> {
    /// Does this flag set contain any field value?
    fn any_set(&self) -> bool {
        self.trigger.is_some()
            || self.check.is_some()
            || self.caveat.is_some()
            || !self.prereq.is_empty()
            || self.cadence.is_some()
            || self.role.is_some()
            || !self.dispatches_to.is_empty()
            || !self.handles_directly.is_empty()
            || self.scope.is_some()
            || self.evidence.is_some()
            || self.tension.is_some()
            || self.context.is_some()
            || !self.refs.is_empty()
    }
}

/// Check that each populated per-type flag is valid for the given unit type.
/// Shared between `add` and `edit`; callers handle their own mutex and
/// content-presence rules upstream.
fn validate_flag_type_compat(unit_type: &UnitType, flags: &TypeFlags<'_>) -> Result<()> {
    let check = |present: bool, name: &str, valid: &[UnitType]| -> Result<()> {
        if !present {
            return Ok(());
        }
        if valid.iter().any(|v| v.as_str() == unit_type.as_str()) {
            return Ok(());
        }
        let joined: Vec<&str> = valid.iter().map(|v| v.as_str()).collect();
        anyhow::bail!(
            "--{name} is not valid for --type {}; valid types: {}",
            unit_type.as_str(),
            joined.join(", ")
        );
    };

    use UnitType::*;
    check(flags.trigger.is_some(), "trigger", &[Procedure])?;
    check(flags.check.is_some(), "check", &[Procedure])?;
    check(flags.caveat.is_some(), "caveat", &[Procedure])?;
    check(!flags.prereq.is_empty(), "prereq", &[Procedure])?;
    check(flags.cadence.is_some(), "cadence", &[Procedure])?;
    check(flags.role.is_some(), "role", &[Aspect])?;
    check(!flags.dispatches_to.is_empty(), "dispatches-to", &[Aspect])?;
    check(
        !flags.handles_directly.is_empty(),
        "handles-directly",
        &[Aspect],
    )?;
    check(flags.scope.is_some(), "scope", &[Fact, Lesson])?;
    check(flags.evidence.is_some(), "evidence", &[Fact])?;
    check(flags.tension.is_some(), "tension", &[Principle])?;
    check(flags.context.is_some(), "context", &[Lesson])?;
    check(
        !flags.refs.is_empty(),
        "ref",
        &[Procedure, Aspect, Fact, Principle, Lesson],
    )?;
    Ok(())
}

/// Validate `add`-path rules: per-type flag compatibility, mutex with
/// `--from-file`, and positional content presence.
fn validate_add_flags(
    unit_type: &UnitType,
    flags: &TypeFlags<'_>,
    from_file: Option<&str>,
    content: Option<&str>,
) -> Result<()> {
    // Mutex check — from-file cannot combine with field flags.
    if from_file.is_some() && flags.any_set() {
        anyhow::bail!(
            "--from-file is mutually exclusive with per-type field flags \
             (--trigger, --check, --role, ...); pass fields inside the file's frontmatter"
        );
    }

    // Content presence — if no file, body must be supplied positionally.
    if from_file.is_none() && content.is_none() {
        anyhow::bail!("positional <content> required when --from-file is not used");
    }

    validate_flag_type_compat(unit_type, flags)
}

/// Validate `edit`-path rules (P1.5):
/// - `--from-file` + any field flag → mutex error.
/// - `--from-file` + `--replace-all` → mutex error (`--from-file` is already
///   total replacement).
/// - Per-type flag compatibility against the existing unit's type.
fn validate_edit_flags(
    unit_type: &UnitType,
    flags: &TypeFlags<'_>,
    from_file: Option<&str>,
    replace_all: bool,
) -> Result<()> {
    if from_file.is_some() && flags.any_set() {
        anyhow::bail!(
            "--from-file is mutually exclusive with per-type field flags \
             (--trigger, --check, --role, ...); pass fields inside the file's frontmatter"
        );
    }
    if from_file.is_some() && replace_all {
        anyhow::bail!(
            "--from-file and --replace-all are mutually exclusive; --from-file \
             already replaces content verbatim"
        );
    }

    validate_flag_type_compat(unit_type, flags)
}

/// Assemble the per-type `(key, FieldValue)` list in spec order for a given
/// unit type. Returns `None` for types that carry no schema (preference,
/// idea). The returned list always contains one entry per schema key — empty
/// `FieldValue`s stand for "flag not passed" and are skipped downstream by
/// both `build_frontmatter` and `merge_frontmatter`.
fn type_field_list<'a>(
    unit_type: &UnitType,
    flags: &'a TypeFlags<'_>,
) -> Option<Vec<(&'static str, frontmatter::FieldValue)>> {
    use frontmatter::FieldValue;

    let scalar = |v: &Option<String>| FieldValue::Scalar(v.clone().unwrap_or_default());
    let list = |v: &'a [String]| FieldValue::List(v.to_vec());

    let fields: Vec<(&'static str, FieldValue)> = match unit_type {
        UnitType::Procedure => vec![
            ("trigger", scalar(flags.trigger)),
            ("check", scalar(flags.check)),
            ("cadence", scalar(flags.cadence)),
            ("caveat", scalar(flags.caveat)),
            ("prereq", list(flags.prereq)),
            ("refs", list(flags.refs)),
        ],
        UnitType::Aspect => vec![
            ("role", scalar(flags.role)),
            ("dispatches_to", list(flags.dispatches_to)),
            ("handles_directly", list(flags.handles_directly)),
            ("refs", list(flags.refs)),
        ],
        UnitType::Fact => vec![
            ("scope", scalar(flags.scope)),
            ("evidence", scalar(flags.evidence)),
            ("refs", list(flags.refs)),
        ],
        UnitType::Principle => vec![
            ("tension", scalar(flags.tension)),
            ("refs", list(flags.refs)),
        ],
        UnitType::Lesson => vec![
            ("context", scalar(flags.context)),
            ("scope", scalar(flags.scope)),
            ("refs", list(flags.refs)),
        ],
        UnitType::Preference | UnitType::Idea => return None,
    };
    Some(fields)
}

/// Build a YAML frontmatter block for the given unit type from populated
/// flags. Field order follows the per-type schema in the frontmatter-p1 spec.
/// Returns `None` when no fields are set.
fn build_type_frontmatter(unit_type: &UnitType, flags: &TypeFlags<'_>) -> Option<String> {
    let fields = type_field_list(unit_type, flags)?;
    frontmatter::build_frontmatter(&fields)
}

/// Compose the new stored content for an edit, applying P1.5 merge rules.
///
/// - `existing_content` — current `content` column.
/// - `unit_type` — resolved type (existing type, or overridden via `--type`).
/// - `flags` — per-type field flags. Any populated flag merges into fm.
/// - `new_body` — value of `--content`. `None` means leave body as-is.
/// - `replace_all` — when `true`, skip merge and return `new_body` verbatim
///   (legacy clobber behavior).
///
/// Returns `None` when nothing changes (neither flags nor `--content`).
fn compose_edit_content(
    existing_content: &str,
    unit_type: &UnitType,
    flags: &TypeFlags<'_>,
    new_body: Option<&str>,
    replace_all: bool,
) -> Option<String> {
    // Legacy clobber — ignore existing frontmatter entirely.
    if replace_all {
        return new_body.map(str::to_string);
    }

    let parsed = frontmatter::parse(existing_content);
    let has_fm = parsed.frontmatter.is_some();
    let any_flag = flags.any_set();

    // Pure-body edit on prose unit: nothing to merge, caller may still use
    // the new content as-is. Return None to signal "pass through".
    if !any_flag && !has_fm {
        return new_body.map(str::to_string);
    }

    // Case: flags present, OR existing frontmatter we must preserve.
    let body = match new_body {
        Some(b) => b,
        None => parsed.body,
    };

    // Build the ordered override list for this type. Types without a schema
    // (preference, idea) just pass through — flag validation already rejects
    // field flags on those types upstream.
    let overrides = type_field_list(unit_type, flags).unwrap_or_default();

    let merged = frontmatter::merge_frontmatter(parsed.frontmatter.as_ref(), &overrides);
    match merged {
        Some(fm_block) => Some(format!("{fm_block}{body}")),
        // No fm (neither existing nor overrides) — body-only edit.
        None => Some(body.to_string()),
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
            tags,
            force,
            flow,
            from_file,
            trigger,
            check,
            caveat,
            prereq,
            cadence,
            role,
            dispatches_to,
            handles_directly,
            scope,
            evidence,
            tension,
            context,
            refs,
        } => {
            let flags = TypeFlags {
                trigger: &trigger,
                check: &check,
                caveat: &caveat,
                prereq: &prereq,
                cadence: &cadence,
                role: &role,
                dispatches_to: &dispatches_to,
                handles_directly: &handles_directly,
                scope: &scope,
                evidence: &evidence,
                tension: &tension,
                context: &context,
                refs: &refs,
            };
            validate_add_flags(&r#type, &flags, from_file.as_deref(), content.as_deref())?;

            let final_content = if let Some(path) = from_file.as_deref() {
                let body = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("failed to read --from-file `{path}`: {e}"))?;
                frontmatter::validate_from_file(&body)?;
                body
            } else {
                let body = content.unwrap_or_default();
                let fm_block = build_type_frontmatter(&r#type, &flags);
                match fm_block {
                    Some(block) => format!("{block}{body}"),
                    None => body,
                }
            };

            let tag_vec: Vec<String> = tags
                .as_deref()
                .map(|t| {
                    t.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            size_guard::check_size(&final_content, &tag_vec, flow, force)?;
            let id = if tags.is_some() {
                db::add_unit_full(&conn, &final_content, r#type.as_str(), &source, &tag_vec)?
            } else {
                db::add_unit(&conn, &final_content, r#type.as_str(), &source)?
            };
            display::print_added(&id, cli.json);
            let linked = db::auto_link(&conn, &id)?;
            if linked > 0 && !cli.json {
                println!("  auto-linked to {linked} existing unit(s)");
            }
        }
        Command::Show { id, raw } => {
            let id = db::resolve_id(&conn, &id)?;
            let unit = db::get_unit(&conn, &id)?;
            let outgoing = db::get_links_from(&conn, &id)?;
            let incoming = db::get_links_to(&conn, &id)?;
            let slugs = db::get_slugs_for_unit(&conn, &id)?;
            display::print_unit(&unit, &outgoing, &incoming, &slugs, cli.json, raw);
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
            force,
            flow,
            replace_all,
            from_file,
            trigger,
            check,
            caveat,
            prereq,
            cadence,
            role,
            dispatches_to,
            handles_directly,
            scope,
            evidence,
            tension,
            context,
            refs,
        } => {
            let id = db::resolve_id(&conn, &id)?;
            let flags = TypeFlags {
                trigger: &trigger,
                check: &check,
                caveat: &caveat,
                prereq: &prereq,
                cadence: &cadence,
                role: &role,
                dispatches_to: &dispatches_to,
                handles_directly: &handles_directly,
                scope: &scope,
                evidence: &evidence,
                tension: &tension,
                context: &context,
                refs: &refs,
            };

            // Resolve the effective type against which to validate field
            // flags. `--type` override wins; otherwise use the existing unit's
            // type. Load the existing unit once and reuse.
            let existing = db::get_unit(&conn, &id)?;
            let effective_type = match &r#type {
                Some(t) => t.clone(),
                None => parse_unit_type(&existing.unit_type)?,
            };

            validate_edit_flags(&effective_type, &flags, from_file.as_deref(), replace_all)?;

            // Compute the new content field (if any).
            //
            // Precedence:
            //   1. --from-file — verbatim replacement (validated).
            //   2. --replace-all + --content — verbatim replacement of body.
            //   3. flag-driven merge (optionally including new body).
            //   4. content-only: delegate to compose_edit_content which
            //      preserves existing frontmatter.
            let new_content: Option<String> = if let Some(path) = from_file.as_deref() {
                let body = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("failed to read --from-file `{path}`: {e}"))?;
                frontmatter::validate_from_file(&body)?;
                Some(body)
            } else if flags.any_set() || content.is_some() {
                compose_edit_content(
                    &existing.content,
                    &effective_type,
                    &flags,
                    content.as_deref(),
                    replace_all,
                )
            } else {
                None
            };

            let tag_vec: Option<Vec<String>> = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            // Only check size when content is being set — tag/source-only
            // edits do not touch body and must not retroactively complain.
            if let Some(ref nc) = new_content {
                let effective_tags: Vec<String> = match tag_vec {
                    Some(ref t) => t.clone(),
                    None => existing.tags.clone(),
                };
                size_guard::check_size(nc, &effective_tags, flow, force)?;
            }

            let unit = db::update_unit(
                &conn,
                &id,
                new_content.as_deref(),
                r#type.as_ref().map(|t| t.as_str()),
                source.as_deref(),
                tag_vec.as_deref(),
            )?;
            let outgoing = db::get_links_from(&conn, &id)?;
            let incoming = db::get_links_to(&conn, &id)?;
            let slugs = db::get_slugs_for_unit(&conn, &id)?;
            display::print_unit(&unit, &outgoing, &incoming, &slugs, cli.json, false);
        }
        Command::Delete { id } => {
            let id = db::resolve_id(&conn, &id)?;
            db::delete_unit(&conn, &id)?;
            display::print_deleted(&id, cli.json);
        }
        Command::Scan {
            stale_days,
            unstructured,
            type_filter,
        } => {
            if unstructured {
                let filter = type_filter.as_ref().map(UnitType::as_str);
                let rows = db::scan_unstructured(&conn, filter)?;
                display::print_scan_unstructured(&rows, cli.json);
            } else {
                let result = db::scan(&conn, stale_days)?;
                display::print_scan(&result, cli.json);
            }
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
        Command::Rewrite {
            id,
            template_only,
            suggest,
            dry_run,
            force,
            flow,
        } => {
            if suggest {
                rewrite::run_suggest(&conn, &id, dry_run, force, flow)?;
            } else {
                rewrite::run(&conn, &id, template_only)?;
            }
        }
        Command::Restore { .. } => unreachable!(),
    }

    Ok(())
}
