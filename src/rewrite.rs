//! P3a — `simaris rewrite <id>` editor core (no LLM).
//!
//! Opens `$EDITOR` (override: `SIMARIS_EDITOR`; fallback: `vi`) with a
//! type-aware skeleton pre-filled with the unit's existing content. After
//! the editor exits, strips leading `#` header-comment lines, validates the
//! frontmatter (if any), and writes the new content back — preserving
//! identity (id, tags, links, marks, slugs, created).
//!
//! Buffer composition rules (spec `frontmatter-p3a`):
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
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::db;
use crate::frontmatter;
use crate::size_guard;

/// LLM call timeout — claude is short-lived but unpredictable when API is hot.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(60);

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

// ---------------------------------------------------------------------------
// P3b — `rewrite --suggest` (LLM pre-fill)
// ---------------------------------------------------------------------------

/// Resolve the LLM model — mirrors `digest::model()` so users can override
/// per-call via `SIMARIS_MODEL`.
fn llm_model() -> String {
    std::env::var("SIMARIS_MODEL").unwrap_or_else(|_| "sonnet".to_string())
}

/// Type-aware schema doc snippet baked into the LLM prompt. Listed in
/// frontmatter-p1 spec order; fields without a stored schema (preference,
/// idea) return an empty doc and trigger a body-only formatting prompt.
fn schema_doc(unit_type: &str) -> &'static str {
    match unit_type {
        "procedure" => {
            "  trigger: scalar — condition that fires the procedure\n  \
             check: scalar — verification condition (\"done\" shape)\n  \
             cadence: scalar — how often (every-commit, daily, ship-once, ...)\n  \
             caveat: scalar — edge case or pitfall\n  \
             prereq: list — prerequisites (one bullet each)\n  \
             refs: list — slug or id refs\n"
        }
        "aspect" => {
            "  role: scalar — one-line role summary\n  \
             dispatches_to: list — subagent roles dispatched\n  \
             handles_directly: list — tasks handled in-aspect\n  \
             refs: list — slug or id refs\n"
        }
        "fact" => {
            "  scope: scalar — where the fact applies\n  \
             evidence: scalar — source / how known\n  \
             refs: list — slug or id refs\n"
        }
        "principle" => {
            "  tension: scalar — underlying design tension\n  \
             refs: list — slug or id refs\n"
        }
        "lesson" => {
            "  context: scalar — surrounding context that gave rise to lesson\n  \
             scope: scalar — where the lesson applies\n  \
             refs: list — slug or id refs\n"
        }
        // preference, idea — no schema; LLM returns body only.
        _ => "",
    }
}

