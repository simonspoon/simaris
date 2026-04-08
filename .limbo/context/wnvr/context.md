## Approach
Two-phase ask: (1) CODE: FTS5 search top matches, fetch 1-hop links for each. (2) LLM CALL 1: given query + matches + links, decide which branches to explore deeper, what's missing. (3) CODE: fetch requested deeper units. (4) LLM CALL 2: synthesize response from all gathered context. Shell out to claude CLI (sonnet). Add src/ask.rs module. Output: synthesized text response, --json returns structured {query, units_used, response}.

## Verify
Use rubric of known-answer questions against dev2 store (196 units from 29 skills). Questions should have verifiable expected content. Build after code.

## Result
simaris ask returns intelligent, synthesized responses by traversing the knowledge graph.

## AcceptanceCriteria
1. simaris ask 'query' returns synthesized response. 2. Response uses relevant units from the graph. 3. Graph traversal follows links intelligently. 4. Empty results handled gracefully. 5. --json returns structured output. 6. Pass rubric of 6 known-answer questions. 7. All existing 47 tests pass.

## ScopeOut
No open-ended agent loop, no caching, no feedback logging yet

## AffectedAreas
src/ask.rs (new), src/db.rs (expand_links function), src/main.rs (Ask subcommand)

## TestStrategy
Unit tests: test_expand_links (verify link following). Integration: test_ask_empty_store, test_ask_json_output. Live validation: rubric of 6 questions against dev2 store. Verify: cargo fmt && cargo clippy && cargo test.

## Risks
Medium: LLM steering quality — may over-fetch or under-fetch. Two calls adds latency. Prompt tuning likely needed after rubric results.
