# Simaris CLI Reference

Knowledge unit storage. All commands support `--json` for machine-readable output.

## Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Output as JSON instead of human-readable text. Available on every command. |
| `--debug` | Show debug trace of internal processing. Prints phase-by-phase diagnostics to stderr. Available on every command but only produces output for `ask`. |
| `--include-archived` | Include archived (soft-deleted) units in results. Recognized by `list`, `search`, `ask`, `prime`, and `stats`. |

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SIMARIS_HOME` | Override the data directory path. | `~/.simaris` |
| `SIMARIS_ENV` | Set to `dev` to use `$SIMARIS_HOME/dev` as the data directory. | (unset) |
| `SIMARIS_BIN` | Path to the `simaris` binary used by `simaris-server` to shell out for data ops. | `simaris` (resolved via `PATH`) |
| `SIMARIS_WEB_DIR` | Path to `web/` static assets served by `simaris-server`. | workspace-root `web/` |
| `SIMARIS_MODEL` | LLM model for `ask --synthesize` and `digest`. | `sonnet` |

---

## add

Create a typed knowledge unit.

```
simaris add <CONTENT> --type <TYPE> [--source <SOURCE>] [--tags <TAGS>]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `CONTENT` | string | yes | -- | Content of the unit. |
| `--type` | UnitType | yes | -- | Type of knowledge unit. |
| `--source` | string | no | `inbox` | Source attribution for the unit. |
| `--tags` | string | no | -- | Comma-separated tags (e.g. `"rust,testing,quality"`). |

### UnitType values

`fact`, `procedure`, `principle`, `preference`, `lesson`, `idea`, `aspect`

### Output

```
Added unit 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

### JSON output

```json
{"id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f"}
```

### Example

```
simaris add "Rust edition 2024 uses the new async trait syntax" --type fact --source docs
simaris add "Always run cargo fmt before committing" --type principle
simaris add "Run cargo test before pushing" --type procedure --tags "rust,testing,quality"
```

---

## show

Display a knowledge unit with its metadata, tags, conditions, and all incoming/outgoing links.

```
simaris show <ID>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID (full UUIDv7). |

### Output

```
[019660a3-7b2e-7000-8000-1a2b3c4d5e6f] fact (docs)
Rust edition 2024 uses the new async trait syntax
confidence: 1.0  verified: false
tags: rust, async, traits
created: 2026-04-09T12:00:00Z  updated: 2026-04-09T12:00:00Z

  -> 019660a3-8c3f-7000-8000-2b3c4d5e6f7a (related_to)
  <- 019660a3-9d40-7000-8000-3c4d5e6f7a8b (part_of)
```

### JSON output

```json
{
  "unit": {
    "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
    "type": "fact",
    "content": "Rust edition 2024 uses the new async trait syntax",
    "source": "docs",
    "confidence": 1.0,
    "verified": false,
    "tags": ["rust", "async", "traits"],
    "conditions": {},
    "created": "2026-04-09T12:00:00Z",
    "updated": "2026-04-09T12:00:00Z"
  },
  "links": {
    "outgoing": [
      {"from_id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f", "to_id": "019660a3-8c3f-7000-8000-2b3c4d5e6f7a", "relationship": "related_to"}
    ],
    "incoming": [
      {"from_id": "019660a3-9d40-7000-8000-3c4d5e6f7a8b", "to_id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f", "relationship": "part_of"}
    ]
  }
}
```

---

## link

Create a directed relationship between two knowledge units.

