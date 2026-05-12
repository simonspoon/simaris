#!/usr/bin/env python3
"""m13 abstain — feature extraction.

For each of 62 prompts, call `simaris search --scores --json --top-k 5 "<text>"`,
then parse engine output to compute candidate abstain signals.

Outputs experiments/m13-abstain/features.json:
[
  {
    "prompt_id": "rp01",
    "raw_text": "...",
    "negative": false,
    "expected_unit_ids": [...],   # GT for positives
    "n_results": 5,
    # Candidate 2 — leg agreement
    "top5_vec_ranks": [int,...],   # vec_rank for each surfaced result
    "top5_fts_ranks": [int,...],
    "both_legs_top10": int,        # how many of top-5 results have BOTH vec_rank<=10 AND fts_rank<=10
    "single_leg_count": int,       # how many surfaced via ONE leg only (rank=infinity in the other)
    # Candidate 3 — score concentration
    "top5_scores": [float,...],
    "top1_score": float,
    "top1_top2_gap": float,
    "score_entropy": float,        # Shannon entropy of normalized top-K scores
    "score_gini": float,           # Gini coefficient
    # Candidate 4 — tag overlap
    "query_keywords": [str,...],
    "surfaced_tags": [str,...],     # flattened
    "tag_overlap_count": int,
    "tag_overlap_jaccard": float,
    # raw passthrough
    "surfaced_unit_ids": [str,...],
    "surfaced_headlines": [str,...]
  },
  ...
]
"""
import json
import math
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path("/Users/simonspoon/claudehub/simaris")
PROMPTS_PATH = ROOT / "experiments/m12-negatives/real-prompts-2026-05-10.json"
LABELS_PATH = ROOT / "experiments/m12-negatives/candidate-labels-2026-05-10.json"
SIMARIS = ROOT / "target/release/simaris"
OUT_PATH = ROOT / "experiments/m13-abstain/features.json"

# Stopword list — minimal, just for query keyword extraction.
# We're looking for content words that might overlap with atom tags.
STOPWORDS = set("""
a an the of to in on at for with by from is are was were be been being have has had
do does did i you he she it we they me him her us them my your his their our its
that this these those there here what which who whom whose when where why how
and or but not no yes if then so as too very can could would should might may will
just only also even than then now so up out off down over under into onto about
let lets like such not very really maybe perhaps thats whats theres im you've ive
some any all each every both either neither one two three four five six seven eight
nine ten 's 're 've 'd 'll 'm 't didnt wasnt isnt arent havent hasnt doesnt dont
look over folder rundown sentences each them them
""".split())


def load_json(p):
    with open(p) as f:
        return json.load(f)


def keywords(text: str) -> list[str]:
    """Extract content keywords from prompt text — lowercase, stopwords stripped,
    min length 3 chars, alphabetic only."""
    tokens = re.findall(r"[a-zA-Z][a-zA-Z\-]+", text.lower())
    out = []
    seen = set()
    for t in tokens:
        if len(t) < 3:
            continue
        if t in STOPWORDS:
            continue
        if t in seen:
            continue
        seen.add(t)
        out.append(t)
    return out


def shannon_entropy(scores: list[float]) -> float:
    """Entropy of normalized score distribution. 0 = perfectly concentrated, log(N) = flat."""
    if not scores:
        return 0.0
    s = sum(scores)
    if s <= 0:
        return 0.0
    ps = [x / s for x in scores]
    return -sum(p * math.log(p) for p in ps if p > 0)


def gini(scores: list[float]) -> float:
    """Gini coefficient of a non-negative score list. 0 = flat, ->1 = concentrated."""
    if not scores:
        return 0.0
    xs = sorted(scores)
    n = len(xs)
    s = sum(xs)
    if s <= 0:
        return 0.0
    cum = 0.0
    for i, x in enumerate(xs):
        cum += (2 * (i + 1) - n - 1) * x
    return cum / (n * s)


def run_search(text: str) -> list[dict]:
    """Run `simaris search --json --scores --top-k 5 <text>` and return parsed list."""
    proc = subprocess.run(
        [str(SIMARIS), "search", "--json", "--scores", "--top-k", "5", text],
        capture_output=True, text=True, timeout=30
    )
    if proc.returncode != 0:
        raise RuntimeError(f"simaris failed (exit {proc.returncode}): {proc.stderr[:500]}")
    # Strip the telemetry line "simaris.search.scores=on" emitted to stderr; stdout is pure JSON.
    return json.loads(proc.stdout)


