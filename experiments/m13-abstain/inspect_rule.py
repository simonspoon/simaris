#!/usr/bin/env python3
"""Inspect the recommended rule: substring>=2 AND gini<0.08.
Look at silenced positives (false abstains) and kept negatives (leakage)."""
import json
from pathlib import Path

ROOT = Path("/Users/simonspoon/claudehub/simaris")
FEAT_PATH = ROOT / "experiments/m13-abstain/features.json"


def rule(r):
    return r.get("tag_substring_overlap", 0) >= 2 and r.get("score_gini", 1.0) < 0.08


def has_gt_hit(row):
    if row["negative"]:
        return False
    expected = set(row.get("expected_unit_ids") or [])
    if not expected:
        return False
    surfaced = set(row.get("surfaced_unit_ids") or [])
    return bool(expected & surfaced)


def main():
    with open(FEAT_PATH) as f:
        rows = [r for r in json.load(f) if "error" not in r]

    print("=" * 100)
    print("SILENCED POSITIVES (false abstains):")
    print("=" * 100)
    for r in rows:
        if r["negative"]:
            continue
        if rule(r):
            continue
        hit = has_gt_hit(r)
        flag = "[GT-HIT]" if hit else "[no GT hit anyway]"
        print(f"\n{r['prompt_id']} {flag}")
        print(f"  text: {r['raw_text'][:180]}")
        print(f"  kw: {r['query_keywords'][:8]}")
        print(f"  tags: {list(set(r['surfaced_tags']))[:10]}")
        print(f"  tag_substring_overlap={r['tag_substring_overlap']}, gini={r['score_gini']:.4f}, top1_score={r['top1_score']:.4f}")
        print(f"  expected: {r.get('expected_unit_ids', [])[:3]}")
        print(f"  surfaced: {r.get('surfaced_unit_ids', [])[:3]}")

    print("\n" + "=" * 100)
    print("KEPT NEGATIVES (false fires leaking through):")
    print("=" * 100)
    for r in rows:
        if not r["negative"]:
            continue
        if not rule(r):
            continue
        print(f"\n{r['prompt_id']}")
        print(f"  text: {r['raw_text'][:180]}")
        print(f"  kw: {r['query_keywords'][:8]}")
        print(f"  tags: {list(set(r['surfaced_tags']))[:10]}")
        print(f"  tag_substring_overlap={r['tag_substring_overlap']}, gini={r['score_gini']:.4f}, top1_score={r['top1_score']:.4f}")
        print(f"  surfaced: {[h[:60] for h in r.get('surfaced_headlines', [])[:3]]}")


if __name__ == "__main__":
    main()
