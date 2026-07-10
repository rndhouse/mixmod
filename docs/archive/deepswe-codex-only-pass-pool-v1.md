# DeepSWE Codex-Only Pass Pool v1

Date: 2026-07-10

This note records two additional DeepSWE tasks that direct Codex
`gpt-5.5:high` resolved under the official DeepSWE resolver. These are baseline
tasks for later Mixmod runs with `gpt-5.5` supervision and a local Qwen worker.

The runs used the local Mixmod binary through the DeepSWE Pier runner, but the
agent lane was codex-only: Codex generated the patch directly and the official
resolver graded that patch.

## Passing Baselines

| Task | Runtime | Reward | F2P | P2P | Input | Cached input | Output | Reasoning | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `anko-default-function-arguments` | 676.380s | 1 | 2/2 | 119/119 | 2,618,350 | 2,533,376 | 23,544 | 11,230 | 2,641,894 |
| `tengo-callable-instance-isolation` | 571.843s | 1 | 23/23 | 122/122 | 2,193,232 | 2,107,776 | 18,932 | 7,899 | 2,212,164 |

Both runs reported:

- Model: `openai/gpt-5.5`
- Reasoning effort: `high`
- Codex backend: `exec`
- Token source: `codex_rollout_total_token_usage`
- Codex rollout count: 1
- Codex exit status: 0
- Final status: `success`

## Patch Sizes

| Task | Patch bytes | Patch lines |
| --- | ---: | ---: |
| `anko-default-function-arguments` | 19,127 | 663 |
| `tengo-callable-instance-isolation` | 14,722 | 550 |

## Artifacts

`anko-default-function-arguments`:

```text
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-gpt55high-anko-default-20260710T113929/codex-only-state.json
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-gpt55high-anko-default-20260710T113929/tasks/anko-default-function-arguments
```

`tengo-callable-instance-isolation`:

```text
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-gpt55high-tengo-callable-20260710T122244/codex-only-state.json
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-gpt55high-tengo-callable-20260710T122244/tasks/tengo-callable-instance-isolation
```

## Search Notes

During this search, three codex-only candidate runs did not resolve:

| Task | Runtime | Reward | F2P | P2P | Partial |
| --- | ---: | ---: | ---: | ---: | ---: |
| `katex-multicolumn-array-spans` | 638.413s | 0 | 0/94 | 599/599 | 0.8643578643578643 |
| `termenv-preserve-ansi-resets` | 616.637s | 0 | 34/35 | 87/87 | 0.9918032786885246 |
| `termenv-preserve-ansi-resets` v2 | 531.292s | 0 | 34/35 | 87/87 | 0.9918032786885246 |

Those runs are useful search history, but they are not included in the pass pool.

## Current DeepSWE Baseline Set

Together with the earlier codex-only pass for
`anko-typed-variable-bindings`, we now have three distinct DeepSWE tasks known
to resolve under direct Codex `gpt-5.5:high`:

- `anko-typed-variable-bindings`
- `anko-default-function-arguments`
- `tengo-callable-instance-isolation`
