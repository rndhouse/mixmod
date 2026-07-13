# DeepSWE Helper-Mode Results v1

Date: 2026-07-14

This note records DeepSWE tasks where direct Codex `gpt-5.5:high` resolved the
task and Mixmod helper mode also resolved it under the official DeepSWE
resolver. In these runs, GPT-5.5 stayed the primary solver while local Qwen was
used as a cheap helper/tool through Mixmod.

These are focused task results, not a broad DeepSWE pass-rate claim.

## Pricing Assumption

The cost estimates use the working prices from the experiment discussion:

- GPT input tokens: $5 per 1M tokens
- GPT output tokens: $30 per 1M tokens

Formula:

```text
cost = (input_tokens * 5 / 1_000_000) + (output_tokens * 30 / 1_000_000)
```

The tables price all reported input tokens at the full input-token rate. If
cached input is billed at a different rate, dollar costs should be recalculated
with that price.

## Results

| Task | Mode | Runtime | Reward | F2P | P2P | GPT input | GPT output | GPT reasoning | Est. GPT cost |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `anko-typed-variable-bindings` | Codex-only baseline | 608.922s | 1 | 9/9 | 94/94 | 2,856,258 | 17,340 | 7,025 | $14.80 |
| `anko-typed-variable-bindings` | Mixmod + local Qwen helper | 621.122s | 1 | 9/9 | 94/94 | 118,314 | 312 | 55 | $0.60 |
| `tengo-callable-instance-isolation` | Clean Codex-only baseline | 477.615s | 1 | 23/23 | 122/122 | 1,820,246 | 19,557 | 9,315 | $9.69 |
| `tengo-callable-instance-isolation` | Mixmod + local Qwen helper | 751.844s | 1 | 23/23 | 122/122 | 101,237 | 332 | 143 | $0.52 |

## Savings

| Task | Input saved | Output saved | GPT cost saved | Runtime change |
| --- | ---: | ---: | ---: | ---: |
| `anko-typed-variable-bindings` | 95.9% | 98.2% | 95.9% | +12.200s |
| `tengo-callable-instance-isolation` | 94.4% | 98.3% | 94.7% | +274.229s |

## Local Helper Use

| Task | Local helper calls | Helper shape |
| --- | ---: | --- |
| `anko-typed-variable-bindings` | 5 | diff review, inspect, checks, bounded review |
| `tengo-callable-instance-isolation` | 7 | 6 command proxies, 1 bounded ask |

For `tengo-callable-instance-isolation`, the local helper ran repeated
`go test ./...` checks successfully. The bounded ask review returned
`needs_supervisor` after idling out, so GPT treated it as incomplete evidence
and still completed the task directly.

## Artifacts

`anko-typed-variable-bindings`:

```text
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-screen/deepswe-codex-gpt55high-anko-typed-20260708T220804Z/screen-state.json
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/mixmod/deepswe-agent-qwen-cli-trustprofile-anko-typed-20260712T223147Z/mixmod-state.json
```

`tengo-callable-instance-isolation`:

```text
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-direct-tengo-callable-20260714T001411Z/codex-only-state.json
/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/mixmod/deepswe-mixmod-qwen-tengo-callable-20260714T002434Z/mixmod-state.json
```

## Interpretation

The important result is that helper mode has now resolved two DeepSWE tasks
that direct Codex also resolved, while using far fewer GPT tokens. Qwen did not
own the patch in these runs; Mixmod made it useful as a local helper for bounded
evidence, command execution, and review while GPT retained final responsibility
for the code and approval.