```
simaris link <FROM_ID> <TO_ID> --rel <RELATIONSHIP>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `FROM_ID` | string | yes | Source unit ID. |
| `TO_ID` | string | yes | Target unit ID. |
| `--rel` | Relationship | yes | Relationship type. |

### Relationship values

`related_to`, `part_of`, `depends_on`, `contradicts`, `supersedes`, `sourced_from`

### Output

```
Linked 019660a3-7b2e-... -> 019660a3-8c3f-... (related_to)
```

### JSON output

```json
{
  "from": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
  "to": "019660a3-8c3f-7000-8000-2b3c4d5e6f7a",
  "relationship": "related_to"
}
```

### Example

```
simaris link 019660a3-7b2e-7000-8000-1a2b3c4d5e6f 019660a3-8c3f-7000-8000-2b3c4d5e6f7a --rel depends_on
```

---

## edit

Update one or more fields on an existing knowledge unit. At least one of `--content`, `--type`, `--source`, or `--tags` must be provided. Shows the updated unit after applying changes.

```
simaris edit <ID> [--content <CONTENT>] [--type <TYPE>] [--source <SOURCE>] [--tags <TAGS>]
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID (full UUIDv7). |
| `--content` | string | no | New content for the unit. |
| `--type` | UnitType | no | New type for the unit. |
| `--source` | string | no | New source attribution. |
| `--tags` | string | no | Comma-separated tags (replaces existing tags). |

### Output

Displays the updated unit in the same format as `show`.

### Example

```
simaris edit 019660a3-7b2e-7000-8000-1a2b3c4d5e6f --tags "rust,async,updated"
simaris edit 019660a3-7b2e-7000-8000-1a2b3c4d5e6f --content "Updated content here" --type lesson
```

---

## delete

Delete a knowledge unit by ID. Cascades to linked edges (via `ON DELETE CASCADE`).

```
simaris delete <ID>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID to delete. |

### Output

```
Deleted unit 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

### JSON output

```json
{"deleted": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f"}
```

Errors if the unit does not exist.

### Example

```
simaris delete 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

---

## drop

Capture raw content into the inbox for later triage.

```
simaris drop <CONTENT> [--source <SOURCE>]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `CONTENT` | string | yes | -- | Content to capture. |
| `--source` | string | no | `cli` | Source attribution. |

### Output

```
Dropped item 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

### JSON output

```json
{"id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f"}
```

### Example

```
simaris drop "Investigate whether tokio runtime is needed for async traits" --source research
simaris drop "Simon prefers dark mode in all terminals"
```

---

## promote

Convert an inbox item into a typed knowledge unit.

```
simaris promote <ID> --type <TYPE>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Inbox item ID. |
| `--type` | UnitType | yes | Type for the new unit. See `add` for valid values. |

### Output

```
Added unit 019660a3-8c3f-7000-8000-2b3c4d5e6f7a
```

### JSON output

```json
{"id": "019660a3-8c3f-7000-8000-2b3c4d5e6f7a"}
```

### Example

```
simaris promote 019660a3-7b2e-7000-8000-1a2b3c4d5e6f --type preference
```

---

## inbox

List all pending inbox items.

```
simaris inbox
```

Takes no arguments.

### Output

```
[019660a3] 2026-04-09T12:00:00Z (cli)  Investigate whether tokio runtime is needed...
[019660a4] 2026-04-09T12:05:00Z (research)  New crate for structured logging...
```

Each line shows: `[short_id] created (source) content_preview`. Content is truncated at 80 characters.

### JSON output

```json
[
  {
    "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
    "content": "Investigate whether tokio runtime is needed for async traits",
    "source": "cli",
    "created": "2026-04-09T12:00:00Z"
  }
]
```

If the inbox is empty, prints `Inbox is empty.` (or an empty JSON array with `--json`).

---

## list

List knowledge units with an optional type filter.

```
simaris list [--type <TYPE>]
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `--type` | UnitType | no | Filter results to a specific unit type. |

### Output

```
[019660a3] fact (docs)  Rust edition 2024 uses the new async trait syntax
[019660a4] principle (cli)  Always run cargo fmt before committing...
```

Each line shows: `[short_id] type (source) content_preview`. Content is truncated at 80 characters.

### JSON output

Returns the full `Unit` object array:

```json
[
  {
    "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
    "type": "fact",
    "content": "Rust edition 2024 uses the new async trait syntax",
    "source": "docs",
    "confidence": 1.0,
    "verified": false,
    "tags": ["rust", "async"],
    "conditions": {},
    "created": "2026-04-09T12:00:00Z",
    "updated": "2026-04-09T12:00:00Z"
  }
]
```

If no units match, prints `No units found.` (or an empty JSON array with `--json`).

### Example

```
simaris list
simaris list --type procedure
simaris list --type lesson --json
```

---

## search

Full-text search (FTS5) across knowledge units with an optional type filter.

```
simaris search <QUERY> [--type <TYPE>]
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `QUERY` | string | yes | Search query. Matched against unit content via SQLite FTS5. |
| `--type` | UnitType | no | Filter results to a specific unit type. |

