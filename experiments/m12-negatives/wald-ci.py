#!/usr/bin/env python3
"""wald-ci.py — compute Wald 95% CI on m3 false_fire_rate for hook-v2 on expanded panel.

Reads score.py output JSON. Reports point + Wald CI for each method's false_fire on real.
"""
import json, math, sys
from pathlib import Path


def wald_ci(p, n, z=1.96):
    if n == 0:
        return (None, None, None)
    se = math.sqrt(p * (1 - p) / n)
    return (se, max(0.0, p - z * se), min(1.0, p + z * se))


def wilson_ci(p, n, z=1.96):
    """Wilson score 95% CI — accurate at p=0 and p=1 (Wald breaks)."""
    if n == 0:
        return (None, None)
    x = p * n
    denom = n + z * z
    center = (x + z * z / 2.0) / denom
    half = (z / denom) * math.sqrt(x * (n - x) / n + z * z / 4.0)
    return (max(0.0, center - half), min(1.0, center + half))


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "scores-expanded.json"
    d = json.load(open(path))
    real = d["eval_set"]["real"]
    print(f"95% CIs on m3 false_fire_rate (real, expanded panel):")
    print(f"{'method':<18} {'p':>7} {'n':>4} {'SE':>7} {'Wald-lo':>8} {'Wald-hi':>8} {'Wils-lo':>8} {'Wils-hi':>8}")
    for method, m in real.items():
        p = m.get("false_fire_rate")
        n = m.get("n_negative", 0)
        if p is None:
            continue
        se, wlo, whi = wald_ci(p, n)
        wlo2, whi2 = wilson_ci(p, n)
        print(f"{method:<18} {p:>7.3f} {n:>4}  {se:>6.3f}   {wlo:>6.3f}   {whi:>6.3f}   {wlo2:>6.3f}   {whi2:>6.3f}")
    print(f"\nr@5 / MRR on real (positives only):")
    print(f"{'method':<18} {'r@5':>7} {'mrr':>7} {'top1':>7} {'agg':>7}")
    for method, m in real.items():
        print(f"{method:<18} {m['r_at_5'] or 0:>7.3f} {m['mrr'] or 0:>7.3f} {m['top_1'] or 0:>7.3f} {m['aggregate_score']:>7.3f}")


if __name__ == "__main__":
    main()
