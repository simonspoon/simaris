<p align="center">
  <img src="icon.png" width="128" height="128" alt="simaris">
</p>

# simaris

Knowledge management CLI with SQLite, FTS5 search, and graph-based linking.

## Overview

Simaris stores typed knowledge units in a local SQLite database with full-text search, graph-based relationships between units, and confidence scoring via feedback marks. It supports LLM-powered classification of raw input (digest) and synthesis of query results (ask). Built for LLM agents and developers who need structured, searchable knowledge that evolves over time.

## Installation

### Homebrew

```bash
brew install simonspoon/tap/simaris
```

### Cargo

```bash
cargo install --git https://github.com/simonspoon/simaris.git
```

### From Source

```bash
git clone https://github.com/simonspoon/simaris.git
cd simaris
cargo build --release
# Binary at ./target/release/simaris
```

## Quick Start

```bash
# Add a typed knowledge unit
simaris add "Rust edition 2024 requires cargo 1.85+" --type fact --tags "rust,toolchain"

# Search by content
simaris search "rust edition"

# Link related units
simaris link <id-1> <id-2> --rel related_to

# Record feedback (adjusts confidence)
simaris mark <id> --kind helpful

# Ask a question with LLM synthesis
simaris ask "What do I know about Rust editions?" --synthesize
```

## Commands

| Command | Description |
|---------|-------------|
| `add <content> --type <type>` | Create a typed knowledge unit |
| `show <id>` | Display a unit with its links |
| `edit <id> [--content] [--type] [--source] [--tags]` | Update an existing unit |
| `link <from> <to> --rel <relationship>` | Create a graph edge between units |
| `drop <content>` | Capture raw input to the inbox |
| `promote <id> --type <type>` | Convert an inbox item to a typed unit |
| `inbox` | List pending inbox items |
| `list [--type <type>] [--include-archived]` | List knowledge units |
| `search <query> [--type <type>] [--include-archived]` | Full-text search across units |
| `ask <query> [--synthesize] [--type <type>] [--include-archived]` | Query with optional LLM synthesis |
| `prime <task> [--filter <strategy>] [--primary <id\|slug>]...` | Assemble a task-focused mindset grouped by unit type |
| `stats [--top <n>] [--include-archived]` | Aggregate metrics for the admin dashboard |
| `archive <id>` | Soft-delete a unit (reversible via `unarchive`) |
| `unarchive <id>` | Restore an archived unit |
| `clone <id> [--type] [--source] [--tags]` | Copy a unit into a fresh UUIDv7 |
| `digest` | Classify inbox items via LLM into typed units |
| `mark <id> --kind <kind>` | Record feedback on a unit |
| `delete <id>` | Delete a knowledge unit |
| `slug set\|unset\|list` | Manage human-readable slugs that resolve to unit IDs |
| `emit --target <target> --type <type>` | Emit typed units as build artifacts |
| `rewrite <id> [--suggest]` | Edit a unit in `$EDITOR` with type-aware skeleton or LLM pre-fill |
| `scan [--stale-days <days>]` | Find low-confidence, stale, or orphaned units |
| `backup` | Create a database backup |
| `restore [<filename>]` | Restore from backup (no args = list backups) |

### Global Flags

- `--json` -- Machine-readable JSON output on all commands
- `--debug` -- Trace internal processing (used with `ask`)

## Knowledge Types

| Type | Description |
|------|-------------|
| `fact` | Verified information or data point |
| `procedure` | Step-by-step process or method |
| `principle` | Guiding rule or design philosophy |
| `preference` | Personal choice or configuration |
| `lesson` | Insight learned from experience |
| `idea` | Speculative concept or proposal |
| `aspect` | Facet or dimension of a broader topic |

## Relationships

| Relationship | Description |
|--------------|-------------|
| `related_to` | General association between units |
| `part_of` | Unit is a component of another |
| `depends_on` | Unit requires another to be valid |
| `contradicts` | Units present conflicting information |
| `supersedes` | Unit replaces an older unit |
| `sourced_from` | Unit was derived from another |

## Marks

| Kind | Confidence Delta |
|------|-----------------|
| `used` | +0.05 |
| `helpful` | +0.10 |
| `outdated` | -0.10 |
| `wrong` | -0.20 |

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `SIMARIS_HOME` | Override data directory | `~/.simaris/` |
| `SIMARIS_ENV=dev` | Isolate to dev database | `~/.simaris/dev/sanctuary.db` |
| `SIMARIS_MODEL` | Override LLM model for digest/ask | `sonnet` |

Data lives at `~/.simaris/sanctuary.db`. The `claude` CLI is required for `digest` and `ask --synthesize`.

## Architecture

```
src/main.rs         CLI entry, clap derive command parsing, dispatch
src/db.rs           SQLite schema, migrations, CRUD, backup/restore, scan
src/ask.rs          FTS5 search, graph expansion, relevance filter, LLM synthesis
src/digest.rs       LLM classification of inbox items into typed units
src/display.rs      Text and JSON output formatting
src/emit.rs         Build-artifact emission (claude-code aspects, etc.)
src/rewrite.rs      $EDITOR rewrite flow with type-aware skeletons + LLM pre-fill
src/frontmatter.rs  YAML frontmatter parse/write + refs: graph materialization
src/size_guard.rs   Write-time body-size thresholds + warnings
tests/integration.rs  End-to-end CLI tests via subprocess
simaris-server/     Axum HTTP admin dashboard (separate workspace member)
web/                Static dashboard + units page (vanilla JS + ECharts)
```

### Schema

- **units** -- UUIDv7 primary key, content, type, source, confidence, verified, archived, tags (JSON), timestamps
- **links** -- Composite key (from_id, to_id, relationship), CASCADE delete
- **inbox** -- UUIDv7 primary key, content, source, timestamp
- **marks** -- UUIDv7 primary key, unit_id FK, kind, timestamp
- **slugs** -- TEXT primary key, unit_id FK, CASCADE delete
- **units_fts** -- FTS5 virtual table synced via triggers

Default views (`list`, `search`, `ask`, `prime`, `scan`, `emit`) hide archived units. Pass `--include-archived` to fold them back in.

## Admin Dashboard (simaris-server)

`simaris-server` is an HTTP admin UI for the knowledge store. It binds `0.0.0.0:3535`, mounts a JSON API under `/api`, and serves a static SPA from `web/`. All data and mutations shell out to the `simaris` CLI — no direct SQLite access. See [docs/simaris-server.md](docs/simaris-server.md) for launchd setup on macOS.

```bash
cargo install --path ./simaris-server
simaris-server   # http://localhost:3535
```

## License

[MIT](LICENSE)
