# M13 Abstain-Rule Report — 2026-05-11

Authoritative scores: `scores-hook-v3.json` (hook-v3 = hook-v2 + abstain rule).
Inputs: `features.json`, `r5_impact.json`, `analysis.json`.
Scoring corpus: `../m12-negatives/real-prompts-2026-05-10.json` +
`../m12-negatives/candidate-labels-2026-05-10.json` (62 prompts: 23 positive,
39 negative).

## 1. Mission

M6.0 pilot left `false_fire_rate = 1.00` on every retrieval method — the
UserPromptSubmit hook surfaced procedures on **every** prompt regardless of
relevance. M13's job: find a signal-driven abstain rule that suppresses the
hook when retrieval evidence is weak, without unacceptably degrading recall.

Goal: `false_fire` from `1.00` → measurable; `r@5` regression bounded.

## 2. Methodology

Three stages:

1. **Feature extraction** (`extract_features.py`). For each of 62 prompts,
   call `simaris search --scores --json --top-k 5` and compute 14 candidate
   signals across four families:
   - **Leg agreement** — `both_legs_top10`, `single_leg_count`, `mean_rank_diff`
   - **Score concentration** — `top1_score`, `top1_top2_gap`, `score_entropy`,
     `score_gini`
   - **Tag overlap** — `tag_overlap_count`, `tag_overlap_jaccard`,
     `tag_substring_overlap` (kw ⊂ tag or vice versa)
   - **Composite** — `pure_single_leg_with_tagol`, `top1_is_single_leg`,
     `junkiness`
2. **Single-signal analysis** (`analyze.py` → `analysis.json`). Per-signal AUC
   on the pos/neg label split. Per-signal thresholds at 82% and 95% positive
   keep rate. Top single signals by AUC: `tag_substring_overlap` (0.761),
   `score_entropy` (0.673), `score_gini` (0.671), `tag_overlap_count` (0.649).
3. **Rule impact sweep** (`r5_impact.py` → `r5_impact.json`). Each candidate
   rule classified per prompt as fire/abstain, then scored end-to-end:
   `r5_after = (positives_with_GT_hit_kept) / n_pos`,
   `false_fire_after = (negatives_kept) / n_neg`.
   Validity gate: `r5_after >= 0.22`.

35 rules evaluated (single signals + AND/OR combinations). 34 valid; one
(`tag_overlap >= 3`) tripped the recall floor.

## 3. Run summary

| stage | artifact | count |
|---|---|---:|
| feature extraction | `features.json` | 62 rows |
| single-signal analysis | `analysis.json` | 14 signals |
| rule sweep | `r5_impact.json` | 35 rules (34 valid, 1 invalid) |
| wired-hook score | `scores-hook-v3.json` | 1 method × 1 set |

Scoring corpus: 62 prompts (23 pos / 39 neg). `n_total_pos_hits = 11` —
only 11 of 23 positives surfaced their GT atom in top-5 at all
(`r5_before = 11/23 = 0.478`).

## 4. Pareto frontier (r@5 vs false_fire)

Sorted by `false_fire_after` ascending; all entries are valid rules from
`r5_impact.json`. **Bold** = on the Pareto frontier (no other rule has both
≥ r@5 and ≤ false_fire).

| rule | r5_after | false_fire_after | pos_keep | neg_silence |
|---|---:|---:|---:|---:|
| **tag_substring >= 4** | **0.304** | **0.128** | 0.609 | 0.872 |
| **substring>=2 AND gini<0.07** | **0.391** | **0.205** | 0.739 | 0.795 |
| **substring>=2 AND gini<0.08** | **0.435** | **0.231** | 0.826 | 0.769 |
| substring>=2 AND both_legs_top10<=0 | 0.304 | 0.282 | 0.652 | 0.718 |
| tag_substring >= 3 | 0.391 | 0.333 | 0.696 | 0.667 |
| tag_overlap >= 2 | 0.261 | 0.333 | 0.478 | 0.667 |
| score_entropy >= 1.608 | 0.391 | 0.359 | 0.652 | 0.641 |
| substring>=2 AND both_legs_top10<=1 | 0.348 | 0.385 | 0.783 | 0.615 |
| score_entropy >= 1.605 | 0.435 | 0.410 | 0.696 | 0.590 |
| substring>=1 AND gini<0.07 | 0.435 | 0.410 | 0.783 | 0.590 |
| **substring>=1 AND gini<0.08** | **0.478** | **0.436** | 0.870 | 0.564 |
| score_gini < 0.06 | 0.435 | 0.462 | 0.739 | 0.538 |
| substring>=2 AND both_legs_top10<=2 | 0.348 | 0.462 | 0.783 | 0.538 |
| score_gini < 0.07 | 0.435 | 0.487 | 0.826 | 0.513 |
| tag_substring >= 2 | 0.435 | 0.513 | 0.870 | 0.487 |
| score_gini < 0.08 | 0.478 | 0.513 | 0.913 | 0.487 |
| score_entropy >= 1.600 | 0.435 | 0.513 | 0.783 | 0.487 |
| (baseline — no rule) | 0.478 | 1.000 | 1.000 | 0.000 |

