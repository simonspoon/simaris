# LOD-as-graph for simaris

**Status:** draft spec (PdM, 2026-04-22)
**Author:** PdM aspect session
**Storage:** plain md in repo (simaris project does not use limbo)

## Problem

Agent calls returning multiple simaris units (`search`, `list`, `ask`) routinely blow the Claude Code bash output cap. The cap shunts oversized output to disk where the agent cannot read it — simaris becomes unreadable at the moment it is most useful.

Evidence: four `search` and `list` calls during this session returned 70–220 KB each, all spilled to disk. Primary driver is large bodies — aspects are multi-KB, procedures run hundreds of lines.

## Shape

Level-of-detail via graph decomposition, not tiering. Small atomic units, thin bodies, dense link mesh. Agents read the entry, follow citations only when depth is needed. Modeled on Claude Code skill docs: entry file links out to sub-docs; reader pulls what they need.

## Design pillars

- `search` + `list` return lean rows (id, type, slug, headline, tags) — no body by default
- `show` returns the unit body; bodies stay thin because units stay atomic
- Depth composed by graph — agent walks links via follow-up `show` calls
- Authors split big units per a deterministic ruleset
- Write-time signal warns when body exceeds budget

---

## Stories

### 1. Lean default for search + list

Default output of `search` and `list` returns per-unit headline fields only. Full bodies retrieved on demand via `show`.

**Acceptance criteria:**
- `simaris search <query>` default output per unit: id, type, slug, headline, tags, source, confidence
- `simaris list` same shape
- No `body` / `content` field in default text or JSON output
- `--full` flag (or `--body`) restores current behavior (full body per unit)
- `simaris show <id>` unaffected — still returns full body
- Hook-emitted procedure previews (UserPromptSubmit) continue to work (already lean today)
- Scripts parsing JSON that relied on `content` field get a clear missing-field error, not a silent empty value

### 2. Atomicity/split ruleset → cite-able procedure unit

Decomposition rules captured as a single simaris procedure unit that authors cite when writing or refactoring units. Cited by write-time warnings. Itself obeys the ruleset.

**Acceptance criteria:**
- Procedure unit exists with slug `split-ruleset` (or similar)
- Body contains four sections: A atomicity test, B size budget, C split grammar, D keep-together override
- Unit body ≤ chosen size target (validates the ruleset by existing within it)
- Linked from write-time warning messages (citation by slug)
- Size-budget numbers in section B filled from story 4 findings, not guessed

### 3. Write-time size signal

At `add` and `edit` time, simaris measures body size and signals when it crosses configured thresholds.

**Acceptance criteria:**
- Body > warn threshold → stderr warning, write proceeds, message cites `split-ruleset` slug and shows actual vs target bytes
- Body > hard threshold → non-zero exit, write rejected, same citation + size info
- `--force` overrides hard threshold (still prints warning)
- Flow-sequence exception: units tagged `flow` (or flag `--flow`) bypass the warning — shell recipes and numbered procedures legitimately exceed the budget
- Thresholds are CLI-visible (e.g. `simaris config show` or documented env vars) so authors can see what the limits are
- No retroactive enforcement — existing units above threshold are not flagged on read

### 4. Research calibration run

Empirically validate the ruleset and derive size-budget numbers before story 3 thresholds lock. Apply decomposition to PdM aspect + Orokin-init aspect + orokin-protocol.md; run fixed query suite against monolith vs decomposed corpora; record findings.

**Acceptance criteria:**
- Baseline metrics captured on monolith corpus: search bytes, show bytes, total session bytes, per query from fixed suite
- Corpus decomposed per draft ruleset to isolated dev DB (`SIMARIS_ENV=dev` or scratch DB)
- Split metrics captured on decomposed corpus: same queries, same shape
- Comparison table produced: Query | Monolith bytes | Split bytes | Monolith shows | Split shows | Both correct?
- Naturalness check performed: 3 random atoms read alone and judged for standalone readability
- Regression check: monolith answers still surface correctly (confirms baseline was valid)
- Test depth: medium — sub-agent dispatched with simaris-only access, cold start, recorded tool-call trace
- Findings recorded as simaris lesson unit
- Size budgets (warn, hard) chosen from data and written into story 2 ruleset

---

## Scope-out (v1)

- No LLM-generated summary at search time (slow, expensive, adds new failure mode)
- No fixed N-tier model for `show` — rejected during discovery in favor of graph decomposition
- No auto-split at write time — author always decides atomicity
- No auto-retrofit of existing big units — retrofit handled manually or via dream-loop over time
- No changes to `ask --synthesize` (already LOD by nature via LLM)
- No schema change to unit body storage (`content` column unchanged)
- No changes to `simaris show` default behavior

---

## Deterministic ruleset (draft — finalized by story 4)

### A. Atomicity test — "is this one unit?"