### Output

Same format as `list`. Content is truncated at 80 characters in human-readable mode.

### JSON output

Same format as `list` -- returns full `Unit` object array.

### Example

```
simaris search "async traits"
simaris search "homebrew formula" --type procedure
simaris search "git workflow" --type lesson --json
```

---

## ask

Query the knowledge store with LLM-powered retrieval. Performs FTS5 search, 1-hop graph expansion to pull in linked units, relevance filtering (via Haiku), and optional synthesis (via Sonnet or `SIMARIS_MODEL`).

Requires the `claude` CLI to be installed and available on PATH.

```
simaris ask <QUERY> [--synthesize] [--type <TYPE>] [--debug]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `QUERY` | string | yes | -- | Your question or context. |
| `--synthesize` | flag | no | false | Run LLM synthesis on results. Without this flag, returns matched units only. |
| `--type` | UnitType | no | -- | Filter search results to a specific unit type. |
| `--debug` | flag | no | false | Print phase-by-phase trace to stderr. |

### Pipeline

1. **Phase 1 -- FTS5 Search + Graph Expansion**: Runs a full-text search (up to 15 direct matches), then fetches all 1-hop linked units.
2. **Phase 2 -- Relevance Filter**: Sends unit summaries to Haiku to select only relevant units. Falls back to all units on failure.
3. **Phase 3 -- Synthesis** (only with `--synthesize`): Sends relevant units to Sonnet (or `SIMARIS_MODEL`) to produce a synthesized response.

### Output (without --synthesize)

```
Found 3 relevant unit(s):

[019660a3-7b2e-7000-8000-1a2b3c4d5e6f] fact (tags: rust, async)
  Rust edition 2024 uses the new async trait syntax
  Links: 019660a4-... Overview of Rust 2024 changes (related_to)
```

### Output (with --synthesize)

Prints the synthesized response text directly.

### Debug output (stderr)

```
+-- PHASE 1: FTS5 Search + Graph Expansion
|  query: "async traits"
|  fts_query: "\"async\" OR \"traits\""
|  "async traits" -> 3 matches
|  deduplicated: 3 unique units
|  1-hop expansion: +2 linked units -> 5 total
|
+-- PHASE 2: Relevance Filter (haiku)
|  input: 5 units
|  kept: 3 units
|  fallback: false
|
+-- PHASE 3: Synthesis (sonnet)
   units_used: 3
```

### JSON output

```json
{
  "query": "async traits",
  "units": [
    {
      "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
      "content": "Rust edition 2024 uses the new async trait syntax",
      "unit_type": "fact",
      "tags": ["rust", "async"],
      "source": "docs",
      "is_direct_match": true,
      "links": [
        {
          "unit_id": "019660a4-8c3f-7000-8000-2b3c4d5e6f7a",
          "relationship": "related_to",
          "title": "Overview of Rust 2024 changes"
        }
      ]
    }
  ],
  "units_used": ["019660a3-7b2e-7000-8000-1a2b3c4d5e6f"],
  "response": "Synthesized text here (only present with --synthesize)",
  "debug": {
    "fts_query": "\"async\" OR \"traits\"",
    "matches_per_query": {"async traits": 3},
    "total_gathered": 5,
    "filter_kept": 3,
    "filter_total": 5,
    "filter_fallback": false,
    "units_in_result": 3
  }
}
```

The `response` field is omitted when `--synthesize` is not used. The `debug` field is omitted when `--debug` is not used.

### Example

```
simaris ask "how do I release a homebrew formula"
simaris ask "git workflow conventions" --synthesize
simaris ask "testing patterns" --type procedure --json
simaris ask "deployment steps" --synthesize --debug
```

---

## prime

Assemble a task-focused "mindset" from the knowledge graph. Searches for units relevant to the task, filters them via the chosen strategy, and groups results by type into ordered sections (`Aspects`, `Procedures`, `Principles`, `Preferences`, `Context`).

Intended for LLM agents that need to load relevant context at the start of a task.

```
simaris prime <TASK> [--filter <FILTER>]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `TASK` | string | yes | -- | Task description used as the retrieval query. |
| `--filter` | FilterStrategy | no | `standard` | Strategy for narrowing the gathered units. |

