## Approach
Add --debug global flag to CLI. In ask module, pass debug flag through all phases. Each phase writes trace info to stderr via eprintln!. Phase 0: query + extracted search_queries. Phase 1: per-query matches with IDs, dedup count, expansion count. Phase 2: steering result (sufficient + explore IDs). Phase 3: unit IDs used for synthesis. Stdout remains clean (response only). --debug --json includes debug trace in JSON output as a 'debug' field.

## Verify
simaris ask --debug 'how do I release?' 2>trace.log — stdout has answer, trace.log has phase-by-phase output. Without --debug, stderr is silent. cargo test passes.

## Result
Full visibility into ask pipeline for debugging and prompt tuning.

## Outcome
Debug flag working. stderr trace shows all 4 phases. JSON mode includes debug object. 51 tests. Committed as 031eebd.