1. Name in ≤ 5 words → one idea
2. No "also:" / "additionally:" pivot in body → one idea
3. Agent would cite this atom alone without surrounding text → cite-target worthy
4. Rule reused across contexts → own unit

Fail any → split candidate.

### B. Size budget

1. Warn: body > **2048 bytes** at write — stderr warning, write proceeds, message cites `split-ruleset` slug
2. Hard: body ≥ **4096 bytes** rejects without override
3. Exception: flow sequences (shell recipes, numbered procedures, dispatch briefs) bypass the hard limit via `--flow` flag, tag `flow`, or `--force`
4. Numbers derived from the calibration run — see simaris lesson `lod-calibration-2026-04` for corpus, method, and rationale (split-atom p95=1870, max=2494; smallest failed monolith body=5541)

### C. Split grammar

1. Parent keeps: identity, 1-line overview, TOC of sub-units, cite links to voice/convention units
2. Each child = one atom from section A
3. Link type:
   - `part_of` → child composes parent
   - `depends_on` → step-order dependency
   - `related_to` → lateral reference
4. Body cites by slug (human-readable), not UUID

### D. Keep-together override (block split)

1. Step sequence meaningful only in order → one unit
2. Menu or table meaningless piece-by-piece → one unit
3. Rule + its own tight constraint → one unit

---

## Research procedure (reusable recipe)

```
1. Pick corpus (unit OR file)
2. Apply atomicity test → list atoms
3. Apply split grammar → atoms + link map
4. Write to isolated DB (SIMARIS_ENV=dev)
5. Replay typical queries:
     - search common term
     - show parent entry
     - walk ≥1 link depth
6. Measure:
     - atom count, avg / max body size
     - search hit total bytes
     - show calls needed to answer typical question
7. Judge against signals
8. Record findings → simaris lesson + feed ruleset
```

**Signals:**
- ✓ search total ≤ 20 KB, answer reachable in ≤ 3 `show` calls, atoms read natural alone
- ✗ 10+ `show` calls to assemble one picture (too fine)
- ✗ Still blow cap (too coarse)
- ✗ Atom unusable without sibling context (split broke meaning)

---

## Test plan (for story 4)

**Baseline run (monolith, current state):**
1. Freeze corpus snapshot
2. Run fixed query suite → capture metrics
3. Record: search bytes, show bytes, total session bytes
4. Record: did answer surface? (human judge + note)

**Split run (after ruleset applied):**
1. Same corpus, decomposed per ruleset, fresh DB
2. Same fixed query suite → capture same metrics
3. Extra: count `show` calls needed to assemble full answer

**Comparison table:**

| Query | Monolith bytes | Split bytes | Monolith shows | Split shows | Both correct? |
|-------|----------------|-------------|----------------|-------------|---------------|

**Naturalness check:**
- Pick 3 random atoms, read alone, judge: complete? needs sibling? cite-worthy?
- 0/3 natural → atoms too fine
- All rely on parent → split broke meaning

**Regression check:**
- Confirm monolith answers query at all — baseline sane

### Fixed query suite — PdM aspect

1. "How do holes work in PdM?" → expect hole-handling unit
2. "When does PdM exit?" → expect job boundary + approval/discard
3. "What files can PdM read?" → expect discovery kit + no-src rule
4. "What's the strawman format?" → expect strawman shape unit
5. "Why re-present after every change?" → expect re-present rule unit
6. Off-topic probe — "does PdM write code?" → expect no-code rule

### Fixed query suite — Orokin-init aspect + protocol

*(To be drafted during story 4 execution — mirror the PdM pattern: 5 on-topic probes mapping to expected atoms + 1 off-topic negative probe.)*

### Agent role

Medium depth chosen: sub-agent dispatch with simaris-only tool access, cold start, no prior context. Sub-agent receives each query, produces an answer, trace is recorded (tool calls, bytes consumed, final answer). Reproducible across ruleset tweaks.

---

## Open / deferred

- **Size-budget numbers** — uniform vs tiered per type — answered by story 4 findings
- **`show` "see also" format** — inline footnote vs separate trailing section vs JSON field
- **Auto-expand on `show`** — does `show` resolve one-hop linked-unit headlines automatically, or is `--expand` needed?
- **Headline source** — new stored column vs derived from body first line vs metadata blob
- **Byte counter** — UTF-8 bytes vs chars vs token estimate

## Handoff to PM

Implementation decisions deferred to PM / downstream:
- Where headline lives (schema column vs derived vs metadata)
- Exact byte counter and threshold config surface
- Sub-agent dispatch mechanics for story 4 (which tool, which prompt frame, how metrics get captured)
- `--json` backward-compat strategy for consumers expecting `content` field
- Whether `--full` is a boolean or takes a level argument (future-proofing)
- Shape of write-time warning output (text vs structured)

---

*End of PdM spec. PM can intake for decomposition into executable tasks.*
