# Mixmod SWE-bench Codex-Pass Pool v1

Date of run: 2026-06-30

This document records the last SWE-bench Lite pilot for Mixmod and explains how
the benchmark instances were turned into Codex and Mixmod executions.

## Summary

The run tested the core Mixmod hypothesis on three SWE-bench Lite instances that
Codex-only had already proven it could solve. This was intentional: the point was
not to show that the local worker can rescue cases Codex cannot solve, but to ask
whether Mixmod can preserve patch quality while reducing frontier model output.

Result: both arms resolved all three selected instances under the official
SWE-bench Docker harness. Mixmod reduced frontier output tokens from 13,038 to
1,907, a reduction of 11,131 output tokens, or about 85.4%.

| Metric | Codex-only | Mixmod default | Delta |
| --- | ---: | ---: | ---: |
| Official SWE-bench resolved | 3/3 | 3/3 | 0 |
| Frontier output tokens | 13,038 | 1,907 | -11,131 |
| Frontier input tokens | 1,398,349 | 122,676 | -1,275,673 |
| Frontier total tokens | 1,413,143 | 125,634 | -1,287,509 |
| Codex-visible bytes | 25,523 | 78,276 | +52,753 |
| Codex calls | 3 | 9 | +6 |
| OpenCode calls | 0 | 5 | +5 |
| Local-worker text bytes | 0 | 17,288 | +17,288 |
| Wall-clock time | 399.3s | 506.7s | +107.3s |

The main result supports the low-bandwidth supervision idea for this small
pilot: Codex produced far fewer output tokens while OpenCode/Qwen handled the
implementation text locally. The result is provisional because the sample size is
three instances and the selected set was conditioned on Codex-only passing first.

## Model And Effort

The recorded Codex model was `gpt-5.5` for all 12 Codex session rollouts in this
pilot. The Codex session metadata reported provider `openai` and CLI version
`codex-cli 0.142.4`.

The recorded reasoning effort for this pilot was unset:

```json
"reasoning_effort": null
```

After this run, Mixmod was changed so future isolated Codex calls pass an
explicit repo-local setting:

```bash
-c model_reasoning_effort="high"
```

So this document describes the last measured result, not the next run's exact
configuration.

The local worker model was:

```text
local-ollama/qwen3.6:27b
```

All Mixmod runs recorded local inference as verified, GPU activity observed, and
backend activity observed.

## Selected Instances

The candidate pool contained six SWE-bench Lite instances:

| Instance | Repository | Base commit | Outcome |
| --- | --- | --- | --- |
| `pytest-dev__pytest-11143` | `pytest-dev/pytest` | `6995257cf470d2143ad1683824962de4071c0eb7` | selected |
| `sympy__sympy-20212` | `sympy/sympy` | `a106f4782a9dbe7f8fd16030f15401d977e03ae9` | selected |
| `scikit-learn__scikit-learn-13439` | `scikit-learn/scikit-learn` | `a62775e99f2a5ea3d51db7160fad783f6cd8a4c5` | selected |
| `django__django-12908` | `django/django` | `49ae7ce50a874f8a04cd910882fb9571ff3a0d7a` | not run after three selected |
| `pytest-dev__pytest-6116` | `pytest-dev/pytest` | `e670ff76cbad80108bde9bab616b66771b8653cf` | not run after three selected |
| `django__django-16046` | `django/django` | `ec13e801b820614ff374cb0046092caab8d67249` | not run after three selected |

Selection rule: keep an instance only after Codex-only resolves it under the
official SWE-bench harness. The run stopped after finding three Codex-pass
instances.

## Per-Instance Results

| Instance | Codex out | Mixmod out | Out delta | Codex total | Mixmod total | Total delta | Codex resolved | Mixmod resolved | OpenCode calls | GPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | ---: | --- |
| `pytest-dev__pytest-11143` | 6,177 | 632 | -5,545 | 694,510 | 43,560 | -650,950 | yes | yes | 2 | yes |
| `sympy__sympy-20212` | 4,747 | 594 | -4,153 | 549,643 | 39,956 | -509,687 | yes | yes | 2 | yes |
| `scikit-learn__scikit-learn-13439` | 2,114 | 681 | -1,433 | 168,990 | 42,118 | -126,872 | yes | yes | 1 | yes |