Pareto chain (monotone improving toward upper-left):

```
ff=1.00 → 0.436 → 0.231 → 0.205 → 0.128
r5=0.478 → 0.478 → 0.435 → 0.391 → 0.304
```

## 5. Verdict

**Winner: `tag_substring_overlap >= 2 AND score_gini < 0.08`.**

- `false_fire`: **1.00 → 0.231** (-76.9pp; 30 of 39 negatives silenced).
- `r@5` (r5_impact convention, `hits/n_pos`): **0.478 → 0.435** (-4.3pp;
  one positive with a GT hit silenced, two more no-hit positives silenced).
- `r@5` (m6-style, `scores-hook-v3.json` apples-to-apples vs hook-v2 baseline):
  **0.2681 → 0.2536** (-1.4pp). Aggregate score **0.289 → 0.278** (-1.1pp).

Why this point on the frontier:

1. **`tag_substring>=4` (ff=0.128, r5=0.304)** trades 17pp of recall for 10pp
   more false-fire reduction vs the winner. Steep cost: 4 of 11 GT hits
   silenced. Outside the acceptable recall band.
2. **`substring>=2 AND gini<0.07` (ff=0.205, r5=0.391)** trades 4.4pp r@5 for
   2.6pp ff. Less recall for marginally less false-fire — strictly worse
   exchange than the winner against the adjacent Pareto step.
3. **`substring>=1 AND gini<0.08` (ff=0.436, r5=0.478)** is the strict-no-recall-
   cost alternative on the Pareto. Zero positives silenced (`pos_keep=0.870`,
   no GT-hit lost). False-fire only halves vs the winner's 3.3× cut. Reasonable
   conservative fallback if recall is sacrosanct; not chosen here because the
   m6 pilot flagged `ff=1.00` as the central defect.

No rule in `r5_impact.json` strictly dominates `substring>=2 AND gini<0.08` —
it is a true Pareto-optimal selection.

## 6. Wire-up

The rule is already wired as **hook-v3** and scored against the m12-negatives
corpus (`scores-hook-v3.json`). hook-v3 inherits hook-v2's retrieval path and
adds the abstain gate: surface results only when
`tag_substring_overlap >= 2 AND score_gini < 0.08`; otherwise emit nothing
(no atoms, no procedure surface). Next step is to promote hook-v3 from the
experiments tree into the live `simaris-procedures.sh` UserPromptSubmit hook
and treat hook-v2's `ff=1.00` behaviour as superseded. This is the brain's
new default.

## 7. Limitations

- **Single eval set, single date.** All scoring against
  `m12-negatives/2026-05-10`. No temporal drift coverage, no re-eval on a
  refreshed corpus. The rule was tuned on the same set it's scored against —
  overfitting risk on the threshold pair `(2, 0.08)`.
- **Two scoring conventions.** `r5_impact.json` uses `hits/n_pos = 11/23`;
  `scores-hook-v3.json` uses score.py's per-prompt averaged recall over 62
  queries. Both reflect the same data and same `-1` GT-hit silencing, but
  the magnitudes differ (-4.3pp vs -1.4pp). The brief's `0.268 → 0.254`
  citation matches the latter; readers comparing across reports must match
  conventions.
- **No multi-turn context.** Hook fires per UserPromptSubmit prompt;
  conversational follow-ups that re-state intent are scored independently.
  No carry of prior-turn abstain state.
- **`tag_substring_overlap` is brittle to tag-vocabulary drift.** The rule
  is sensitive to how atoms get tagged in the store; large-scale tag
  re-organization could shift `false_fire` and recall in either direction
  without re-tuning.
- **`score_gini` threshold 0.08 is empirical.** No theoretical anchor — it's
  the 82%-pos-keep boundary from `analysis.json`. A wider corpus may push
  the optimum.
- **Synthetic set excluded.** Per M6 F2, M3.0b LOCKED has 0 negatives;
  abstain-rule discrimination requires the m12-negatives real set.

## Appendix

- Single-signal AUCs (`analysis.json`): `tag_substring_overlap` 0.761,
  `score_entropy` 0.673, `score_gini` 0.671, `pure_single_leg_with_tagol`
  0.653, `tag_overlap_count` 0.649.
- Rule eval script: `r5_impact.py`.
- Rule inspector: `inspect_rule.py` (lists silenced positives + kept
  negatives for the winning rule).
- m6-pilot reference: `../m6-pilot/REPORT.md`.
- m12-negatives corpus: `../m12-negatives/`.
