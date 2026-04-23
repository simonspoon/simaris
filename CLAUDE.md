# simaris

Rust CLI knowledge management system. Stores typed knowledge units in SQLite with FTS5 search, graph-based linking, confidence scoring via marks, and LLM-powered digest/synthesis via Claude CLI.

## Build and Run

| Command | Purpose |
|---------|---------|
| `cargo build` | Debug build |
| `cargo build --release` | Release build |
| `cargo test` | Run all tests |
| `cargo test test_name` | Run a single test |
| `cargo install --path .` | Install binary |

Binary: `./target/release/simaris`

## Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `SIMARIS_HOME` | Override data directory | `~/.simaris/` |
| `SIMARIS_ENV=dev` | Isolate to dev database | `~/.simaris/dev/sanctuary.db` |
| `SIMARIS_MODEL` | Override LLM model for digest/ask | `sonnet` |
| `SIMARIS_WARN_BYTES` | Warn threshold — body above this triggers a stderr warning citing `split-ruleset` at `add`/`edit` time | `2048` (placeholder; Story 4 calibrates) |
| `SIMARIS_HARD_BYTES` | Hard threshold — body above this rejects the write (exit non-zero) unless `--force` or tag `flow`/`--flow` | `8192` (placeholder; Story 4 calibrates) |

Data lives at `~/.simaris/sanctuary.db`. Backups go to `~/.simaris/backups/`.

## External Dependencies

- `claude` CLI required for `digest` and `ask --synthesize` commands
- SQLite is bundled via rusqlite (no system SQLite needed)

## Source Layout

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | 444 | CLI entry, clap derive command parsing, dispatch |
| `src/db.rs` | 1528 | SQLite schema, migrations, CRUD, backup/restore, scan |
| `src/ask.rs` | 500 | FTS5 search, graph expansion, relevance filter, LLM synthesis |
| `src/digest.rs` | 120 | LLM classification of inbox items into typed units |
| `src/display.rs` | 275 | Text and JSON output formatting |
| `src/size_guard.rs` | ~100 | Write-time body-size thresholds + warnings (`add`/`edit`) |
| `tests/integration.rs` | 1800+ | End-to-end CLI tests via subprocess |

## Architecture

### Knowledge Types

`fact`, `procedure`, `principle`, `preference`, `lesson`, `idea`, `aspect`

### Relationship Types (links)

`related_to`, `part_of`, `depends_on`, `contradicts`, `supersedes`, `sourced_from`

### Mark Kinds (confidence feedback)

| Kind | Delta |
|------|-------|
| `used` | +0.05 |
| `helpful` | +0.10 |
| `outdated` | -0.10 |
| `wrong` | -0.20 |

### Schema (5 tables)

- `units` -- TEXT primary key (UUIDv7), content, type, source, confidence, verified, tags (JSON), conditions (JSON), timestamps
- `links` -- composite PK (from_id, to_id, relationship), foreign keys to units with CASCADE delete
- `inbox` -- TEXT primary key (UUIDv7), content, source, timestamp
- `marks` -- TEXT primary key (UUIDv7), unit_id FK, kind, timestamp
- `slugs` -- TEXT primary key (slug), unit_id FK to units with CASCADE delete, created timestamp; indexed on unit_id for reverse lookup
- `units_fts` -- FTS5 virtual table synced via triggers (uuid, content, type, tags, source)

### Data Flow

1. Raw input enters via `drop` -> inbox table
2. `digest` classifies inbox items via LLM -> typed units (with overview unit first)
3. `add` creates typed units directly (bypasses inbox)
4. `promote` converts an inbox item to a typed unit
5. `link` creates graph edges between units
5b. `add` and `digest` auto-link new units to existing units sharing 2+ tags via `related_to`
6. `mark` records feedback, adjusts unit confidence
7. `edit` updates content, type, source, or tags on existing units
8. `ask` searches FTS5, expands via graph links, optionally synthesizes via LLM
9. `scan` finds low-confidence, stale, orphaned, or contradicted units

## Conventions

- Rust edition 2024
- UUIDv7 for all entity IDs (units, inbox items, marks, links use composite key)
- All commands support `--json` for machine-readable output
- `--debug` flag traces internal processing (used in `ask`)
- Error handling via `anyhow::Result` throughout
- Tests use `TestEnv` struct that creates an isolated `SIMARIS_HOME` in temp dir, cleaned up on drop
- Integration tests invoke the compiled binary as a subprocess

## Commands

```
simaris add <content> --type <type> [--source <source>] [--tags <comma-separated>]
simaris show <id>
simaris edit <id> [--content <content>] [--type <type>] [--source <source>] [--tags <comma-separated>]
simaris link <from_id> <to_id> --rel <relationship>
simaris drop <content> [--source <source>]
simaris promote <id> --type <type>
simaris inbox
simaris list [--type <type>]
simaris search <query> [--type <type>]
simaris ask <query> [--synthesize] [--type <type>]
simaris digest
simaris delete <id>
simaris mark <id> --kind <kind>
simaris slug set <slug> <id>
simaris slug unset <slug>
simaris slug list
simaris emit --target <target> --type <type>
simaris scan [--stale-days <days>]
simaris backup
simaris restore [<filename>]
```

Global flags: `--json`, `--debug`
