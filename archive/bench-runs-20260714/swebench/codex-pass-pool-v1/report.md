# SWE-bench Codex-Pass Pool v1 Report

Selection rule: candidates are kept only if Codex-only resolves the instance under the official SWE-bench Docker harness. This run stopped after finding three Codex-pass instances.

## Headline Metrics

| Metric | Codex-only | Mixmod default | Delta |
|---|---:|---:|---:|
| Frontier output tokens | 13038 | 1907 | -11131 |
| Frontier input tokens | 1398349 | 122676 | -1275673 |
| Frontier total tokens | 1413143 | 125634 | -1287509 |
| Codex-visible bytes | 25523 | 78276 | 52753 |
| Official SWE-bench resolved | 3/3 | 3/3 | 0 |
| Codex calls | 3 | 9 | 6 |
| OpenCode calls | 0 | 5 | 5 |
| Local-worker text bytes | 0 | 17288 | 17288 |

Mixmod default beat Codex-only on frontier output tokens: yes.

## Local Worker Verification

- OpenCode models: local-ollama/qwen3.6:27b (local-ollama/qwen3.6:27b)
- Local inference verified for all Mixmod runs: yes
- GPU activity observed for all Mixmod runs: yes
- Full Mixmod sessions/raw logs read by Codex: no, per recorded metrics

## Per-Instance Results

| Instance | Codex out | Mixmod out | Out delta | Codex total | Mixmod total | Total delta | Codex resolved | Mixmod resolved | OpenCode calls | GPU | Patch bytes C/B | Experiment |
|---|---:|---:|---:|---:|---:|---:|---|---|---:|---|---:|---|
| pytest-dev__pytest-11143 | 6177 | 632 | -5545 | 694510 | 43560 | -650950 | true | true | 2 | true | 1767/557 | `.mixmod/experiments/swebench-codex-pass-pool-v1-pytest-dev__pytest-11143/report.md` |
| sympy__sympy-20212 | 4747 | 594 | -4153 | 549643 | 39956 | -509687 | true | true | 2 | true | 934/502 | `.mixmod/experiments/swebench-codex-pass-pool-v1-sympy__sympy-20212/report.md` |
| scikit-learn__scikit-learn-13439 | 2114 | 681 | -1433 | 168990 | 42118 | -126872 | true | true | 1 | true | 928/467 | `.mixmod/experiments/swebench-codex-pass-pool-v1-scikit-learn__scikit-learn-13439/report.md` |

## Candidate Scan

- `pytest-dev__pytest-11143`: Codex-only resolved, selected.
- `sympy__sympy-20212`: Codex-only resolved, selected.
- `scikit-learn__scikit-learn-13439`: Codex-only resolved, selected.
- Remaining prepared candidates were not run because the three-pass target was reached: `django__django-12908`, `pytest-dev__pytest-6116`, `django__django-16046`.

## Artifact Pointers

- Aggregate metrics: `.mixmod/swebench/codex-pass-pool-v1/metrics.json`
- Codex-only prediction files: `.mixmod/swebench/codex-pass-pool-v1/eval/codex-only-*.jsonl`
- Mixmod prediction file: `.mixmod/swebench/codex-pass-pool-v1/eval/default-selected3-predictions.jsonl`
- Official summary JSONs: `.mixmod/swebench/codex-pass-pool-v1/eval/official-summary/`
- Official Codex reports: `.mixmod/swebench/codex-pass-pool-v1/eval/run_evaluation_logs/mixmod-codex-pass-pool-v1-codex-*/*/*/report.json`
- Official Mixmod reports: `.mixmod/swebench/codex-pass-pool-v1/eval/run_evaluation_logs/mixmod-codex-pass-pool-v1-default-selected3/mixmod-default-codex-pass-pool-v1/*/report.json`

## Conclusion

On this Codex-pass three-instance subset, Mixmod default preserved official SWE-bench pass rate at 3/3 while reducing frontier output tokens by 11131 tokens. The reduction came with more Codex calls and more frontier input/context, while verbose implementation text moved to the local OpenCode/Qwen worker.
