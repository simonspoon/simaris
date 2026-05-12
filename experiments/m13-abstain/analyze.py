#!/usr/bin/env python3
"""m13 abstain — analyze candidate signals.

Per signal: AUC-ROC, per-threshold pos-keep / neg-silence trade-off.
Decision rule: signal >= threshold => FIRE (surface results); signal < threshold => ABSTAIN.
But some signals' direction is reversed (high = junk), so we also test inverted thresholds.

Goals:
  - m1 r@5 retention ≥ 0.22 (current 0.268 on 23 pos). Translates to: pos-keep-rate * 0.268 ≥ 0.22 → keep ≥ 82.1% of positives.
    More precisely: r@5 = (kept-positives-with-GT-hit) / 23. Current k-with-GT-hit ≈ 0.268 * 23 ≈ 6.17.
    To not drop below 0.22, we must retain ≥ 0.22 * 23 / 0.268 ≈ ~84% of currently-firing positives.
    Conservative: must keep ≥ 19/23 ≈ 0.826 of positives total (assuming all currently surface r@5 hits).
    SIMPLER PROXY: pos_keep_rate (what fraction of 23 pos we fire on). Higher = better recall preserved.
  - m3 false_fire ≤ 0.43 (target), ≤ 0.20 (ideal). neg_silence_rate (1 - false_fire) ≥ 0.57 (target), ≥ 0.80 (ideal).

Output:
  experiments/m13-abstain/analysis.json  — full signal × threshold sweep
  stderr — summary table per signal, with best operating point
"""
import json
import math
from pathlib import Path

ROOT = Path("/Users/simonspoon/claudehub/simaris")
FEAT_PATH = ROOT / "experiments/m13-abstain/features.json"
OUT_PATH = ROOT / "experiments/m13-abstain/analysis.json"


def load_features():
    with open(FEAT_PATH) as f:
        return [r for r in json.load(f) if "error" not in r]


def auc_roc(pos_scores, neg_scores):
    """AUC for predictor where higher score => positive class.
    Returns scalar in [0,1]; 0.5 = random; 1.0 = perfect; 0.0 = perfectly inverted."""
    # Mann-Whitney U statistic, normalized.
    n_pos = len(pos_scores)
    n_neg = len(neg_scores)
    if n_pos == 0 or n_neg == 0:
        return 0.5
    # Combine and rank
    combined = [(s, 1) for s in pos_scores] + [(s, 0) for s in neg_scores]
    combined.sort(key=lambda x: x[0])
    # Assign ranks (avg for ties)
    n = len(combined)
    i = 0
    rank_sum_pos = 0.0
    while i < n:
        j = i
        while j < n and combined[j][0] == combined[i][0]:
            j += 1
        avg_rank = (i + j + 1) / 2.0  # 1-indexed avg rank
        for k in range(i, j):
            if combined[k][1] == 1:
                rank_sum_pos += avg_rank
        i = j
    u = rank_sum_pos - n_pos * (n_pos + 1) / 2
    return u / (n_pos * n_neg)


