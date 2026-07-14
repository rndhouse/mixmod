# SWE-bench Explicit Model/Effort Rerun

Run id: `explicit-gpt55-high-v1`

Date: 2026-06-30

## Verdict

Yes, rerunning with model and effort set explicitly was a good idea. It removed
an ambiguity from the earlier pilot and exposed the real tradeoff: Mixmod default
substantially reduced frontier output tokens, but it failed to preserve full
patch quality on this three-instance sample.

Codex-only resolved all 3 selected instances. Mixmod default resolved 2 of 3 and
produced an empty patch for `sympy__sympy-20212`.

## Leading Metrics

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
| Artifact bytes | 0 | 81,134 | +81,134 |
| Wall-clock time | 445.6s | 775.1s | +329.5s |
| Official SWE-bench resolved | 3/3 | 2/3 | -1 |
| Official empty patches | 0 | 1 | +1 |

## Model And Effort

Codex was invoked explicitly with:

```bash
codex exec --json \
  --ignore-user-config \
  --dangerously-bypass-approvals-and-sandbox \
  --model gpt-5.5 \
  -c model_reasoning_effort=\"high\" \
  -C <work-dir> \
  -o <last-message-json> \
  -
```

The Codex rollout metadata for all measured turns recorded:

```text
model = gpt-5.5
reasoning_effort = high
```

OpenCode was invoked explicitly with:

```bash
opencode run --dangerously-skip-permissions --model local-ollama/qwen3.6:27b ...
```

The worker was Qwen 3.6 27B through local Ollama, not Qwen 3.6 35B. This matches
the current repo-local default after the user pulled `qwen3.6:27b`.

## Local Inference Verification

All Mixmod default runs recorded:

```text
opencode_model_arg = local-ollama/qwen3.6:27b
local_inference_verified = true
gpu_activity_observed = true
backend_activity_observed = true
```

During the runs, `ollama ps` showed `qwen3.6:27b` loaded at 100% GPU. `nvidia-smi`
samples during worker execution showed high GPU utilization, including 97%,
100%, and 97% samples across the three instances.

Global Codex config was not modified. Automated Codex calls used isolated
`CODEX_HOME` directories under each experiment worktree.

## Per-Instance Results

| Instance | Codex out | Mixmod out | Out delta | Codex total | Mixmod total | Official Codex | Official Mixmod | Mixmod patch |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| `pytest-dev__pytest-11143` | 6,864 | 608 | -6,256 | 707,314 | 43,860 | resolved | resolved | non-empty |
| `sympy__sympy-20212` | 5,490 | 516 | -4,974 | 778,739 | 38,902 | resolved | empty patch | empty |
| `scikit-learn__scikit-learn-13439` | 4,058 | 2,267 | -1,791 | 272,543 | 78,578 | resolved | resolved | non-empty |

## Official SWE-bench Evaluation

Codex-only evaluator summary:

```json
{
  "submitted_instances": 3,
  "completed_instances": 3,
  "resolved_instances": 3,
  "empty_patch_instances": 0,
  "error_instances": 0
}
```

Mixmod default evaluator summary:

```json
{
  "submitted_instances": 3,
  "completed_instances": 2,
  "resolved_instances": 2,
  "empty_patch_instances": 1,
  "error_instances": 0
}
```

The empty patch instance was:

```text
sympy__sympy-20212
```

## How The Benchmark Was Executed

Each benchmark instance was materialized as a Mixmod experiment with two clean
worktrees:

```text
.mixmod/experiments/swebench-explicit-gpt55-high-v1-<instance>/
  task.json
  codex-only/
  default/
  work/
    codex-only/
    default/
```

The Codex-only arm called:

```bash
target/debug/mixmod experiment record-codex-only <experiment> --task <experiment>/task.json
```

The Mixmod arm called:

```bash
target/debug/mixmod experiment run-default <experiment> --require-local
```

Official patch quality was measured with the SWE-bench Docker harness using the
generated JSONL prediction files:

```text
.mixmod/swebench/explicit-gpt55-high-v1/eval/codex-only-selected3-predictions.jsonl
.mixmod/swebench/explicit-gpt55-high-v1/eval/default-selected3-predictions.jsonl
```

## Artifact Paths

Aggregate metrics:

```text
.mixmod/swebench/explicit-gpt55-high-v1/metrics.json
```

Official evaluator summaries:

```text
.mixmod/swebench/explicit-gpt55-high-v1/eval/mixmod-codex-only-explicit-gpt55-high-v1.explicit-gpt55-high-v1-codex-only.json
.mixmod/swebench/explicit-gpt55-high-v1/eval/mixmod-default-explicit-gpt55-high-v1.explicit-gpt55-high-v1-default.json
```

Raw evaluator logs:

```text
.mixmod/swebench/explicit-gpt55-high-v1/eval/logs/codex-only-selected3-eval.stdout.txt
.mixmod/swebench/explicit-gpt55-high-v1/eval/logs/default-selected3-eval.stdout.txt
```

## Conclusion

The pinned rerun supports the token-saving side of the Mixmod hypothesis but not
the quality-preservation side. Frontier output fell by 13,021 tokens across the
three instances, but Mixmod default lost one official SWE-bench result because
the local worker path ended with an empty SymPy patch.

The next engineering target is not more benchmarking. It is empty-patch
recovery: Mixmod should detect a failed local edit before final evaluation,
surface that exact failure to Codex, and force a worker retry or escalate back
to Codex implementation.
