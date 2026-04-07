## Approach
Add simaris digest command. Shell out to claude CLI (claude -p --model haiku) with a structured prompt for each inbox item. Prompt asks haiku to return JSON: {type, tags[], content (refined)}. Parse response, create unit via new add_unit_full(conn, content, type, source, tags) function, delete inbox item. No new crate deps — just std::process::Command. Add src/digest.rs module for LLM interaction. Process items sequentially. Report results. SIMARIS_MODEL env var for model override (default haiku).

## Verify
Drop a skill file content into inbox. Run simaris digest. Verify: inbox is empty after, new units created with appropriate types and tags. Run with no inbox items — exits cleanly. Run without ANTHROPIC_API_KEY — clear error message.

## Result
Inbox items are automatically processed by LLM into typed, tagged knowledge units. The brain can organize incoming raw information.

## AcceptanceCriteria
1. simaris digest with inbox items creates typed/tagged units. 2. Inbox empty after digest. 3. Empty inbox — exits cleanly with message. 4. Claude CLI not found — clear error. 5. Malformed LLM response — skip item, report error, leave in inbox. 6. Tags populated on created units. 7. All existing 43 tests pass.

## ScopeOut
No batching, no --dry-run, no confidence scoring from LLM, no splitting items into multiple units (v1)

## AffectedAreas
src/digest.rs (new module), src/db.rs (add_unit_full + delete_inbox_item), src/main.rs (Digest subcommand), src/display.rs (digest output), tests/integration.rs

## TestStrategy
Unit tests in db.rs: test_add_unit_full (verify tags stored). Integration tests: test_digest_empty_inbox (clean exit), test_digest_no_claude (mock missing binary, clear error). Live test (manual): drop skill file content, run digest, inspect results. Note: LLM output is non-deterministic so integration tests mock the claude call or test structural aspects only. Verify: cargo fmt --check && cargo clippy -- -D warnings && cargo test.

## Risks
Medium: LLM output parsing — haiku may not always return valid JSON. Mitigate: robust parsing with fallback, skip-and-report on failure. Low: claude CLI availability — check with which/Command before processing.

## Report
TL built digest. New digest.rs module (classify via claude CLI, JSON parsing with fence stripping, type validation). db.rs: add_unit_full with tags, delete_inbox_item. main.rs: Digest subcommand with progress output. 47/47 tests pass.

## Notes
### 2026-04-07T16:48:18-04:00
Revised: Use claude CLI (claude -p --model haiku) instead of Anthropic API. Zero new deps. SIMARIS_MODEL env for override.

### 2026-04-07T16:52:29-04:00
REVIEW FINDINGS:

### 0001-01-01T00:00:00Z
- [src/digest.rs:62-70] Code fence stripping uses trim_start/end_matches incorrectly. These methods strip individual *characters* from the set, not the literal substring. trim_start_matches("'```json") strips any leading char that is one of: `, j, s, o, n — not the literal prefix. If the JSON body starts with one of those chars (e.g., a nested string value starting with 'j' or 's'), leading bytes of the actual JSON are silently consumed. Use strip_prefix/strip_suffix instead:
  let json_str = response.strip_prefix("```json").or_else(|| response.strip_prefix("```")).map(|s| s.trim_end_matches("```").trim()).unwrap_or(response);

### 0001-01-01T00:00:00Z
- [src/main.rs:237-244] add_unit_full + delete_inbox_item are not wrapped in a transaction. If the process is killed between them, or delete_inbox_item fails, a unit is created but the inbox item survives, causing a duplicate on the next digest run. Compare with promote_item in db.rs which correctly uses unchecked_transaction(). Move this pair into a transactional helper in db.rs.

- [src/main.rs:237-244] The ? operators on add_unit_full and delete_inbox_item inside the for loop abort the entire command on any DB error, not just the current item. Classify errors are correctly caught and counted. DB errors should be treated the same way (logged, skipped, loop continues) for consistency with the stated skip-and-report behavior (acceptance criterion 5).

- [tests/integration.rs:369-376] test_digest_empty_inbox calls run_ok (asserts success), but check_claude() runs first. If claude is not in PATH in the test environment, the test fails before reaching the empty-inbox branch. Either check empty inbox before checking claude availability in main.rs, or gate this test on claude presence.

### 0001-01-01T00:00:00Z
- [src/digest.rs:16-17] SIMARIS_MODEL defaults to "haiku". Verify this is a valid shorthand accepted by the installed claude CLI version; full model identifiers may be required.

- [src/main.rs:218-259] Digest output bypasses the display module and uses println! directly. --json flag has no effect on digest output. AffectedAreas listed display.rs but it was not changed. Minor inconsistency now, needs addressing if JSON output for digest is required later.

- [src/digest.rs:73] .context(format!(...)) allocates eagerly on every parse attempt. Prefer .with_context(|| format!(...)) for lazy allocation.

- [tests/integration.rs] No test coverage for acceptance criteria 4 (claude not found) or 5 (malformed response — skip and continue). test_digest_no_claude mentioned in the test strategy was not implemented.

### 2026-04-07T16:52:32-04:00
VERDICT:review:REQUEST_CHANGES
