# SWE-bench Current Default Strategy 10-Instance Snapshot

Date: 2026-07-01

This report captures the current Mixmod default strategy on a 10-instance SWE-bench Lite pool. It includes the original three current-strategy instances plus seven additional instances selected by screening for Codex-only official resolves under the same explicit frontier settings.

## Executive Summary

The current Mixmod default strategy preserved patch quality on this selected pool while materially reducing frontier-token usage.

- Codex-only resolved: 10/10
- Mixmod default resolved: 10/10
- Frontier output-token reduction: 51.4%
- Frontier input-token reduction: 76.1%
- Total frontier-token reduction: 75.5%
- Local worker: `local-ollama/qwen3.6:27b`
- Local inference and GPU activity: verified on every Mixmod run

This supports the current working hypothesis for this selected pool: Codex can spend fewer output tokens by supervising a local OpenCode/Qwen worker, while preserving official SWE-bench patch success.

## Selection

The original three instances were:

- `pytest-dev__pytest-11143`
- `scikit-learn__scikit-learn-13439`
- `sympy__sympy-20212`

The seven additional instances were selected by running Codex-only baselines with `gpt-5.5` and high reasoning effort, then keeping instances that resolved under the official SWE-bench evaluator:

- `django__django-12908`
- `pytest-dev__pytest-6116`
- `django__django-13447`
- `django__django-15814`
- `django__django-11179`
- `sympy__sympy-13480`
- `scikit-learn__scikit-learn-13584`

Repository mix:

- Django: 4
- Pytest: 2
- scikit-learn: 2
- SymPy: 2

## Strategy

Mixmod default strategy:

1. Codex receives the user task and produces a compact worker handoff.
2. Mixmod passes the original task plus Codex handoff to OpenCode.
3. OpenCode runs locally with `local-ollama/qwen3.6:27b`.
4. Codex reviews compact artifacts and either asks for another worker attempt, stops as inconclusive/blocked, or approves.
5. If a patch is expected and OpenCode exits with no diff, Mixmod performs one neutral empty-patch follow-up for that worker attempt.

In this report, "Mixmod frontier output tokens" means Codex output tokens during the Mixmod run, not local OpenCode/Qwen output.

## Environment

- Frontier model: `gpt-5.5`
- Frontier reasoning effort: `high`
- Local worker: `local-ollama/qwen3.6:27b`
- Local inference verified: yes on all 10 Mixmod runs
- GPU activity observed: yes on all 10 Mixmod runs
- Global Codex config: untouched during runs; observed hash `b7582816e9bd3139357a8c24c9ea8f0276b9bc445a2e4237046ca9ca40254235`

## Headline Results

| Metric | Codex-only | Mixmod default | Delta |
|---|---:|---:|---:|
| Official SWE-bench resolved | 10/10 | 10/10 | 0 |
| Frontier output tokens | 54,407 | 26,469 | -27,938 (-51.4%) |
| Frontier input tokens | 4,571,174 | 1,093,597 | -3,477,577 (-76.1%) |
| Total frontier tokens | 4,642,623 | 1,137,724 | -3,504,899 (-75.5%) |
| Patch bytes | 17,354 | 14,931 | -2,423 |
| Changed lines | 139 | 137 | -2 |

## Per-Instance Output Tokens

| Instance | Repo | Official Codex-only | Official Mixmod | Codex-only output | Mixmod frontier output | Output delta |
|---|---|---:|---:|---:|---:|---:|
| `pytest-dev__pytest-11143` | pytest | resolved | resolved | 6,864 | 2,361 | -4,503 (-65.6%) |
| `scikit-learn__scikit-learn-13439` | scikit-learn | resolved | resolved | 4,058 | 2,518 | -1,540 (-37.9%) |
| `sympy__sympy-20212` | SymPy | resolved | resolved | 5,490 | 2,826 | -2,664 (-48.5%) |
| `django__django-12908` | Django | resolved | resolved | 2,994 | 1,689 | -1,305 (-43.6%) |
| `pytest-dev__pytest-6116` | pytest | resolved | resolved | 9,848 | 4,317 | -5,531 (-56.2%) |
| `django__django-13447` | Django | resolved | resolved | 4,573 | 1,228 | -3,345 (-73.1%) |
| `django__django-15814` | Django | resolved | resolved | 5,982 | 2,305 | -3,677 (-61.5%) |
| `django__django-11179` | Django | resolved | resolved | 4,023 | 3,139 | -884 (-22.0%) |
| `sympy__sympy-13480` | SymPy | resolved | resolved | 3,117 | 1,229 | -1,888 (-60.6%) |
| `scikit-learn__scikit-learn-13584` | scikit-learn | resolved | resolved | 7,458 | 4,857 | -2,601 (-34.9%) |

## Per-Instance Total Tokens