### FilterStrategy values

| Strategy | Description | Requires `claude` CLI |
|----------|-------------|-----------------------|
| `none` | Return all gathered units without filtering. | no |
| `standard` | LLM-backed relevance filter via Haiku. Falls back to unfiltered on error. | yes |
| `tag-vote` | Rank units by tag overlap with task keywords; keep the top-scoring set. | no |

### Output

Sections are printed in order, each prefixed with `# <Section>`. Units in a section are separated by blank lines. If no relevant knowledge is found:

```
No relevant knowledge found for: <task>
```

Example:

```
# Aspects

Code review is a rigorous, multi-phase process...

# Procedures

Always run cargo fmt before committing.

# Context

Simaris stores typed knowledge units in SQLite.
```

### JSON output

```json
{
  "task": "review this PR",
  "sections": [
    {
      "label": "Aspects",
      "units": [
        {
          "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
          "content": "Code review is a rigorous...",
          "tags": ["code-review", "aspect"]
        }
      ]
    }
  ],
  "unit_count": 1
}
```

### Example

```
simaris prime "implement a new CLI command"
simaris prime "debug a flaky test" --filter tag-vote
simaris prime "review this PR" --filter none --json
```

---

## digest

Process all pending inbox items through LLM classification. Each inbox item is broken into discrete typed knowledge units (3-8 per item) and promoted into the store. The first unit extracted from each item is an overview summary.

Requires the `claude` CLI to be installed and available on PATH. Uses the model specified by `SIMARIS_MODEL` (default: `sonnet`).

```
simaris digest
```

Takes no arguments.

### Output

```
Processing 2 inbox item(s)...

[019660a3-7b2e-...] Investigate whether tokio runtime is needed...
  * -> unit 019660a5-... (fact) [rust, tokio, async]
    -> unit 019660a6-... (procedure) [rust, async, migration]

[019660a4-8c3f-...] New crate for structured logging...
    -> unit 019660a7-... (fact) [logging, tracing]

Digested: 2 items -> 3 units, Skipped: 0
```

Lines marked with `*` are overview units. If the inbox is empty, prints `Inbox is empty. Nothing to digest.`

### JSON output

The `digest` command does not currently support `--json` output. Progress is printed to stdout.

---

## mark

Record a feedback signal on a knowledge unit. Adjusts the unit's confidence score.

```
simaris mark <ID> --kind <KIND>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID to mark. |
| `--kind` | MarkKind | yes | Kind of feedback signal. |

### MarkKind values

| Kind | Confidence delta |
|------|-----------------|
| `used` | +0.05 |
| `helpful` | +0.10 |
| `outdated` | -0.10 |
| `wrong` | -0.20 |

Confidence is clamped to the range `[0.0, 1.0]`.

### Output

```
Marked unit 019660a3-7b2e-7000-8000-1a2b3c4d5e6f as wrong (confidence: 0.80)
```

### JSON output

```json
{
  "id": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f",
  "mark": "wrong",
  "confidence": 0.8
}
```

### Example

```
simaris mark 019660a3-7b2e-7000-8000-1a2b3c4d5e6f --kind used
simaris mark 019660a3-7b2e-7000-8000-1a2b3c4d5e6f --kind wrong
```

---

## scan

Run a health check on the knowledge store. Reports issues across five categories.

```
simaris scan [--stale-days <DAYS>]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `--stale-days` | u32 | no | `90` | Number of days without marks before a unit is considered stale. |

