#!/usr/bin/env python3
"""m13 abstain — proper r@5 impact analysis for top signals.

For each candidate signal + threshold, compute actual r@5 and false_fire impact:
  - For each positive: is GT in surfaced top-5 (a "hit")? Does abstain rule silence it?
  - r@5 = kept hits / total positives.
  - false_fire = negatives that still fire / total negatives.

Tests:
  - Single signals at multiple thresholds
  - Combined AND/OR signals
"""
import json
import math
from pathlib import Path

ROOT = Path("/Users/simonspoon/claudehub/simaris")
FEAT_PATH = ROOT / "experiments/m13-abstain/features.json"
OUT_PATH = ROOT / "experiments/m13-abstain/r5_impact.json"


def load_features():
    with open(FEAT_PATH) as f:
        return [r for r in json.load(f) if "error" not in r]


def has_gt_hit(row):
    """Returns True if any expected_unit_id appears in surfaced_unit_ids."""
    if row["negative"]:
        return False
    expected = set(row.get("expected_unit_ids") or [])
    if not expected:
        return False
    surfaced = set(row.get("surfaced_unit_ids") or [])
    return bool(expected & surfaced)


def evaluate_rule(rows, rule_fn, rule_name):
    """rule_fn(row) -> bool. True = FIRE (surface results); False = ABSTAIN.
    Returns dict with r@5, false_fire, etc."""
    n_pos = sum(1 for r in rows if not r["negative"])
    n_neg = sum(1 for r in rows if r["negative"])

    n_kept_pos = 0
    n_kept_pos_with_hit = 0
    n_silenced_pos_with_hit = 0
    n_silenced_pos_without_hit = 0
    n_kept_neg = 0
    n_silenced_neg = 0
    n_total_pos_hits = 0

    for r in rows:
        gt_hit = has_gt_hit(r)
        if not r["negative"]:
            if gt_hit:
                n_total_pos_hits += 1
            if rule_fn(r):
                n_kept_pos += 1
                if gt_hit:
                    n_kept_pos_with_hit += 1
            else:
                if gt_hit:
                    n_silenced_pos_with_hit += 1
                else:
                    n_silenced_pos_without_hit += 1
        else:
            if rule_fn(r):
                n_kept_neg += 1
            else:
                n_silenced_neg += 1

    return {
        "rule": rule_name,
        "n_pos": n_pos,
        "n_neg": n_neg,
        "n_total_pos_hits": n_total_pos_hits,
        "r5_before": n_total_pos_hits / n_pos,
        "r5_after": n_kept_pos_with_hit / n_pos,
        "false_fire_before": 1.0,
        "false_fire_after": n_kept_neg / n_neg,
        "n_kept_pos": n_kept_pos,
        "n_silenced_pos_with_hit": n_silenced_pos_with_hit,
        "n_silenced_pos_without_hit": n_silenced_pos_without_hit,
        "n_kept_neg": n_kept_neg,
        "n_silenced_neg": n_silenced_neg,
        "pos_keep_rate": n_kept_pos / n_pos,
        "neg_silence_rate": n_silenced_neg / n_neg,
    }


def main():
    rows = load_features()

    # Tag substring overlap thresholds
    rules = []
    for theta in [1, 2, 3, 4]:
        rules.append((f"tag_substring >= {theta}",
                      lambda r, t=theta: r.get("tag_substring_overlap", 0) >= t))

    # Tag exact overlap
    for theta in [1, 2, 3]:
        rules.append((f"tag_overlap >= {theta}",
                      lambda r, t=theta: r.get("tag_overlap_count", 0) >= t))

    # Score gini (low = pos)
    for theta in [0.060, 0.070, 0.080, 0.090]:
        rules.append((f"score_gini < {theta}",
                      lambda r, t=theta: r.get("score_gini", 1.0) < t))

    # Score entropy (high = pos)
    for theta in [1.595, 1.600, 1.605, 1.608]:
        rules.append((f"score_entropy >= {theta}",
                      lambda r, t=theta: r.get("score_entropy", 0) >= t))

    # both_legs_top10 (low = pos)
    for theta in [0, 1, 2]:
        rules.append((f"both_legs_top10 <= {theta}",
                      lambda r, t=theta: r.get("both_legs_top10", 999) <= t))

    # Combined: substring AND gini
    for st in [1, 2]:
        for gt_ in [0.07, 0.08]:
            rules.append((f"substring>={st} AND gini<{gt_}",
                          lambda r, s=st, g=gt_: r.get("tag_substring_overlap", 0) >= s
                          and r.get("score_gini", 1.0) < g))

    # Combined: substring OR gini
    for st in [1, 2]:
        for gt_ in [0.07, 0.08]:
            rules.append((f"substring>={st} OR gini<{gt_}",
                          lambda r, s=st, g=gt_: r.get("tag_substring_overlap", 0) >= s
                          or r.get("score_gini", 1.0) < g))

    # Combined: substring AND both_legs_top10 low
    for st in [1, 2]:
        for bt in [0, 1, 2]:
            rules.append((f"substring>={st} AND both_legs_top10<={bt}",
                          lambda r, s=st, b=bt: r.get("tag_substring_overlap", 0) >= s
                          and r.get("both_legs_top10", 999) <= b))

    # substring OR single-leg-top1
    for st in [1, 2, 3]:
        rules.append((f"substring>={st} OR top1_single_leg",
                      lambda r, s=st: r.get("tag_substring_overlap", 0) >= s
                      or (r.get("top5_vec_ranks", [None])[0] is None
                          or r.get("top5_fts_ranks", [None])[0] is None)))

    results = [evaluate_rule(rows, fn, name) for name, fn in rules]

    # Pretty print sorted by best m3-after that meets m1 >= 0.22
    valid = [r for r in results if r["r5_after"] >= 0.22]
    invalid = [r for r in results if r["r5_after"] < 0.22]
    valid.sort(key=lambda r: r["false_fire_after"])

    print(f"\n{'rule':<48} {'r5_aft':<7} {'ff_aft':<7} {'pos_keep':<9} {'neg_sil':<8} {'pos_sil_hit':<11}")
    print("-" * 100)
    print("[VALID — r5_after >= 0.22]")
    for r in valid:
        print(f"{r['rule']:<48} {r['r5_after']:<7.3f} {r['false_fire_after']:<7.3f} "
              f"{r['pos_keep_rate']:<9.3f} {r['neg_silence_rate']:<8.3f} {r['n_silenced_pos_with_hit']:<11}")
    print("\n[INVALID — r5_after < 0.22]")
    for r in sorted(invalid, key=lambda r: -r["r5_after"]):
        print(f"{r['rule']:<48} {r['r5_after']:<7.3f} {r['false_fire_after']:<7.3f} "
              f"{r['pos_keep_rate']:<9.3f} {r['neg_silence_rate']:<8.3f} {r['n_silenced_pos_with_hit']:<11}")

    with open(OUT_PATH, "w") as f:
        json.dump({"results": results, "valid": valid, "invalid": invalid}, f, indent=2)


if __name__ == "__main__":
    main()
