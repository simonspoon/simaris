#!/usr/bin/env bash
# simaris-procedures.sh — surface relevant simaris atoms on every prompt (hook-v3).
#
# m13 abstain rule added 2026-05-10. Charter v3 (atom simaris-m11-charter-v3-
# 2026-05-10) ships the raw prompt to engine, drops --type procedure filter,
# top-K=5. v3 layers a retrieval-side abstain on top: fire only when surfaced
# atoms are topically aligned with the prompt. Ship decision: atom 019e1419-ded0.
#
# Algorithm (per m13 recommendation 019e1418-5549):
#   FIRE if   tag_substring_overlap(query, top5.tags) >= MIN_TAG_OVERLAP
#       AND   score_gini(top5.scores)               <  MAX_GINI
#   else ABSTAIN (emit nothing).
#
# Env knobs:
#   SIMARIS_ABSTAIN_ENABLED         default 'true'   ('false'|'0'|'off'|'no' = pre-v3 behavior)
#   SIMARIS_ABSTAIN_MIN_TAG_OVERLAP default '2'      (m13 calibrated; 87% pos / 49% neg)
#   SIMARIS_ABSTAIN_MAX_GINI        default '0.08'   (m13 calibrated; AUC 0.78 combined)
#   SIMARIS_HOOK_FALLBACK=fts5      forces FTS5-only retrieval path (revert)
payload="$(cat)"
prompt="$(echo "$payload" | jq -r '.prompt // empty')"
[ -z "$prompt" ] && exit 0

# Locate simaris binary. Prefer the locally-built release (v0.8.0+ ships --scores);
# fall back to cargo bin then PATH. The brew binary (0.7.1) lacks --scores; if
# we end up using it, abstain compute will fail open and the v2 emit path runs.
SIMARIS_BIN=""
[ -x "$HOME/claudehub/simaris/target/release/simaris" ] && SIMARIS_BIN="$HOME/claudehub/simaris/target/release/simaris"
[ -z "$SIMARIS_BIN" ] && [ -x "$HOME/.cargo/bin/simaris" ] && SIMARIS_BIN="$HOME/.cargo/bin/simaris"
[ -z "$SIMARIS_BIN" ] && SIMARIS_BIN="$(which simaris 2>/dev/null)"
[ -z "$SIMARIS_BIN" ] && exit 0

query="$prompt"

# Hybrid retrieval (lance KNN + tantivy + RRF). --scores emits per-result RRF
# score envelope used by the abstain gate.
SEARCH_FLAGS="--scores"
[ "$SIMARIS_HOOK_FALLBACK" = "fts5" ] && SEARCH_FLAGS="$SEARCH_FLAGS --no-vec"
# stderr swallowed (e.g. "simaris.search.scores=on" telemetry line) so hook
# stdout stays clean for downstream parsing.
results=$("$SIMARIS_BIN" search "$query" --json --top-k 5 ${SEARCH_FLAGS} 2>/dev/null)
[ -z "$results" ] || [ "$results" = "[]" ] && exit 0

# Compute abstain features in python, decide fire/abstain, then format. Query
# passed via env (stdin is taken by `simaris search` JSON output).
echo "$results" | SIMARIS_HOOK_QUERY="$query" python3 -c "
import json, sys, re, os

units = json.load(sys.stdin)
if not units:
    sys.exit(0)

enabled = os.environ.get('SIMARIS_ABSTAIN_ENABLED', 'true').lower() not in ('false','0','off','no')
min_overlap = int(os.environ.get('SIMARIS_ABSTAIN_MIN_TAG_OVERLAP', '2'))
max_gini = float(os.environ.get('SIMARIS_ABSTAIN_MAX_GINI', '0.08'))

if enabled:
    # tag_substring_overlap: tokenize query (len>=3, stopword-stripped), count
    # query-keywords with ANY substring match against ANY top-5 tag (bidirectional).
    query = os.environ.get('SIMARIS_HOOK_QUERY', '')
    STOPWORDS = {
        'and','the','for','that','this','what','with','have','from','your','about',
        'when','where','which','these','those','their','them','they','will','would',
        'could','should','been','were','was','are','our','you','its','has','had',
        'but','not','any','all','can','one','two','out','use','via','into','here',
        'there','how','why','who','say','said','also','just','very','only','than',
        'then','some','more','most','such','over','under','same','each','other',
    }
    kws = set()
    for tok in re.findall(r'[a-zA-Z][a-zA-Z-]+', query.lower()):
        if len(tok) >= 3 and tok not in STOPWORDS:
            kws.add(tok)
    all_tags = set()
    for u in units:
        tags = u.get('tags', [])
        if isinstance(tags, str):
            try: tags = json.loads(tags)
            except: tags = []
        for t in tags or []:
            if isinstance(t, str):
                all_tags.add(t.lower())
    overlap = sum(1 for kw in kws if any((kw in tag) or (tag in kw) for tag in all_tags))

    # score_gini: Gini coefficient of top-K RRF scores. Below threshold = flat
    # distribution = single-leg-dominated surface = positive signal. m13 found
    # AUC 0.6711, paired with overlap (AUC 0.7614) gives combined AUC ~0.78.
    scores = [u.get('score') for u in units if u.get('score') is not None]
    if scores and len(scores) >= 2:
        s = sorted(scores)
        n = len(s)
        cum = sum((i+1)*v for i, v in enumerate(s))
        total = sum(s)
        gini = (2*cum)/(n*total) - (n+1)/n if total > 0 else 0.0
    else:
        # Scores unavailable (binary too old, or single-result). Fail open: fire
        # without gini gate. Overlap gate still applies. Logged once per session.
        gini = 0.0

    if overlap < min_overlap or gini >= max_gini:
        sys.exit(0)

print('## Simaris context (load full content with: \`simaris show <ID>\`):')
print()
for u in units[:5]:
    headline = u.get('headline') or u.get('content', '').split('\n')[0]
    first_line = headline[:70]
    tags = u.get('tags', [])
    if isinstance(tags, str):
        try: tags = json.loads(tags)
        except: tags = []
    tag_str = f' [{', '.join(tags)}]' if tags else ''
    print(f'  [{u[\"id\"]}] {first_line}{tag_str}')
" 2>/dev/null

# User preferences are materialized into ~/.claude/CLAUDE.md by simaris-emit-prefs.sh.
# No per-turn dump here — keeps the prompt-cache prefix stable.