### Categories reported

| Category | Description |
|----------|-------------|
| Low confidence | Units with confidence below 0.6. |
| Negative marks | Units that have received `wrong` or `outdated` marks. |
| Contradictions | Pairs of units linked with the `contradicts` relationship. |
| Orphans | Units with no incoming or outgoing links. |
| Stale | Units with no marks within the `--stale-days` window. |

### Output

```
Low confidence:
  [019660a3] (0.40) Some dubious claim about Rust lifetimes...

Negative marks:
  [019660a4] Old deployment procedure that no longer works...

Contradictions:
  [019660a5] Use tokio for all async <-> [019660a6] Avoid tokio in CLI tools...

Orphans:
  [019660a7] Standalone fact with no links...

Stale:
  [019660a8] (2025-01-15T10:00:00Z) Ancient procedure nobody references...
```

If no issues are found in any category, prints `No issues found.`

### JSON output

```json
{
  "low_confidence": [{"id": "...", "type": "...", "content": "...", ...}],
  "negative_marks": [{"id": "...", "type": "...", "content": "...", ...}],
  "contradictions": [
    {
      "from_id": "...",
      "from_content": "...",
      "to_id": "...",
      "to_content": "..."
    }
  ],
  "orphans": [{"id": "...", "type": "...", "content": "...", ...}],
  "stale": [{"id": "...", "type": "...", "content": "...", ...}]
}
```

Each unit object in the arrays has the full `Unit` schema (id, type, content, source, confidence, verified, tags, conditions, created, updated).

### Example

```
simaris scan
simaris scan --stale-days 30
simaris scan --json
```

---

## backup

Create a timestamped backup of the knowledge store database.

```
simaris backup
```

Takes no arguments. The backup file is saved to `$SIMARIS_HOME/backups/` (default `~/.simaris/backups/`).

### Output

```
Backup created: /Users/you/.simaris/backups/sanctuary-20260409-120000.db
```

### JSON output

```json
{"path": "/Users/you/.simaris/backups/sanctuary-20260409-120000.db"}
```

---

## restore

Restore the knowledge store from a backup, or list available backups.

```
simaris restore [FILENAME]
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `FILENAME` | string | no | Backup filename to restore. If omitted, lists available backups. |

### Output (list mode)

```
sanctuary-20260409-120000.db
sanctuary-20260408-090000.db
```

If no backups exist, prints `No backups found.`

### Output (restore mode)

```
Restored from: sanctuary-20260409-120000.db
```

### JSON output (list mode)

```json
["sanctuary-20260409-120000.db", "sanctuary-20260408-090000.db"]
```

### JSON output (restore mode)

```json
{"restored": "sanctuary-20260409-120000.db"}
```

### Example

```
simaris restore
simaris restore sanctuary-20260409-120000.db
```

---
## stats

Aggregate metrics for the admin dashboard, computed in a single SQL pass.

```
simaris stats [--top <N>] [--include-archived]
```

### Arguments

| Argument | Type | Required | Default | Description |
|----------|------|----------|---------|-------------|
| `--top` | usize | no | `50` | Cap the per-tag breakdown at this many tags (most frequent first). `total_unique` still reports the full distinct-tag count. |
| `--include-archived` | flag | no | -- | Include archived units in `total`, `by_type`, `by_tag`, `confidence`, and `superseded_count`. `inbox_size`, `marks`, and `archived_count` are global regardless. |

### JSON output

```json
{
  "total": 1234,
  "by_type": { "fact": 412, "procedure": 198, "...": 0 },
  "by_tag": { "top": [{ "tag": "rust", "count": 87 }], "total_unique": 304 },
  "confidence": { "low": 12, "medium": 88, "high": 410, "verified": 724 },
  "inbox_size": 4,
  "marks": { "used": 502, "helpful": 188, "outdated": 9, "wrong": 2 },
  "superseded_count": 17,
  "archived_count": 31,
  "include_archived": false
}
```

`confidence` buckets: `low` (<0.6), `medium` (0.6-<0.8), `high` (0.8-<0.95), `verified` (≥0.95).

### Example

```
simaris stats --json
simaris stats --top 10 --include-archived
```

---

## archive

Soft-delete a unit. Reversible via `unarchive`. Preserves the row in `units`, all incoming/outgoing links, and the FTS index — `unarchive` cleanly restores the unit to every default surface.

Archived units are hidden from `list`, `search`, `ask`, `prime`, `scan`, and `emit` unless `--include-archived` is passed.

```
simaris archive <ID>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID or slug. |

