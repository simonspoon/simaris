# Contributing to Simaris

## Prerequisites

- **Rust** (edition 2024) -- install via [rustup](https://rustup.rs/)
- **`claude` CLI** -- required for LLM features (`digest`, `ask --synthesize`)

## Build

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

## Test

```bash
# Full test suite (unit + integration)
cargo test

# Single test by name
cargo test test_add_command

# With stdout/stderr output visible
cargo test -- --nocapture

# Single test with output
cargo test test_add_command -- --nocapture
```

## Test Architecture

Tests live in two places:

- **Unit tests** -- `src/db.rs` has a `#[cfg(test)] mod tests` block with in-memory SQLite tests. These test database functions directly without spawning a process.
- **Integration tests** -- `tests/integration.rs` runs the compiled binary as a subprocess, testing the full CLI end-to-end.

### TestEnv

Integration tests use a `TestEnv` struct that provides process isolation:

```rust
struct TestEnv {
    dir: std::path::PathBuf,
}
```

**How it works:**

1. `TestEnv::new("name")` creates a unique temp directory at `$TMPDIR/simaris-test-{name}-{pid}`
2. `env.run(&["add", "content", "--type", "fact"])` spawns the `simaris` binary with `SIMARIS_HOME` set to that temp directory. The database is created inside it, fully isolated from the real store.
3. `env.run_ok(...)` does the same but asserts the command succeeded and returns stdout as a `String`.
4. When `TestEnv` is dropped, it deletes the temp directory via its `Drop` impl.

### DB unit tests

The `db::tests` module uses an in-memory SQLite connection:

```rust
fn memory_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    initialize(&conn).unwrap();
    conn
}
```

These tests call `db::` functions directly (e.g. `add_unit`, `search_units`, `scan`) without going through the CLI.

### Seeding test data

Both test layers seed data by calling simaris commands or db functions inline. There are no fixture files. Some tests manipulate SQLite directly (e.g. backdating timestamps for stale-detection tests):

```rust
conn.execute(
    "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
    rusqlite::params![id],
).unwrap();
```

## Dev Environment

### SIMARIS_ENV

Set `SIMARIS_ENV=dev` to isolate development data from the production store. When set, the database is created at `$SIMARIS_HOME/dev/sanctuary.db` instead of `$SIMARIS_HOME/sanctuary.db`.

```bash
export SIMARIS_ENV=dev
simaris add "test content" --type fact
# DB created at ~/.simaris/dev/sanctuary.db (or $SIMARIS_HOME/dev/sanctuary.db)
```

### SIMARIS_HOME

Override the base data directory. Defaults to `~/.simaris`. Tests set this to a temp directory for full isolation.

```bash
SIMARIS_HOME=/tmp/simaris-scratch simaris list
```

## Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `anyhow` | 1 | Error handling with context |
| `clap` | 4 (derive) | CLI argument parsing with derive macros |
| `dirs` | 6 | Platform-specific home directory resolution |
| `rusqlite` | 0.32 (bundled) | SQLite database access; `bundled` compiles SQLite from source |
| `serde` | 1 (derive) | Serialization/deserialization for data structs |
| `serde_json` | 1 | JSON output (used in both main and dev-dependencies) |
| `uuid` | 1 (v7) | UUIDv7 generation for unit/item IDs (time-sortable) |

## Adding a New Command

### 1. Add the variant to the `Command` enum in `src/main.rs`

```rust
#[derive(Subcommand)]
enum Command {
    // ... existing variants ...

    /// Description of your command
    NewCommand {
        /// A required argument
        arg: String,

        /// An optional flag
        #[arg(long)]
        flag: Option<String>,
    },
}
```

### 2. Add the handler in the `match cli.command` block

In `main()`, add an arm to the match:

```rust
Command::NewCommand { arg, flag } => {
    // Call into db:: or other modules
    let result = db::some_function(&conn, &arg)?;
    // Use display:: for output (handles --json flag)
    display::print_something(&result, cli.json);
}
```

### 3. Add display functions if needed

Add both human-readable and JSON output paths in `src/display.rs`. Follow the existing pattern -- check the `json` flag and branch accordingly.

### 4. Add an integration test

Add a test in `tests/integration.rs` using `TestEnv`:

```rust
#[test]
fn test_new_command() {
    let env = TestEnv::new("newcommand");
    // Seed any prerequisite data
    env.run_ok(&["add", "some data", "--type", "fact"]);
    // Run your command
    let out = env.run_ok(&["new-command", "arg-value"]);
    // Assert on output
    assert!(out.contains("expected output"), "got: {out}");
}
```

## Source Layout

```
src/
  main.rs    -- CLI definition (Cli, Command, enums) and dispatch
  db.rs      -- SQLite schema, all database operations, unit tests
  display.rs -- Output formatting (human-readable + JSON)
  ask.rs     -- Ask command: query expansion, search, LLM synthesis
  digest.rs  -- Digest command: LLM-based inbox classification
tests/
  integration.rs -- End-to-end CLI tests via TestEnv
```