def featurize(prompt: dict, label: dict) -> dict:
    text = prompt["raw_text"]
    t0 = time.perf_counter()
    results = run_search(text)
    latency_ms = (time.perf_counter() - t0) * 1000

    n = len(results)
    scores = [r.get("score", 0.0) or 0.0 for r in results]
    vec_ranks = [r.get("vec_rank") for r in results]
    fts_ranks = [r.get("fts_rank") for r in results]

    # Candidate 2 — leg agreement
    # vec_rank / fts_rank are 0-indexed positions in each leg's full result list (or None if absent).
    # Wait: looking at output, vec_rank=13 / fts_rank=10 for surfaced atoms — these can be > top-K.
    # If neither leg had the atom, RRF would not have ranked it. Both ranks present (not None) = two-leg agreement.
    # We use ranks <=10 as a "leg fired strongly" heuristic.
    def in_leg(rank, threshold=10):
        return rank is not None and rank <= threshold

    both_top10 = sum(1 for v, f in zip(vec_ranks, fts_ranks) if in_leg(v) and in_leg(f))
    single_leg = sum(1 for v, f in zip(vec_ranks, fts_ranks)
                     if (v is None and f is not None) or (v is not None and f is None))
    # Sum of |vec_rank - fts_rank| over top-K — high = disagreement.
    rank_disagreement = 0.0
    valid_pairs = 0
    for v, f in zip(vec_ranks, fts_ranks):
        if v is not None and f is not None:
            rank_disagreement += abs(v - f)
            valid_pairs += 1
    mean_rank_diff = rank_disagreement / valid_pairs if valid_pairs else None

    # Candidate 3 — concentration
    top1_score = scores[0] if scores else 0.0
    top2_score = scores[1] if len(scores) > 1 else 0.0
    top1_top2_gap = top1_score - top2_score
    ent = shannon_entropy(scores)
    g = gini(scores)

    # Candidate 4 — tag overlap
    kws = keywords(text)
    kw_set = set(kws)
    surfaced_tags = []
    for r in results:
        for tag in (r.get("tags") or []):
            surfaced_tags.append(tag.lower())
    tag_set = set(surfaced_tags)
    overlap = kw_set & tag_set
    jaccard = len(overlap) / len(kw_set | tag_set) if (kw_set | tag_set) else 0.0

    # Also try a more permissive overlap: any kw is SUBSTRING of any tag (or vice versa).
    # Captures e.g. query "release" matching tag "releases" or "release-cli".
    substring_overlap = 0
    for k in kw_set:
        for t in tag_set:
            if k in t or t in k:
                substring_overlap += 1
                break

    return {
        "prompt_id": prompt["prompt_id"],
        "raw_text": text,
        "negative": label["negative"],
        "expected_unit_ids": label.get("expected_unit_ids") or [],
        "search_latency_ms": round(latency_ms, 1),
        "n_results": n,
        "surfaced_unit_ids": [r["id"] for r in results],
        "surfaced_headlines": [r.get("headline", "") for r in results],
        "surfaced_tags": surfaced_tags,
        # leg agreement
        "top5_vec_ranks": vec_ranks,
        "top5_fts_ranks": fts_ranks,
        "both_legs_top10": both_top10,
        "single_leg_count": single_leg,
        "mean_rank_diff": mean_rank_diff,
        # concentration
        "top5_scores": scores,
        "top1_score": top1_score,
        "top1_top2_gap": top1_top2_gap,
        "score_entropy": ent,
        "score_gini": g,
        # tag overlap
        "query_keywords": kws,
        "tag_overlap_count": len(overlap),
        "tag_overlap_jaccard": jaccard,
        "tag_substring_overlap": substring_overlap,
    }


def main():
    prompts = load_json(PROMPTS_PATH)
    labels = {l["prompt_id"]: l for l in load_json(LABELS_PATH)}
    rows = []
    for i, p in enumerate(prompts, 1):
        label = labels.get(p["prompt_id"])
        if not label:
            print(f"[!] no label for {p['prompt_id']}", file=sys.stderr)
            continue
        try:
            row = featurize(p, label)
            rows.append(row)
            print(f"[{i}/{len(prompts)}] {p['prompt_id']} ({'NEG' if label['negative'] else 'POS'}) "
                  f"top1={row['top1_score']:.4f} ent={row['score_entropy']:.3f} both10={row['both_legs_top10']} "
                  f"tagol={row['tag_overlap_count']} lat={row['search_latency_ms']:.0f}ms", file=sys.stderr)
        except Exception as e:
            print(f"[!] {p['prompt_id']} FAILED: {e}", file=sys.stderr)
            rows.append({
                "prompt_id": p["prompt_id"],
                "raw_text": p["raw_text"],
                "negative": label["negative"],
                "expected_unit_ids": label.get("expected_unit_ids") or [],
                "error": str(e),
            })
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(OUT_PATH, "w") as f:
        json.dump(rows, f, indent=2)
    print(f"\nwrote {OUT_PATH} ({len(rows)} rows)", file=sys.stderr)


if __name__ == "__main__":
    main()