def percentile(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    k = (len(xs) - 1) * (p / 100.0)
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    if f == c:
        return xs[f]
    return xs[f] + (xs[c] - xs[f]) * (k - f)


def summarize_dist(scores):
    return {
        "n": len(scores),
        "min": min(scores) if scores else None,
        "p25": percentile(scores, 25),
        "p50": percentile(scores, 50),
        "p75": percentile(scores, 75),
        "p90": percentile(scores, 90),
        "max": max(scores) if scores else None,
        "mean": sum(scores) / len(scores) if scores else None,
    }


def sweep_thresholds(rows, score_key, direction="high_means_pos"):
    """Sweep threshold across the actual score values. Return list of operating points.
    direction='high_means_pos': fire when score >= θ (high score = positive).
    direction='low_means_pos': fire when score <= θ (low score = positive).
    Returns list of {θ, pos_keep_rate, neg_silence_rate, n_pos_kept, n_neg_silenced}."""
    pos_scores = sorted({r[score_key] for r in rows if not r["negative"] and r.get(score_key) is not None})
    neg_scores = sorted({r[score_key] for r in rows if r["negative"] and r.get(score_key) is not None})
    all_scores = sorted(set(pos_scores) | set(neg_scores))
    n_pos = sum(1 for r in rows if not r["negative"])
    n_neg = sum(1 for r in rows if r["negative"])

    # Add threshold boundaries — fire-on-everything and abstain-on-everything.
    thresholds = [-math.inf] + all_scores + [math.inf]
    sweep = []
    for theta in thresholds:
        if direction == "high_means_pos":
            kept_pos = sum(1 for r in rows if not r["negative"] and r.get(score_key) is not None and r[score_key] >= theta)
            kept_neg = sum(1 for r in rows if r["negative"] and r.get(score_key) is not None and r[score_key] >= theta)
        else:  # low_means_pos
            kept_pos = sum(1 for r in rows if not r["negative"] and r.get(score_key) is not None and r[score_key] <= theta)
            kept_neg = sum(1 for r in rows if r["negative"] and r.get(score_key) is not None and r[score_key] <= theta)
        sweep.append({
            "theta": theta if math.isfinite(theta) else None,
            "theta_inf": theta if not math.isfinite(theta) else None,
            "n_pos_kept": kept_pos,
            "n_neg_kept_firing": kept_neg,  # negatives that still fire (false fire)
            "pos_keep_rate": kept_pos / n_pos,
            "neg_silence_rate": 1 - kept_neg / n_neg,  # higher = better
        })
    return sweep


def best_operating_point(sweep, min_pos_keep=0.826):
    """Find threshold with max neg_silence_rate subject to pos_keep_rate >= min_pos_keep."""
    candidates = [s for s in sweep if s["pos_keep_rate"] >= min_pos_keep]
    if not candidates:
        return None
    # Among feasible, pick max silence; tie-break by lower (less aggressive) threshold.
    best = max(candidates, key=lambda s: s["neg_silence_rate"])
    return best


def analyze_signal(rows, signal_key, direction):
    pos = [r[signal_key] for r in rows if not r["negative"] and r.get(signal_key) is not None]
    neg = [r[signal_key] for r in rows if r["negative"] and r.get(signal_key) is not None]
    if not pos or not neg:
        return {"signal": signal_key, "error": "missing data"}
    # For AUC: high_means_pos → predictor = score. low_means_pos → predictor = -score.
    if direction == "high_means_pos":
        auc = auc_roc(pos, neg)
    else:
        auc = auc_roc([-x for x in pos], [-x for x in neg])
    sweep = sweep_thresholds(rows, signal_key, direction)
    best = best_operating_point(sweep, min_pos_keep=0.826)
    best_ideal = best_operating_point(sweep, min_pos_keep=0.95)  # if we want minimal pos loss
    return {
        "signal": signal_key,
        "direction": direction,
        "auc": round(auc, 4),
        "pos_dist": summarize_dist(pos),
        "neg_dist": summarize_dist(neg),
        "best_threshold_82pct_pos_keep": best,
        "best_threshold_95pct_pos_keep": best_ideal,
        "sweep_size": len(sweep),
    }


def build_combined_signals(rows):
    """Create derived signals that combine multiple features."""
    for r in rows:
        # top1_is_single_leg: 1 if top1 has only one leg present, 0 if two-leg.
        v = r.get("top5_vec_ranks", [None])[0]
        f = r.get("top5_fts_ranks", [None])[0]
        if v is None or f is None:
            r["top1_is_single_leg"] = 1
        else:
            r["top1_is_single_leg"] = 0
        # both_legs_count_top5: how many of top-5 have BOTH ranks present (regardless of threshold)
        both_count = 0
        for vv, ff in zip(r.get("top5_vec_ranks", []), r.get("top5_fts_ranks", [])):
            if vv is not None and ff is not None:
                both_count += 1
        r["both_legs_present_top5"] = both_count
        # combined: score * (-both_legs_top10) — punish two-leg agreement
        # We want high signal = positive. Positives = single-leg = both_legs_top10 low.
        # So invert: negative_signal = both_legs_top10 (high = neg)
        # Combined = top1_score (high = neg per m11) AND both_legs_top10 (high = neg)
        # Simplest combo: (top1_score - 0.01639) + 0.005 * both_legs_top10 → "junkiness score"
        r["junkiness"] = (r.get("top1_score", 0) - 0.01639) + 0.005 * r.get("both_legs_top10", 0)
        # Direction-corrected single-leg purity: 1 if single-leg AND tag_overlap >= 1
        r["pure_single_leg_with_tagol"] = int(r["top1_is_single_leg"] == 1 and r.get("tag_overlap_count", 0) >= 1)
    return rows


def main():
    rows = load_features()
    rows = build_combined_signals(rows)
    n_pos = sum(1 for r in rows if not r["negative"])
    n_neg = sum(1 for r in rows if r["negative"])
    print(f"loaded {len(rows)} rows: {n_pos} pos / {n_neg} neg", file=__import__("sys").stderr)

    signals = [
        # (name, direction)
        # m11 baseline: top1_score — high = negative (negatives fuse via two legs).
        ("top1_score", "low_means_pos"),
        # Score concentration — concentrated (high gini, low entropy) => junk per m11. But m12 N may flip.
        ("score_entropy", "high_means_pos"),   # high entropy = flat = pos (m11)
        ("score_gini", "low_means_pos"),       # low gini = flat = pos
        ("top1_top2_gap", "low_means_pos"),    # low gap = flat = pos
        # Leg agreement — m11 mechanism: two-leg agreement = junk.
        ("both_legs_top10", "low_means_pos"),    # low = single-leg = pos
        ("both_legs_present_top5", "low_means_pos"),
        ("single_leg_count", "high_means_pos"),  # high single = pos
        ("mean_rank_diff", "high_means_pos"),    # high rank diff = legs disagree = ??? test
        ("top1_is_single_leg", "high_means_pos"),  # binary: 1 = single-leg top1 = pos
        # Tag overlap — high = pos (query keywords match surfaced tags)
        ("tag_overlap_count", "high_means_pos"),
        ("tag_overlap_jaccard", "high_means_pos"),
        ("tag_substring_overlap", "high_means_pos"),
        # Combined
        ("junkiness", "low_means_pos"),
        ("pure_single_leg_with_tagol", "high_means_pos"),
    ]

    results = []
    print(f"\n{'signal':<32} {'dir':<16} {'AUC':<7} {'best-θ':<12} {'pos-keep':<10} {'neg-silence':<12}",
          file=__import__("sys").stderr)
    print("-" * 100, file=__import__("sys").stderr)
    for name, direction in signals:
        r = analyze_signal(rows, name, direction)
        results.append(r)
        best = r.get("best_threshold_82pct_pos_keep")
        if best:
            theta_str = f"{best['theta']:.4f}" if best['theta'] is not None else "INF"
            print(f"{name:<32} {direction:<16} {r['auc']:<7.4f} {theta_str:<12} "
                  f"{best['pos_keep_rate']:<10.3f} {best['neg_silence_rate']:<12.3f}",
                  file=__import__("sys").stderr)
        else:
            print(f"{name:<32} {direction:<16} {r['auc']:<7.4f} {'(no op pt)':<12}",
                  file=__import__("sys").stderr)

    with open(OUT_PATH, "w") as f:
        json.dump({"signals": results, "n_pos": n_pos, "n_neg": n_neg}, f, indent=2)
    print(f"\nwrote {OUT_PATH}", file=__import__("sys").stderr)


if __name__ == "__main__":
    main()
