# Mixmod Architecture

Mixmod is a CLI harness for comparing direct frontier-model coding with a
frontier-supervised local-worker workflow.

## Boundaries

- `src/main.rs` initializes tracing, parses CLI arguments, and calls `run_cli`.
- `src/lib.rs` currently owns most runtime orchestration: CLI dispatch,
  installation, OpenCode execution, Codex execution, experiment runs, reports,
  filesystem helpers, and git helpers.
- `src/task.rs` owns task loading and benchmark hygiene. Any model-facing task
  representation must pass through this module.

## Main Flows

### Single Mixmod Run

1. Load a task JSON into `TaskSpec`.
2. Build compact OpenCode instructions.
3. Run the configured local OpenCode command.
4. Capture stdout, stderr, heartbeat logs, local verification data, patch,
   tests, metrics, receipt, and report.
5. Return a receipt for Codex or the caller to inspect.

### Experiment: Codex-Only

1. Copy or use an isolated worktree.
2. Write an agent-visible task file into the worktree.
3. Build a generic Codex-only prompt from the sanitized task.
4. Run one isolated `codex exec` turn using an experiment-local `CODEX_HOME`.
5. Capture the resulting patch and telemetry.

### Experiment: Mixmod Default

1. Write an agent-visible task file into the default worktree.
2. Ask Codex for a compact executable worker handoff. Codex may inspect and
   reason freely, but should minimize frontier output. Default to a terse
   guided handoff: one command-style message, likely files, and at most one or
   two checks. Use `as_given` only when the original task is already
   worker-ready.
3. Build a local-worker task from the sanitized original task and Codex brief.
4. Run OpenCode locally.
5. Ask Codex to review compact artifacts.
6. Repeat local revision attempts until approval, stop, or the configured loop
   limit is reached.
7. Capture final patch, tests, metrics, and report.

## Error Context Standard

Fallible operations should include:

- operation: read, parse, write, copy, spawn, wait, evaluate
- phase: codex-only, worker-brief, opencode-implementation, codex-review
- path or cwd
- command and arguments for subprocess failures
- run or experiment identifier when available

Optional telemetry may degrade gracefully, but required artifacts and process
steps should fail with context rather than silently continuing.

## Refactor Direction

The desired end state is a set of narrow modules:

- `cli`
- `config`
- `task`
- `install`
- `opencode`
- `frontier`
- `run`
- `experiment`
- `git`
- `report`
- `swebench`
- `fsutil`

New functionality should prefer these boundaries instead of adding more
unrelated code to `lib.rs`.