### Output

```
Archived 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

### JSON output

```json
{ "archived": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f" }
```

### Example

```
simaris archive 019660a3
simaris archive my-stale-procedure --json
```

---

## unarchive

Reverse of `archive`. Restores a soft-deleted unit to all default views.

```
simaris unarchive <ID>
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Unit ID or slug. |

### Output

```
Unarchived 019660a3-7b2e-7000-8000-1a2b3c4d5e6f
```

### JSON output

```json
{ "unarchived": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f" }
```

---

## clone

Copy a unit into a fresh UUIDv7. Content, type, source, and tags are copied from the source unit; confidence resets to 1.0 and `verified` resets to false (a clone is unverified by definition). Links and marks are NOT copied. Auto-link runs against the new unit (`related_to` on 2+ tag overlap), matching `add`. Frontmatter `refs:` are re-materialized as `related_to` edges from the new unit.

```
simaris clone <ID> [--type <TYPE>] [--source <SOURCE>] [--tags <TAGS>]
```

### Arguments

| Argument | Type | Required | Description |
|----------|------|----------|-------------|
| `ID` | string | yes | Source unit ID or slug. |
| `--type` | UnitType | no | Override the cloned unit's type (default: same as source). |
| `--source` | string | no | Override the cloned unit's source (default: same as source). |
| `--tags` | string | no | Override the cloned unit's tags, comma-separated. Empty string clears tags. Default: same as source. |

### Output

```
Cloned 019660a3-7b2e-7000-8000-1a2b3c4d5e6f -> 0196b021-9c4f-7000-8000-aa11bb22cc33
  auto-linked to 3 existing unit(s)
```

### JSON output

```json
{ "id": "0196b021-9c4f-7000-8000-aa11bb22cc33", "from": "019660a3-7b2e-7000-8000-1a2b3c4d5e6f" }
```

### Example

```
simaris clone 019660a3
simaris clone canonical-procedure --tags "rust,toolchain,2024"
simaris clone 019660a3 --type principle --source design-doc
```

---


## Data Types Reference

### Unit

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | UUIDv7 identifier. |
| `type` | string | One of: `fact`, `procedure`, `principle`, `preference`, `lesson`, `idea`, `aspect`. |
| `content` | string | Unit content. |
| `source` | string | Source attribution. |
| `confidence` | float | Confidence score, starts at 1.0. Adjusted by `mark`. |
| `verified` | bool | Whether the unit has been verified. |
| `tags` | string[] | Associated tags. |
| `conditions` | object | Conditions under which the unit applies. |
| `archived` | bool | Whether the unit is soft-deleted. Default views (`list`, `search`, `ask`, `prime`, `scan`, `emit`) hide archived units; pass `--include-archived` to fold them in. Reverse via `unarchive`. |
| `created` | string | ISO 8601 creation timestamp. |
| `updated` | string | ISO 8601 last-updated timestamp. |

### InboxItem

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | UUIDv7 identifier. |
| `content` | string | Raw captured content. |
| `source` | string | Source attribution. |
| `created` | string | ISO 8601 creation timestamp. |

### Link

| Field | Type | Description |
|-------|------|-------------|
| `from_id` | string | Source unit ID. |
| `to_id` | string | Target unit ID. |
| `relationship` | string | One of: `related_to`, `part_of`, `depends_on`, `contradicts`, `supersedes`, `sourced_from`. |
