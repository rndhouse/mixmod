# Exploration Avenues

This file records promising directions that need design and experiment work.
These are not claims about current Mixmod behavior.

## Supervisor Session Rotation

The supervisor currently benefits from a persistent Codex session because it can
remember prior worker turns. The downside is that large artifact reads, diffs,
tool logs, and failed-turn evidence remain in the session context. Later
supervisor calls can then carry that accumulated history again as input tokens,
even when the next decision only needs a narrow slice of state.

The DeepSWE `anko-typed-variable-bindings` run
`deepswe-full-qwen-read-deny-20260714T172000Z` showed this failure mode clearly:
the supervisor kept continuity, but late review calls carried roughly
170k-230k input tokens each. The largest direct reads were mostly Mixmod
artifacts such as `tool-events.jsonl`, `supervision-loop-summary.json`,
`report.md`, and repeated patch/diff views.

Explore rotating the supervisor session at phase boundaries or after high-cost
events:

- Large artifact or diff reads.
- Worker context overflow or timeout.
- A long failed-turn recovery sequence.
- Moving from parser/env implementation to tests or final review.

A fresh supervisor session should receive only a compact state summary plus
artifact paths, then inspect only what it needs. The goal is to preserve enough
decision context while avoiding persistent-session token baggage.