Official SWE-bench summaries reported all three Codex-only patches resolved and
all three Mixmod patches resolved.

Mixmod's own `final_status` for the default arm was `needs_review` on all three
instances. That was a conservative internal supervisor status because the final
Codex approval-check returned `revise`, not because the official benchmark
failed. The official Docker evaluation is the patch-quality result used here.

## Worker Briefs

The default Mixmod strategy begins with a Codex-generated worker brief. The
brief is intentionally sparse because OpenCode also receives the original
SWE-bench task.

Recorded worker briefs:

| Instance | Handoff | Worker-brief output tokens | Notes |
| --- | --- | ---: | --- |
| `pytest-dev__pytest-11143` | `focused` | 175 | Pointed OpenCode at assertion rewrite docstring detection and two focused checks. |
| `sympy__sympy-20212` | `focused` | 230 | Pointed OpenCode at `sympy/core/power.py`, `test_power.py`, and the `0**-oo -> zoo` edge case. |
| `scikit-learn__scikit-learn-13439` | `as_given` | 106 | Codex judged the original task sufficient for the worker. |

This is the low-bandwidth supervision pattern we want to iterate on: Codex uses
intelligence to decide how much guidance the local worker needs, and emits
little or nothing when the task is already clear.

## How A Benchmark Becomes A Codex Run

Each selected SWE-bench instance was represented as a Mixmod experiment:

```text
.mixmod/experiments/swebench-codex-pass-pool-v1-<instance>/
  task.json
  codex-only/
  default/
  work/
    codex-only/
    default/
```

The `task.json` file contains the SWE-bench problem statement, repository, and
base commit. The `work/codex-only` and `work/default` directories are clean
copies checked out at the benchmark base commit.

The Codex-only arm calls:

```text
mixmod experiment record-codex-only <experiment> --task <experiment>/task.json
```

Internally that enters `run_codex_only_baseline`, builds a prompt asking Codex to
solve the task directly, and invokes a single isolated Codex non-interactive
turn.

For the last recorded run, the Codex invocation was equivalent to:

```bash
CODEX_HOME=<work-dir>/.mixmod/codex-home \
codex exec \
  --json \
  --ignore-user-config \
  --dangerously-bypass-approvals-and-sandbox \
  -C <work-dir> \
  -o <artifact-dir>/<label>-last-message.json \
  -
```

The prompt is written to stdin. Codex stdout/stderr are captured under the
experiment logs. Mixmod parses Codex JSONL `token_count` and `turn.completed`
events to extract frontier input/output/reasoning token counts.

The current code now adds:

```bash
-c model_reasoning_effort="high"
```

but that setting was not present in the last measured run.

## Codex Isolation

Mixmod does not modify global Codex config and does not write to `~/.codex`.

For automated runs, Mixmod sets:

```bash
CODEX_HOME=<work-dir>/.mixmod/codex-home
```

and passes:

```bash
--ignore-user-config
```

If local Codex auth is needed, Mixmod temporarily copies `~/.codex/auth.json`
into the experiment-local `CODEX_HOME`, runs the Codex turn, then deletes the
temporary copy. The run metrics record `auth_copied_then_removed`.

The model and effort evidence came from the isolated rollout files under:

```text
.mixmod/experiments/swebench-codex-pass-pool-v1-*/work/*/.mixmod/codex-home/sessions/2026/06/30/
```

Those rollout `turn_context` records show `model = "gpt-5.5"` and
`reasoning_effort = null` for this pilot.

## How The Mixmod Default Arm Works

The default Mixmod arm calls:

```text
mixmod experiment run-default <experiment> --require-local
```

The strategy has five phases:

1. Codex worker brief: Codex reads the task and emits compact JSON guidance, or
   `{"handoff":"as_given"}` when the original task is enough.
2. Local OpenCode proposal: Mixmod turns the original task plus Codex brief into
   a worker task and invokes OpenCode locally.
3. Codex critique: Codex reads compact artifacts only: `receipt.json`,
   `report.md`, `changes.patch`, `tests.json`, `metrics.json`, and
   `worker-brief.json`.
