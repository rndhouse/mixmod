# Mixmod Artifacts

Mixmod writes experiment and run state under `.mixmod/`. This document records
which artifacts are model-visible and which are bookkeeping-only.

## Experiment Files

`.mixmod/experiments/<name>/task.json`

- Canonical experiment task.
- May contain evaluator-only metadata for audit and scoring.
- Not safe to expose directly to agents.

`.mixmod/experiments/<name>/work/<arm>/task.json`

- Agent-visible task copy.
- Must be sanitized by `src/task.rs` before a run starts.
- Safe for Codex and OpenCode to read.

`.mixmod/experiments/<name>/<arm>/final.patch`

- Captured patch for the arm.
- Used to write SWE-bench prediction JSONL or to compare arms.

`.mixmod/experiments/<name>/<arm>/metrics.json`

- Run telemetry and summary metrics.
- May include internal accounting fields. Treat as compact supervisor artifact,
  not source task data.

## Default Strategy Artifacts

`worker-brief.json`

- Codex-generated compact executable worker handoff.
- Model-visible to later Codex review turns and local worker tasks.
- May include `expect_patch`. When true, Mixmod expects repository edits from a
  patch-mode worker run and may perform one same-session empty-patch follow-up
  before surfacing artifacts to Codex.

`worker-task.json`

- Local worker task generated from the sanitized original task plus the brief.
- Must not contain hidden tests, gold patches, benchmark hints, or links to an
  unsanitized source task.
- Includes a worker-visible `Completion Self-Check` section that asks OpenCode
  to self-check intended edits/checks before finalizing. This is static Mixmod
  harness support, not Codex-generated strategy output.
- Carries `expect_patch` as structured task metadata so Mixmod can distinguish
  "no patch expected" from "patch expected but not captured."

`revision-task.json`

- Local worker revision task generated from sanitized task data and compact
  Codex feedback.
- Must only include repo focus files, not Mixmod artifact paths as source files.

`frontier-feedback.jsonl`

- Codex review and steering turns.
- Internal telemetry and audit trail.

## Run Artifacts

Each `mixmod run` writes:

- `receipt.json`
- `task.json`
- `report.md`
- `session.jsonl`
- `changes.patch`
- `partial.patch`, when timeout recovery captures one
- `tests.json`
- `metrics.json`
- `local-verification.json`
- `logs/opencode.stdout.txt`
- `logs/opencode.stderr.txt`
- `logs/heartbeat.jsonl`
- optional GPU/backend sample logs
- optional `empty-patch-followup/` subdirectory when `expect_patch=true` and
  the first local-worker attempt exits normally without a captured patch

Codex review prompts should read compact artifacts first:

1. `worker-brief.json`
2. `receipt.json`
3. `report.md`
4. `changes.patch`
5. `tests.json`
6. `metrics.json`

Raw logs and full sessions are fallback artifacts, not the default review input.

## Public Result Artifacts

SWE-bench prediction JSONL files should contain only:

- `instance_id`
- `model_name_or_path`
- `model_patch`

Official evaluator summaries and run logs are scoring outputs. They must not be
available to agents during the same benchmark attempt.