| Instance | Codex-only input | Mixmod input | Input delta | Codex-only total | Mixmod total | Total delta |
|---|---:|---:|---:|---:|---:|---:|
| `pytest-dev__pytest-11143` | 698,755 | 95,054 | -603,701 (-86.4%) | 707,314 | 98,939 | -608,375 (-86.0%) |
| `scikit-learn__scikit-learn-13439` | 267,497 | 91,004 | -176,493 (-66.0%) | 272,543 | 95,235 | -177,308 (-65.1%) |
| `sympy__sympy-20212` | 771,983 | 259,530 | -512,453 (-66.4%) | 778,739 | 263,715 | -515,024 (-66.1%) |
| `django__django-12908` | 240,946 | 105,375 | -135,571 (-56.3%) | 244,796 | 107,787 | -137,009 (-56.0%) |
| `pytest-dev__pytest-6116` | 675,330 | 125,366 | -549,964 (-81.4%) | 688,476 | 132,926 | -555,550 (-80.7%) |
| `django__django-13447` | 336,046 | 27,183 | -308,863 (-91.9%) | 341,958 | 29,443 | -312,515 (-91.4%) |
| `django__django-15814` | 505,415 | 78,541 | -426,874 (-84.5%) | 513,307 | 82,456 | -430,851 (-83.9%) |
| `django__django-11179` | 302,933 | 90,736 | -212,197 (-70.0%) | 308,256 | 96,220 | -212,036 (-68.8%) |
| `sympy__sympy-13480` | 198,789 | 69,956 | -128,833 (-64.8%) | 202,578 | 71,875 | -130,703 (-64.5%) |
| `scikit-learn__scikit-learn-13584` | 573,480 | 150,852 | -422,628 (-73.7%) | 584,656 | 159,128 | -425,528 (-72.8%) |

## Worker Metrics

| Instance | Codex calls | OpenCode calls | Local worker text bytes | Mixmod final status |
|---|---:|---:|---:|---|
| `pytest-dev__pytest-11143` | 3 | 2 | 19,061 | `approved_by_codex` |
| `scikit-learn__scikit-learn-13439` | 5 | 4 | 26,845 | `approved_by_codex` |
| `sympy__sympy-20212` | 2 | 1 | 8,414 | `approved_by_codex` |
| `django__django-12908` | 3 | 2 | 14,305 | `approved_by_codex` |
| `pytest-dev__pytest-6116` | 8 | 7 | 92,874 | `approved_by_codex` |
| `django__django-13447` | 2 | 1 | 11,874 | `approved_by_codex` |
| `django__django-15814` | 2 | 1 | 16,204 | `approved_by_codex` |
| `django__django-11179` | 6 | 5 | 65,402 | `stopped_by_codex` |
| `sympy__sympy-13480` | 2 | 1 | 7,126 | `approved_by_codex` |
| `scikit-learn__scikit-learn-13584` | 6 | 5 | 63,560 | `approved_by_codex` |
| **Total** | **39** | **29** | **325,665** | |

`django__django-11179` is notable because Codex stopped the loop rather than approving, but the final patch still resolved under the official SWE-bench evaluator. That should be reviewed as a strategy/control-flow issue separately from patch quality.

## Runtime

Mixmod was substantially slower than Codex-only in this snapshot.

| Metric | Codex-only | Mixmod default | Delta |
|---|---:|---:|---:|
| Total wall-clock time | 22.8 min | 85.7 min | +62.9 min |
| Runtime ratio | 1.0x | 3.8x | +2.8x |

Per-instance wall-clock ratios:

| Instance | Codex-only | Mixmod default | Ratio |
|---|---:|---:|---:|
| `pytest-dev__pytest-11143` | 2.9 min | 5.5 min | 1.9x |
| `scikit-learn__scikit-learn-13439` | 1.8 min | 9.2 min | 5.1x |
| `sympy__sympy-20212` | 2.7 min | 4.0 min | 1.5x |
| `django__django-12908` | 1.2 min | 4.0 min | 3.3x |
| `pytest-dev__pytest-6116` | 3.6 min | 19.6 min | 5.5x |
| `django__django-13447` | 1.7 min | 2.8 min | 1.6x |
| `django__django-15814` | 2.8 min | 6.0 min | 2.2x |
| `django__django-11179` | 1.6 min | 17.2 min | 11.0x |
| `sympy__sympy-13480` | 1.2 min | 1.6 min | 1.3x |
| `scikit-learn__scikit-learn-13584` | 3.3 min | 15.7 min | 4.8x |

The runtime result is the main tradeoff in the current prototype: Mixmod preserved official patch success and reduced frontier-token use, but took about 3.8x longer overall. The slowest runs were the ones requiring repeated OpenCode revision attempts.

## Artifacts

- Three-instance snapshot: `docs/swebench-current-default-v1.md`
- Expansion screening state: `.mixmod/swebench/current-default-v1-expansion/screen-state.json`
- Expansion Mixmod state: `.mixmod/swebench/current-default-v1-expansion/mixmod-state.json`
- Expansion official summaries: `.mixmod/swebench/current-default-v1-expansion/eval/official-summary/`
- Expansion logs: `.mixmod/swebench/current-default-v1-expansion/logs/`
- Expansion experiments: `.mixmod/experiments/swebench-current-default-v1-*/`

## Conclusion

On this 10-instance SWE-bench Lite pool, Mixmod current-default matched Codex-only official patch success: both resolved 10/10. Mixmod reduced frontier output tokens from 54,407 to 26,469, a 51.4% reduction, while using local OpenCode/Qwen with verified GPU activity on every Mixmod run.

The frontier input-token reduction was also large at 76.1%, but the primary claim remains output-token reduction with preserved official patch quality on this selected Codex-pass pool.

## Caveats

This is a selected Codex-pass pool, not a random SWE-bench Lite sample. The result should be read as: when Codex can solve the task directly, Mixmod often preserved success while reducing frontier output tokens.

The pool is still small at 10 instances. It includes four Django instances, though additional screening was adjusted to avoid making the pool Django-only.

Some Mixmod runs required many worker turns. `pytest-dev__pytest-6116` used 7 OpenCode calls and 8 Codex calls; `django__django-11179` used 5 OpenCode calls and ended with `stopped_by_codex` even though the official evaluator resolved the patch. Those cases are useful evidence for improving loop control, not failures of patch quality.