4. Local OpenCode revision: if Codex asks for revision, Mixmod gives the compact
   feedback back to OpenCode.
5. Codex approval-check: Codex gives a final compact structured verdict.

The Codex calls in phases 1, 3, and 5 use the same isolated `CODEX_HOME` pattern
as Codex-only. The difference is that Codex is not asked to write the patch
directly. It is asked to supervise and critique compact artifacts.

## Local GPU Worker Setup

Repo-local Mixmod config selected a local OpenCode model:

```toml
[opencode]
provider = "local"
model = "qwen-3.6-27b"
require_local = true

[opencode.local_verification]
enabled = true
gpu_command = "nvidia-smi"
backend_command = "ollama ps"

[opencode.model_aliases]
"qwen-3.6-27b" = [
  "qwen-3.6-27b",
  "qwen3.6:27b",
  "qwen/qwen3.6-27b",
  "ollama/qwen3.6:27b",
  "local-ollama/qwen3.6:27b"
]
```

Repo-local `opencode.json` exposes:

```text
local-ollama/qwen3.6:27b
```

The OpenCode invocation recorded in the metrics was equivalent to:

```bash
opencode run \
  --dangerously-skip-permissions \
  --model local-ollama/qwen3.6:27b \
  --title <opencode-session-id> \
  <generated-instruction>
```

Under `--require-local`, Mixmod rejects obvious cloud providers and requires the
selected OpenCode provider/model to match the configured local Qwen aliases.

During OpenCode execution Mixmod samples:

```bash
nvidia-smi
ollama ps
```

and writes:

```text
logs/nvidia-smi-before.txt
logs/nvidia-smi-during.txt
logs/nvidia-smi-after.txt
logs/ollama-ps.txt
logs/heartbeat.jsonl
local-verification.json
```

For this pilot, every Mixmod run had:

```json
"local_inference_verified": true,
"gpu_activity_observed": true,
"backend_activity_observed": true
```

## Official Evaluation

After each arm produced patches, Mixmod wrote SWE-bench prediction JSONL files:

```text
.mixmod/swebench/codex-pass-pool-v1/eval/codex-only-*.jsonl
.mixmod/swebench/codex-pass-pool-v1/eval/default-selected3-predictions.jsonl
```

Those predictions were evaluated with the SWE-bench Docker harness via:

```text
swebench.harness.run_evaluation
```

The evaluation logs show:

```text
Total instances: 3
Instances submitted: 3
Instances completed: 3
Instances resolved: 3
Instances unresolved: 0
Instances with empty patches: 0
Instances with errors: 0
```

for the Mixmod selected-three run, and each Codex-only selected instance also
resolved under the same harness.

## Artifact Pointers

Aggregate report:

```text
.mixmod/swebench/codex-pass-pool-v1/report.md
```

Aggregate metrics:

```text
.mixmod/swebench/codex-pass-pool-v1/metrics.json
```

Candidate pool:

```text
.mixmod/swebench/codex-pass-pool-v1/candidates.json
```

Per-instance experiment reports:

```text
.mixmod/experiments/swebench-codex-pass-pool-v1-pytest-dev__pytest-11143/report.md
.mixmod/experiments/swebench-codex-pass-pool-v1-sympy__sympy-20212/report.md
.mixmod/experiments/swebench-codex-pass-pool-v1-scikit-learn__scikit-learn-13439/report.md
```

Official SWE-bench summaries:

```text
.mixmod/swebench/codex-pass-pool-v1/eval/official-summary/
```

Codex rollout files:

```text
.mixmod/experiments/swebench-codex-pass-pool-v1-*/work/*/.mixmod/codex-home/sessions/2026/06/30/
```

OpenCode and GPU verification logs:

```text
.mixmod/experiments/swebench-codex-pass-pool-v1-*/work/default/.mixmod/runs/*/logs/
```

## Conclusion

On this three-instance Codex-pass SWE-bench Lite subset, Mixmod preserved
official patch quality while substantially reducing frontier output tokens. The
run also verified that OpenCode used local Ollama `qwen3.6:27b` with GPU
activity observed.

The next experiment should rerun the same reporting with the now-explicit Codex
reasoning effort `high`, keep the `as_given` handoff optimization, and expand
the selected set beyond three instances.