/// Hand-curated few-shot exemplars per type. Two per typed schema. Voice =
/// cavespeak (verb+noun, no the/is/that/a). v1 = inline; future = auto-pick
/// from highest-mark same-type units in store.
fn few_shot(unit_type: &str) -> &'static str {
    match unit_type {
        "procedure" => {
            "Example 1 input:\n\
             Run cargo test before commit. If tests fail, fix the failure not the test. After commit, push to remote.\n\n\
             Example 1 output:\n\
             ---\n\
             trigger: \"before commit\"\n\
             check: \"cargo test green\"\n\
             cadence: \"every commit\"\n\
             caveat: \"fix code, not test\"\n\
             prereq:\n  - cargo build clean\n\
             refs: []\n\
             ---\n\n\
             # Test before commit\n\n\
             Run `cargo test`. Failure -> fix code, not test. Then commit + push.\n\n\
             Example 2 input:\n\
             After every release, install via brew (brew upgrade or brew install) instead of manual cargo build + sudo cp. Validates the full distribution path.\n\n\
             Example 2 output:\n\
             ---\n\
             trigger: \"post-release validate install path\"\n\
             check: \"brew install + binary runs\"\n\
             cadence: \"every release\"\n\
             caveat: \"no manual cargo + sudo cp\"\n\
             prereq:\n  - release tagged + pushed\n\
             refs: []\n\
             ---\n\n\
             # Brew install validate post-release\n\n\
             After release cut, run `brew upgrade` or `brew install simonspoon/tap/<tool>`. Confirms full distribution path. No manual `cargo build + sudo cp`.\n"
        }
        "aspect" => {
            "Example 1 input:\n\
             The committer agent writes commit messages and runs git commit. It does not push. It dispatches nothing else.\n\n\
             Example 1 output:\n\
             ---\n\
             role: \"write commit message + run git commit\"\n\
             dispatches_to: []\n\
             handles_directly:\n  - draft commit message\n  - run git commit\n\
             refs: []\n\
             ---\n\n\
             # Committer\n\n\
             Write commit message. Run `git commit`. No push. No further dispatch.\n\n\
             Example 2 input:\n\
             The orchestrator coordinates work across multiple subagents. It dispatches to worker, researcher, and committer. It does not write code itself.\n\n\
             Example 2 output:\n\
             ---\n\
             role: \"coordinate multi-agent work\"\n\
             dispatches_to:\n  - worker\n  - researcher\n  - committer\n\
             handles_directly:\n  - dispatch routing\n  - status aggregation\n\
             refs: []\n\
             ---\n\n\
             # Orchestrator\n\n\
             Route work across subagents. Dispatch worker, researcher, committer. No code write self.\n"
        }
        "fact" => {
            "Example 1 input:\n\
             The simaris CLI binary is at ./target/release/simaris. Built via cargo build --release. Installs via cargo install --path .\n\n\
             Example 1 output:\n\
             ---\n\
             scope: \"simaris build + install\"\n\
             evidence: \"cargo build --release output path\"\n\
             refs: []\n\
             ---\n\n\
             # simaris binary path\n\n\
             Path: `./target/release/simaris`. Build: `cargo build --release`. Install: `cargo install --path .`.\n\n\
             Example 2 input:\n\
             SQLite uses an FTS5 virtual table for full-text search. Synced to units table via triggers on insert/update/delete.\n\n\
             Example 2 output:\n\
             ---\n\
             scope: \"simaris search backend\"\n\
             evidence: \"src/db.rs schema + triggers\"\n\
             refs: []\n\
             ---\n\n\
             # FTS5 sync triggers\n\n\
             `units_fts` virtual table mirrors `units`. Triggers sync on insert / update / delete.\n"
        }
        "principle" => {
            "Example 1 input:\n\
             Always pick the simplest approach first. Over-engineering wastes time. Iterate from simple to complex only when necessary.\n\n\
             Example 1 output:\n\
             ---\n\
             tension: \"simple now vs flexible later\"\n\
             refs: []\n\
             ---\n\n\
             # Simplest first\n\n\
             Pick simplest approach. Iterate to complex only when forced. Over-engineer wastes time.\n\n\
             Example 2 input:\n\
             Verify before reporting status. Re-run commands fresh rather than presenting cached output as current truth.\n\n\
             Example 2 output:\n\
             ---\n\
             tension: \"fast answer vs current truth\"\n\
             refs: []\n\
             ---\n\n\
             # Verify before report\n\n\
             Re-run commands. Fresh output. No present cached as current.\n"
        }
        "lesson" => {
            "Example 1 input:\n\
             Adding NOT NULL column to a large table without a default crashes concurrent writes. Backfill default first, then add the constraint.\n\n\
             Example 1 output:\n\
             ---\n\
             context: \"schema migration on hot table\"\n\
             scope: \"Postgres NOT NULL constraint adds\"\n\
             refs: []\n\
             ---\n\n\
             # Backfill before NOT NULL\n\n\
             Add NOT NULL on hot table -> concurrent write crash. Fix: backfill default first. Add constraint after.\n\n\
             Example 2 input:\n\
             Claude API prompt cache uses a 5-minute TTL. Sleeping past 300 seconds breaks the cache. Stay under 270 seconds for active polling.\n\n\
             Example 2 output:\n\
             ---\n\
             context: \"Claude API prompt cache\"\n\
             scope: \"schedule wakeup intervals\"\n\
             refs: []\n\
             ---\n\n\
             # Prompt cache TTL\n\n\
             Cache TTL = 5 min. Sleep > 300s -> cache miss. Active poll: stay < 270s.\n"
        }
        // preference, idea — no schema; one body-only example shows voice.
        _ => {
            "Example 1 input:\n\
             Default to Sonnet when spawning agents for lightweight tasks (summarization, deduplication, filtering, formatting). Reserve Opus for complex reasoning.\n\n\
             Example 1 output:\n\
             # Spawn-agent model default\n\n\
             Lightweight task (summarize, dedupe, filter, format) -> Sonnet. Complex reason -> Opus.\n"
        }
    }
}

