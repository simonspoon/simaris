#!/usr/bin/env python3
"""auto-prune.py — m12 negatives auto-pruner.

For each unused prompt in the raw harvest (i.e., not in the frozen 30), run
simaris search against the M11 snapshot, capture top-5 results with --scores,
and emit a JSON pool with summary heuristics. The pool feeds manual curation.

Heuristics (informational, NOT a verdict):
  - top1_score        : raw RRF score
  - tag_entropy_top5  : Shannon entropy of tag-bag across top-5 (high = scattered)
  - keyword_overlap   : count of prompt-word ∩ top-5-tags-as-words (low = noise)
  - first_n_atom_ids  : surface info for human reviewer
"""
import json, math, re, subprocess, sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).parent
PILOT = ROOT.parent / "m6-pilot"
SNAPSHOT = PILOT / "snapshot"
RAW_HARVEST = PILOT / "raw-harvest-2026-05-09.json"
FROZEN_PROMPTS = PILOT / "real-prompts-2026-05-09.json"
SIMARIS_BIN = "/Users/simonspoon/claudehub/simaris/target/release/simaris"
OUT = ROOT / "auto-prune-pool.json"

STOPWORDS = set("""
a an the and or but if while of in to for with on at by from as is are was were
be been being do does did have has had this that these those it its
you your we our they their he she them me my i'm i've don't can't won't
what which who whom whose where when why how
all any some no not only just too very also so than then there here now
""".split())

WORD_RE = re.compile(r"[a-z][a-z0-9\-]+")


def keywords(text):
    return [w for w in WORD_RE.findall(text.lower()) if w not in STOPWORDS and len(w) > 2]


def tag_entropy(top5):
    bag = []
    for r in top5:
        bag.extend(r.get("tags") or [])
    if not bag:
        return 0.0
    counts = Counter(bag)
    total = sum(counts.values())
    H = 0.0
    for c in counts.values():
        p = c / total
        H -= p * math.log2(p)
    return H


def keyword_overlap(prompt_text, top5):
    pkw = set(keywords(prompt_text))
    if not pkw:
        return 0
    tagbag = set()
    for r in top5:
        for t in (r.get("tags") or []):
            for w in t.lower().split("-"):
                if len(w) > 2:
                    tagbag.add(w)
        for w in keywords(r.get("headline") or ""):
            tagbag.add(w)
    return len(pkw & tagbag)


def search(prompt_text):
    cmd = [
        SIMARIS_BIN, "search", prompt_text,
        "--scores", "--top-k", "5", "--json",
    ]
    env = dict(__import__("os").environ)
    env["SIMARIS_HOME"] = str(SNAPSHOT)
    try:
        r = subprocess.run(cmd, env=env, capture_output=True, timeout=10, text=True)
        if r.returncode != 0:
            return []
        return json.loads(r.stdout) or []
    except Exception:
        return []


def main():
    raw = json.load(open(RAW_HARVEST))
    frozen_shas = {p["sha256"] for p in json.load(open(FROZEN_PROMPTS))}
    unused = [p for p in raw if p["sha256"] not in frozen_shas]
    print(f"[auto-prune] unused pool: {len(unused)}", file=sys.stderr)

    pool = []
    for i, p in enumerate(unused, 1):
        if i % 50 == 0:
            print(f"  [{i}/{len(unused)}]", file=sys.stderr)
        top5 = search(p["raw_text"])
        # Trim each result to the fields we want
        trimmed = [
            {
                "id": r.get("id"),
                "type": r.get("type"),
                "slug": r.get("slug"),
                "headline": (r.get("headline") or "")[:120],
                "tags": r.get("tags") or [],
                "score": r.get("score"),
            }
            for r in top5
        ]
        top1_score = trimmed[0]["score"] if trimmed else None
        H = tag_entropy(trimmed)
        kov = keyword_overlap(p["raw_text"], trimmed)
        pool.append({
            "orig_harvest_id": p["prompt_id"],
            "raw_text": p["raw_text"],
            "source_session_id": p["source_session_id"],
            "timestamp": p["timestamp"],
            "project": p["project"],
            "sha256": p["sha256"],
            "top5": trimmed,
            "top1_score": top1_score,
            "tag_entropy": round(H, 3),
            "keyword_overlap": kov,
        })

    with open(OUT, "w") as f:
        json.dump(pool, f, indent=2)
    print(f"[auto-prune] wrote {len(pool)} rows to {OUT}", file=sys.stderr)


if __name__ == "__main__":
    main()
