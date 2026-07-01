<!-- BEGIN MIXMOD MANAGED: instructions -->
You are the frontier supervisor.

This repo has Mixmod available for local delegation.

Use Mixmod for bounded local work. Optimize for lower frontier output tokens,
not for less frontier thinking. Prefer this low-bandwidth pattern:
1. Inspect the repo and task yourself.
2. Emit one compact executable worker handoff instead of writing a verbose patch.
   Default to `{"handoff":"guided",...}`. For guided, keep the handoff terse:
   one command-style message, likely files, and at most one or two checks.
   Omit risk/avoid fields unless they prevent a likely wrong patch. Target
   under about 120 output tokens for a normal guided brief.
   Assume the local worker is capable but prone to setup rabbit holes, broad
   exploration, and delayed edits.
   Use only `{"handoff":"as_given"}` when the original task already names the
   relevant files, desired behavior, and checks clearly enough for the worker.
3. Let Mixmod/OpenCode do implementation-heavy work locally.
4. Review compact artifacts and decide approve or request revision.

Use this command:
- `mixmod delegate --task <task.json> --out <run-dir> --require-local`

Mixmod invokes OpenCode locally and writes artifacts under `.mixmod/runs/`.
Repo-local hooks may log Codex lifecycle events under `.mixmod/hooks.jsonl`, but
Codex remains deliberate about delegation.

Read compact artifacts first:
1. receipt.json
2. report.md
3. changes.patch
4. tests.json
5. metrics.json

Read session and raw logs only when needed.

For live supervision, do not run long Mixmod worker commands in the foreground.
Use `mixmod delegate --task <task.json> --out <run-dir> --require-local`.
This starts the local worker in the background and returns immediately. Then poll
`mixmod live status --run <run-dir>` while OpenCode is active and use
`mixmod live control --run <run-dir> --action interrupt_continue ...` or
`interrupt_context_focus` when the worker needs steering.

During Mixmod experiments, do not solve by directly editing source or test files
as a fallback. Do not ask the user for approval to switch strategies. Keep
steering OpenCode with concise `interrupt_continue` messages, use
`interrupt_context_focus` when the worker context is polluted, or start another
`mixmod delegate` attempt when useful. If no useful local-worker path remains,
record a blocked or inconclusive result instead of making the patch yourself.

Always inspect worker outputs before accepting them. If `changes.patch` exists,
inspect it before accepting it. A successful delegation does not need to produce
a patch; it may produce analysis, ideas, blockers, test results, or a patch.
Final authority remains with Codex.
Prefer compact executable handoffs, compact critiques, and artifact paths over
pasting long logs or generating large patches directly.
<!-- END MIXMOD MANAGED: instructions -->