/// Build the LLM prompt — schema + cavespeak voice rules + 2-shot exemplars +
/// the unit body to convert. Mirrors digest.rs's "single big string" approach.
pub fn build_prompt(unit: &db::Unit) -> String {
    let schema = schema_doc(&unit.unit_type);
    let shots = few_shot(&unit.unit_type);
    let schema_block = if schema.is_empty() {
        format!(
            "Type `{}` carries no frontmatter schema. Output body only, lightly formatted, no `---` fences.\n",
            unit.unit_type
        )
    } else {
        format!("Schema for type `{}`:\n{schema}", unit.unit_type)
    };

    format!(
        "You are simaris frontmatter-migration assistant. Convert the prose unit below into structured form per the simaris frontmatter schema.\n\n\
         {schema_block}\n\
         Constraints:\n\
         - Body content stays verbatim except: whitespace, list format, markdown fix-ups.\n\
         - NO word add, remove, or reword in body. Preserve every sentence.\n\
         - Frontmatter scalar values you author. Use cavespeak voice (verb+noun, no the/is/that/a, short words). Shell stays normal.\n\
         - Output exactly: YAML frontmatter between `---` fences, blank line, body. No preamble, no markdown code fence around the unit, no trailing commentary.\n\n\
         {shots}\n\
         Now convert this unit:\n\n\
         {body}\n",
        body = unit.content,
    )
}

/// Spawn `claude -p --model <m> <prompt>` with a hard timeout. Returns stdout
/// as a UTF-8 string on success. On timeout the child is killed and we
/// `wait()` to reap it so we don't leak zombies. Polling at 100 ms keeps the
/// CPU cost negligible against a multi-second LLM call.
fn call_claude(prompt: &str) -> Result<String> {
    let mut child = Command::new("claude")
        .args(["-p", "--model", &llm_model(), prompt])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn claude")?;

    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");

    // Drain pipes in background threads — a full pipe buffer would deadlock
    // the child before it exits.
    let stdout_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stdout_pipe).read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stderr_pipe).read_to_end(&mut buf);
        buf
    });

    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().context("failed to poll claude")? {
            Some(s) => break s,
            None => {
                if start.elapsed() > CLAUDE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("claude timeout after {}s", CLAUDE_TIMEOUT.as_secs());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let out_buf = stdout_handle.join().unwrap_or_default();
    let err_buf = stderr_handle.join().unwrap_or_default();

    if !status.success() {
        bail!(
            "claude failed: {}",
            String::from_utf8_lossy(&err_buf).trim()
        );
    }

    String::from_utf8(out_buf).context("claude stdout not UTF-8")
}

/// Strip a leading markdown code fence (` ```yaml`, ` ```` , etc.) if the LLM
/// wraps its answer despite the prompt. Mirrors the `digest::classify` strip.
fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```yaml")
        .or_else(|| s.strip_prefix("```markdown"))
        .or_else(|| s.strip_prefix("```md"))
        .or_else(|| s.strip_prefix("```"))
        .map(|x| x.trim_start_matches('\n'))
        .unwrap_or(s);
    s.strip_suffix("```").map(|x| x.trim_end()).unwrap_or(s)
}

