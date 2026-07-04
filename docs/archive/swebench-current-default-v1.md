# SWE-bench Current Default Strategy Snapshot

Date: 2026-07-01

This report captures the current Mixmod default strategy on the three SWE-bench Lite instances selected for clean comparison. It excludes older guided, self-check-only, and exploratory runs.

## Strategy

Mixmod default strategy:

1. Codex receives the user task and produces a compact worker handoff.
2. Mixmod passes the original task plus Codex handoff to OpenCode.
3. OpenCode runs locally with `local-ollama/qwen3.6:27b`.
4. Codex reviews compact artifacts and either asks for another worker attempt or approves.
5. If a patch is expected and OpenCode exits with no diff, Mixmod performs one neutral empty-patch follow-up for that worker attempt.

The main metric is supervisor output tokens. In this report, "Mixmod supervisor output tokens" means Codex output tokens during the Mixmod run, not local OpenCode/Qwen output.

## Environment

- Supervisor model: `gpt-5.5`
- Supervisor reasoning effort: `high`
- Local worker: `local-ollama/qwen3.6:27b`
- Local inference verified: yes on all three Mixmod runs
- GPU activity observed: yes on all three Mixmod runs
- Global Codex config: untouched during runs; observed hash `b7582816e9bd3139357a8c24c9ea8f0276b9bc445a2e4237046ca9ca40254235`

## Results

| Instance | Official Codex-only | Official Mixmod | Codex-only output | Mixmod supervisor output | Output delta |
|---|---:|---:|---:|---:|---:|
| `pytest-dev__pytest-11143` | resolved | resolved | 6,864 | 2,361 | -4,503 (-65.6%) |
| `scikit-learn__scikit-learn-13439` | resolved | resolved | 4,058 | 2,518 | -1,540 (-37.9%) |
| `sympy__sympy-20212` | resolved | resolved | 5,490 | 2,826 | -2,664 (-48.5%) |
| **Total** | **3/3** | **3/3** | **16,412** | **7,705** | **-8,707 (-53.1%)** |

## Token Metrics

| Instance | Codex-only input | Mixmod supervisor input | Input delta | Codex-only total | Mixmod supervisor total | Total delta |
|---|---:|---:|---:|---:|---:|---:|
| `pytest-dev__pytest-11143` | 698,755 | 95,054 | -603,701 (-86.4%) | 707,314 | 98,939 | -608,375 (-86.0%) |
| `scikit-learn__scikit-learn-13439` | 267,497 | 91,004 | -176,493 (-66.0%) | 272,543 | 95,235 | -177,308 (-65.1%) |
| `sympy__sympy-20212` | 771,983 | 259,530 | -512,453 (-66.4%) | 778,739 | 263,715 | -515,024 (-66.1%) |
| **Total** | **1,738,235** | **445,588** | **-1,292,647 (-74.4%)** | **1,758,596** | **457,889** | **-1,300,707 (-74.0%)** |

## Worker And Patch Metrics

| Instance | Codex calls | OpenCode calls | Local worker text bytes | Patch bytes | Changed files | Changed lines |
|---|---:|---:|---:|---:|---:|---:|
| `pytest-dev__pytest-11143` | 3 | 2 | 19,061 | 1,410 | 2 | 9 |
| `scikit-learn__scikit-learn-13439` | 5 | 4 | 26,845 | 1,193 | 2 | 14 |
| `sympy__sympy-20212` | 2 | 1 | 8,414 | 1,284 | 2 | 11 |
| **Total** | **10** | **7** | **54,320** | **3,887** | **6** | **34** |

## Artifacts

- Pytest Mixmod run: `.mixmod/experiments/swebench-expect-patch-v1-pytest-dev__pytest-11143/default/`
- Pytest official eval: `.mixmod/swebench/expect-patch-v1/eval/official-summary/mixmod-expect-patch-v1.expect-patch-v1.json`
- Scikit-learn Mixmod run: `.mixmod/experiments/swebench-current-default-v1-scikit-learn__scikit-learn-13439/default/`
- Scikit-learn official eval: `.mixmod/swebench/current-default-v1/eval/official-summary/mixmod-current-default-v1.current-default-v1-scikit.json`
- SymPy Mixmod run: `.mixmod/experiments/swebench-current-default-v1-sympy__sympy-20212/default/`
- SymPy official eval: `.mixmod/swebench/current-default-v1/eval/official-summary/mixmod-current-default-v1.current-default-v1-sympy.json`
- Codex-only baseline metrics: `.mixmod/experiments/swebench-explicit-gpt55-high-v1-*/codex-only/metrics.json`

## Conclusion

On this three-instance SWE-bench Lite snapshot, the current Mixmod default strategy matched Codex-only patch quality on the selected tasks: both resolved 3/3. Mixmod reduced supervisor output tokens from 16,412 to 7,705, a 53.1% reduction, while using local Qwen/OpenCode with verified GPU activity.

The input-token reduction was also large, but it should be treated as a secondary result. The primary result is that supervisor output tokens dropped materially while official benchmark resolution was preserved on this small selected pool.
