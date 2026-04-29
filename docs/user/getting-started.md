# Simaris -- Getting Started

A personal knowledge store. Capture raw thoughts, classify them into typed units, link them together, and query across everything you know.

## Installation

Build and install from source:

```
cargo install --path .
```

Or build without installing:

```
cargo build --release
# binary at target/release/simaris
```

Requires Rust 2024 edition (1.85+).

## First Run

Simaris auto-creates its data directory and database on the first command. There is no init step. Run any command and `~/.simaris/sanctuary.db` appears automatically.

```
simaris inbox
```

```
Inbox is empty.
```

You are ready to go.

## Core Workflow

### 1. Capture a raw thought

Use `drop` to throw something into the inbox without classifying it. Good for quick capture when you do not want to stop and categorize.

```
simaris drop "Rust's borrow checker prevents data races at compile time"
```

```
Dropped item 019682a3-b1c4-7def-8a00-1e2f3a4b5c6d
```

The `--source` flag tags where the knowledge came from (defaults to `cli`):

```
simaris drop "TOML supports inline tables" --source "rust-book"
```

### 2. Review the inbox

```
simaris inbox
```

```
[019682a3] 2026-04-09 14:32:01 (cli)  Rust's borrow checker prevents data races at compile time
[019682a4] 2026-04-09 14:33:15 (rust-book)  TOML supports inline tables
```

Each line shows a short ID prefix, timestamp, source, and a content preview (truncated at 80 characters).

### 3. Promote inbox items to typed units

Once you decide what type of knowledge something is, promote it:

```
simaris promote 019682a3-b1c4-7def-8a00-1e2f3a4b5c6d --type fact
```

```
Added unit 019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f
```

The inbox item is consumed and a new knowledge unit is created with the given type.

Available types: `fact`, `procedure`, `principle`, `preference`, `lesson`, `idea`.

### 4. Add knowledge directly

Skip the inbox entirely when you already know the type:

```
simaris add "Always run cargo fmt before committing Rust code" --type procedure
```

```
Added unit 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f
```

The `--source` flag defaults to `inbox` but accepts any string:

```
simaris add "Premature optimization is the root of all evil" --type principle --source "knuth"
```

Add tags at creation time with `--tags` (comma-separated):

```
simaris add "Run cargo test before pushing" --type procedure --tags "rust,testing"
```

### 5. Link related units

Create typed relationships between units:

```
simaris link 019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f --rel related_to
```

```
Linked 019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f -> 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f (related_to)
```

Relationship types: `related_to`, `part_of`, `depends_on`, `contradicts`, `supersedes`, `sourced_from`.

### 6. Browse and search

List all units, optionally filtered by type:

```
simaris list
simaris list --type procedure
```

Search by keyword:

```
simaris search "borrow checker"
```

```
[019682b7] fact (cli)  Rust's borrow checker prevents data races at compile time
```

Search can also filter by type:

```
simaris search "rust" --type fact
```

### 7. View a unit in full

```
simaris show 019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f
```

```
[019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f] fact (cli)
Rust's borrow checker prevents data races at compile time
confidence: 1.0  verified: false
created: 2026-04-09 14:32:01  updated: 2026-04-09 14:35:00

  -> 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f (related_to)
```

### 8. Edit existing units

Update content, type, source, or tags on any unit:

```
simaris edit 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f --tags "rust,formatting,commit"
simaris edit 019682c1-a2b3-7cde-8f00-3a4b5c6d7e8f --content "Run cargo fmt && cargo test before committing"
```

## Querying the Store

### Ask -- FTS5 search + 1-hop graph expansion

`ask` returns matching units from the store. It runs a full-text search and pulls in 1-hop linked units so related context surfaces alongside direct hits.

```
simaris ask "How does Rust handle memory safety?"
```

```
Found 1 relevant unit(s):

[019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f] fact
  Rust's borrow checker prevents data races at compile time
```

Filter results by type:

```
simaris ask "coding standards" --type procedure
```

## Health Maintenance

### Scan for issues

`scan` checks the knowledge store for problems -- low-confidence units, negative marks, contradictions, orphaned units (no links), and stale units with no activity:

```
simaris scan
```

```
Low confidence:
  [019682e1] (0.50) Some dubious claim that was marked wrong once

Orphans:
  [019682f1] A fact that has no links to anything else

Stale:
  [019682b7] (2026-01-05) Rust's borrow checker prevents data races at compile time
```

Adjust the staleness threshold (default 90 days):

```
simaris scan --stale-days 30
```

### Mark units with feedback

Record feedback on a unit to adjust its confidence score:

```
simaris mark 019682e1-a2b3-7cde-8f00-1a2b3c4d5e6f --kind wrong
```

```
Marked unit 019682e1-a2b3-7cde-8f00-1a2b3c4d5e6f as wrong (confidence: 0.80)
```

Mark kinds and their effect on confidence:

| Kind | Confidence delta |
|------|-----------------|
| `used` | +0.05 |
| `helpful` | +0.10 |
| `outdated` | -0.10 |
| `wrong` | -0.20 |

### Backup and restore

Create a backup:

```
simaris backup
```

```
Backup created: /Users/you/.simaris/backups/sanctuary-2026-04-09T143500.db
```

List available backups:

```
simaris restore
```

Restore from a specific backup:

```
simaris restore sanctuary-2026-04-09T143500.db
```

```
Restored from: sanctuary-2026-04-09T143500.db
```

## JSON Output

Every command supports `--json` for machine-readable output:

```
simaris search "rust" --json
```

```json
[
  {
    "id": "019682b7-e5f8-7abc-9d00-2a3b4c5d6e7f",
    "content": "Rust's borrow checker prevents data races at compile time",
    "type": "fact",
    "source": "cli",
    "confidence": 1.0,
    "verified": false,
    "tags": [],
    "conditions": {},
    "created": "2026-04-09 14:32:01",
    "updated": "2026-04-09 14:35:00"
  }
]
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `SIMARIS_HOME` | Override the data directory | `~/.simaris` |
| `SIMARIS_ENV` | Set to `dev` to use a separate `dev/` subdirectory within the data dir | (unset -- uses production) |

Example -- run against a dev database:

```
SIMARIS_ENV=dev simaris drop "testing a new workflow"
```

This writes to `~/.simaris/dev/sanctuary.db` instead of `~/.simaris/sanctuary.db`.