/// Compose the editor buffer for `--suggest`: 3-line header + ORIGINAL block
/// + LLM (or skeleton) draft.
///
/// Each original line gets a `# ` prefix so the existing
/// `strip_header_comments` consumes the whole reference block before
/// validation runs.
pub fn compose_suggest_buffer(unit: &db::Unit, draft: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# simaris rewrite -- id: {}  type: {}\n",
        unit.id, unit.unit_type
    ));
    out.push_str("# Save + quit to apply. Empty file to abort.\n");
    out.push_str("# Comment lines starting `#` at top stripped automatically.\n");
    out.push_str("#\n");
    out.push_str("# ORIGINAL BODY (reference -- strip on save):\n");
    for line in unit.content.lines() {
        if line.is_empty() {
            out.push_str("#\n");
        } else {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
    // Exactly one blank line separator so strip_header_comments consumes the
    // header + blank in a single pass and leaves the draft fence at byte 0.
    out.push('\n');
    out.push_str(draft);
    if !draft.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Prose-fallback skeleton for `--suggest` LLM failure: type skeleton on top
/// of the original body. Mirrors the prose branch of `compose_buffer`.
fn skeleton_with_body(unit: &db::Unit) -> String {
    let skel = skeleton_for(&unit.unit_type);
    if skel.is_empty() {
        unit.content.clone()
    } else {
        format!("{skel}{}", unit.content)
    }
}

/// Try LLM draft. On any failure, log to stderr and return the
/// `skeleton + original body` fallback so the caller still has something
/// shippable to drop into the editor or print to stdout.
fn obtain_draft(unit: &db::Unit) -> String {
    // 1. claude on PATH?
    if let Err(e) = crate::digest::check_claude() {
        eprintln!("simaris: LLM failed: {e}, falling back to skeleton");
        return skeleton_with_body(unit);
    }

    // 2. invoke claude.
    let prompt = build_prompt(unit);
    let raw = match call_claude(&prompt) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("simaris: LLM failed: {e}, falling back to skeleton");
            return skeleton_with_body(unit);
        }
    };
    let draft = strip_code_fence(&raw).to_string();

    // 3. validate frontmatter shape (only when the type carries a schema).
    //    A typed unit MUST come back with a frontmatter block — if the LLM
    //    skipped fences entirely, the schema is lost; refuse + fall back.
    if !schema_doc(&unit.unit_type).is_empty() {
        if !draft.starts_with("---\n") {
            eprintln!(
                "simaris: LLM output invalid: missing frontmatter fences, falling back to skeleton"
            );
            return skeleton_with_body(unit);
        }
        if let Err(e) = frontmatter::validate(&draft, &unit.unit_type) {
            eprintln!("simaris: LLM output invalid: {e}, falling back to skeleton");
            return skeleton_with_body(unit);
        }
    }

    draft
}

/// Entry point for `simaris rewrite --suggest <id>`. Builds an LLM draft (or
/// skeleton fallback), gates it through the size guard, and either prints the
/// draft to stdout (`--dry-run`) or seeds the editor with the suggest buffer
/// and runs the existing P3a save path.
pub fn run_suggest(
    conn: &Connection,
    id_or_slug: &str,
    dry_run: bool,
    force: bool,
    flow: bool,
) -> Result<()> {
    let id = db::resolve_id(conn, id_or_slug)?;
    let unit = db::get_unit(conn, &id)?;

    let draft = obtain_draft(&unit);

    // Size guard mirrors add/edit. Applies to the draft body the user is
    // about to commit to (or stdout under --dry-run). Existing tags apply
    // because rewrite preserves them.
    size_guard::check_size(&draft, &unit.tags, flow, force)?;

    if dry_run {
        // Stdout the draft, no editor, no DB write.
        print!("{draft}");
        if !draft.ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    // Editor flow: seed buffer with header + ORIGINAL block + draft, hand off
    // to P3a's save path (header-strip → no-op check → validate → write).
    //
    // No-op semantics differ from P3a: under `--suggest` the seeded draft
    // already differs from DB content. "User reviewed + saved without further
    // edit" means *accept the suggestion*, not "skip the write". We compare
    // the post-edit buffer against DB content (`unit.content`) instead of the
    // seeded buffer.
    let buffer = compose_suggest_buffer(&unit, &draft);

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

    if trimmed.is_empty() {
        eprintln!("rewrite aborted: empty buffer, unit unchanged");
        return Ok(());
    }
    if buffers_equal(&unit.content, &stripped) {
        eprintln!("rewrite no-op: no changes, unit unchanged");
        return Ok(());
    }

    frontmatter::validate(&stripped, &unit.unit_type)
        .context("rewrite rejected: invalid frontmatter")?;

    db::update_unit(conn, &id, Some(&stripped), None, None, None)?;
    eprintln!("rewrote unit {id}");
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
