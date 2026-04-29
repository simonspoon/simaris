# simaris

Rust CLI knowledge management system. Stores typed knowledge units in SQLite with FTS5 search, graph-based linking, confidence scoring via marks, and LLM-powered digest/synthesis via Claude CLI. Ships with `simaris-server`, an HTTP admin dashboard.

## Build and Run

| Command | Purpose |
|---------|---------|
| `cargo build` | Debug build (workspace) |
| `cargo build --release` | Release build (workspace) |
| `cargo test` | Run all tests |
| `cargo test test_name` | Run a single test |
| `cargo install --path .` | Install `simaris` CLI |
| `cargo install --path ./simaris-server` | Install `simaris-server` admin dashboard |

Binaries: `./target/release/simaris`, `./target/release/simaris-server`

## Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `SIMARIS_HOME` | Override data directory | `~/.simaris/` |
| `SIMARIS_ENV=dev` | Isolate to dev database | `~/.simaris/dev/sanctuary.db` |
| `SIMARIS_MODEL` | Override LLM model for digest/ask | `sonnet` |
| `SIMARIS_WARN_BYTES` | Warn threshold — body above this triggers a stderr warning citing `split-ruleset` at `add`/`edit` time | `2048` (placeholder; Story 4 calibrates) |
| `SIMARIS_HARD_BYTES` | Hard threshold — body above this rejects the write (exit non-zero) unless `--force` or tag `flow`/`--flow` | `8192` (placeholder; Story 4 calibrates) |
| `SIMARIS_BIN` | Path to `simaris` binary used by `simaris-server` | `simaris` (resolved via `PATH`) |
| `SIMARIS_WEB_DIR` | Path to `web/` assets served by `simaris-server` | workspace-root `web/` |

Data lives at `~/.simaris/sanctuary.db`. Backups go to `~/.simaris/backups/`.

## External Dependencies

- `claude` CLI required for `digest` and `ask --synthesize` commands
- SQLite is bundled via rusqlite (no system SQLite needed)

## Source Layout

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | 1363 | CLI entry, clap derive command parsing, dispatch |
| `src/db.rs` | 3353 | SQLite schema, migrations, CRUD, backup/restore, scan |
| `src/ask.rs` | 770 | FTS5 search, graph expansion, relevance filter, LLM synthesis |
| `src/digest.rs` | 159 | LLM classification of inbox items into typed units |
| `src/display.rs` | 761 | Text and JSON output formatting |
| `src/emit.rs` | ~250 | Build-artifact emission (claude-code aspects, etc.) |
| `src/rewrite.rs` | ~400 | `$EDITOR` rewrite flow with type-aware skeletons + LLM pre-fill |
| `src/frontmatter.rs` | ~600 | YAML frontmatter parse/write + `refs:` graph materialization |
| `src/size_guard.rs` | 143 | Write-time body-size thresholds + warnings (`add`/`edit`) |
| `tests/integration.rs` | 4000+ | End-to-end CLI tests via subprocess |
| `simaris-server/src/main.rs` | 96 | Axum HTTP entry, route mount, static `web/` serve |
| `simaris-server/src/cli.rs` | 56 | Shells out to `simaris` CLI for all data ops |
| `simaris-server/src/routes/` | ~320 | `/api/stats`, `/api/search`, `/api/units/:id` (get/edit/clone/archive/unarchive) |
| `web/` | -- | Static dashboard + units page (vanilla JS + ECharts) |

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

- `units` -- TEXT primary key (UUIDv7), content, type, source, confidence, verified, archived (soft-delete flag), tags (JSON), conditions (JSON), timestamps
- `links` -- composite PK (from_id, to_id, relationship), foreign keys to units with CASCADE delete
- `inbox` -- TEXT primary key (UUIDv7), content, source, timestamp
- `marks` -- TEXT primary key (UUIDv7), unit_id FK, kind, timestamp
- `slugs` -- TEXT primary key (slug), unit_id FK to units with CASCADE delete, created timestamp; indexed on unit_id for reverse lookup
- `units_fts` -- FTS5 virtual table synced via triggers (uuid, content, type, tags, source)

Default views (`list`, `search`, `ask`, `prime`, `scan`, `emit`) hide archived units. Pass `--include-archived` to fold them back in.

### Data Flow

1. Raw input enters via `drop` -> inbox table
2. `digest` classifies inbox items via LLM -> typed units (with overview unit first)
3. `add` creates typed units directly (bypasses inbox)
4. `promote` converts an inbox item to a typed unit
5. `link` creates graph edges between units
5b. `add`, `digest`, and `clone` auto-link new units to existing units sharing 2+ tags via `related_to`
6. `mark` records feedback, adjusts unit confidence
7. `edit` updates content, type, source, or tags on existing units
8. `archive` / `unarchive` soft-delete and restore units (reversible; preserves links + FTS rows)
9. `clone` copies a unit (content/type/source/tags) into a fresh UUIDv7 — confidence + verified reset; links + marks not copied
10. `ask` searches FTS5, expands via graph links, optionally synthesizes via LLM
11. `scan` finds low-confidence, stale, orphaned, or contradicted units
12. `stats` aggregates dashboard metrics in a single SQL pass (totals, by-type, by-tag, confidence histogram, marks)

## Conventions

- Rust edition 2024, workspace resolver 3
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
simaris list [--type <type>] [--include-archived]
simaris search <query> [--type <type>] [--include-archived]
simaris ask <query> [--synthesize] [--type <type>] [--include-archived]
simaris prime <task> [--filter <strategy>] [--primary <id|slug>]... [--include-archived]
simaris stats [--top <n>] [--include-archived]
simaris archive <id>
simaris unarchive <id>
simaris clone <id> [--type <type>] [--source <source>] [--tags <tags>]
simaris digest
simaris delete <id>
simaris mark <id> --kind <kind>
simaris slug set <slug> <id>
simaris slug unset <slug>
simaris slug list
simaris emit --target <target> --type <type>
simaris scan [--stale-days <days>]
simaris rewrite <id> [--suggest]
simaris backup
simaris restore [<filename>]
```

Global flags: `--json`, `--debug`

## simaris-server (admin dashboard)

HTTP admin UI for the knowledge store. Binds `0.0.0.0:3535`. JSON API under `/api`, static files from `web/`. All data and mutations shell out to the `simaris` CLI — no direct SQLite access. See [docs/simaris-server.md](docs/simaris-server.md) for launchd setup.
