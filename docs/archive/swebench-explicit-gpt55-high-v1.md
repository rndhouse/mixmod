# SWE-bench Explicit Model/Effort Rerun

Date: 2026-06-30

This rerun repeated the three selected SWE-bench Lite instances with Codex model
and reasoning effort pinned explicitly:

```text
Codex model: gpt-5.5
Codex reasoning effort: high
OpenCode worker: local-ollama/qwen3.6:27b
```

This was the right control to add. The earlier pilot proved the workflow could
work, but its Codex reasoning effort was unset in rollout metadata. This rerun
records the model and effort directly in Mixmod metrics and Codex rollouts.

## Result

| Metric | Codex-only | Mixmod default | Delta |
| --- | ---: | ---: | ---: |
| Frontier output tokens | 16,412 | 3,391 | -13,021 |
| Frontier input tokens | 1,738,235 | 155,887 | -1,582,348 |
| Frontier total tokens | 1,758,596 | 161,340 | -1,597,256 |
| Frontier reasoning tokens | 3,949 | 2,062 | -1,887 |
| Codex-visible bytes proxy | 25,535 | 74,005 | +48,470 |
| Codex calls | 3 | 9 | +6 |
| OpenCode calls | 0 | 6 | +6 |
| Local-worker text bytes | 0 | 25,581 | +25,581 |
| Wall-clock time | 445.6s | 775.1s | +329.5s |
| Official SWE-bench resolved | 3/3 | 2/3 | -1 |

Mixmod default reduced frontier output tokens by 13,021 tokens, but did not
preserve patch quality on this sample. Codex-only resolved all three instances;
Mixmod default resolved two and produced an empty patch for
`sympy__sympy-20212`.

## Per-Instance Outcomes

| Instance | Codex out | Mixmod out | Out delta | Codex total | Mixmod total | Codex eval | Mixmod eval |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- |
| `pytest-dev__pytest-11143` | 6,864 | 608 | -6,256 | 707,314 | 43,860 | resolved | resolved |
| `sympy__sympy-20212` | 5,490 | 516 | -4,974 | 778,739 | 38,902 | resolved | empty patch |
| `scikit-learn__scikit-learn-13439` | 4,058 | 2,267 | -1,791 | 272,543 | 78,578 | resolved | resolved |

## Execution

Each SWE-bench instance was cloned into clean Codex-only and Mixmod-default
worktrees under:

```text
.mixmod/experiments/swebench-explicit-gpt55-high-v1-<instance>/work/
```

The Codex-only arm used:

```bash
target/debug/mixmod experiment record-codex-only <experiment> --task <experiment>/task.json
```

The Mixmod default arm used:

```bash
target/debug/mixmod experiment run-default <experiment> --require-local
```

Mixmod invoked Codex as an isolated non-interactive turn:

```bash
CODEX_HOME=<work-dir>/.mixmod/codex-home \
codex exec --json \
  --ignore-user-config \
  --dangerously-bypass-approvals-and-sandbox \
  --model gpt-5.5 \
  -c model_reasoning_effort=\"high\" \
  -C <work-dir> \
  -o <last-message-json> \
  -
```

Mixmod invoked OpenCode as:

```bash
opencode run --dangerously-skip-permissions --model local-ollama/qwen3.6:27b ...
```

Official patch quality was measured with the SWE-bench Docker harness from the
generated prediction files in:

```text
.mixmod/swebench/explicit-gpt55-high-v1/eval/
```

## Local GPU Verification

The worker model was Qwen 3.6 27B through local Ollama. All Mixmod default runs
recorded local inference as verified, GPU activity observed, and backend
activity observed. During worker execution, `ollama ps` showed `qwen3.6:27b`
loaded at 100% GPU, and `nvidia-smi` samples showed high GPU utilization.

No global Codex config was modified. Automated Codex calls used experiment-local
`CODEX_HOME` directories under `.mixmod/`.

## Artifact Locations

Experiment report:

```text
.mixmod/swebench/explicit-gpt55-high-v1/report.md
```

Aggregate metrics:

```text
.mixmod/swebench/explicit-gpt55-high-v1/metrics.json
```

Official evaluator summaries:

```text
.mixmod/swebench/explicit-gpt55-high-v1/eval/mixmod-codex-only-explicit-gpt55-high-v1.explicit-gpt55-high-v1-codex-only.json
.mixmod/swebench/explicit-gpt55-high-v1/eval/mixmod-default-explicit-gpt55-high-v1.explicit-gpt55-high-v1-default.json
```

## Conclusion

The pinned rerun supports the token-saving claim but not the quality claim.
Mixmod default beat Codex-only on frontier output tokens, input tokens, and total
tokens, but failed the three-instance quality bar because the SymPy local-worker
path produced no patch.

Next, Mixmod should harden empty-patch recovery before scaling the benchmark:
detect failed local edits early, feed the exact failure back into the worker, and
escalate to Codex implementation only when local recovery cannot produce a
non-empty candidate patch.
