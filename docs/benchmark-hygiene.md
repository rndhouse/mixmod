# Benchmark Hygiene

This project uses SWE-bench data for internal experiments. A valid benchmark run
must avoid giving the agent information that makes the task easier than the
public benchmark task.

## Agent-Visible Data

Agents may see:

- instance id
- repository
- base commit
- benchmark name, dataset, split, and version
- problem statement
- generic constraints, such as keeping the patch focused
- explicit tests only when those tests are part of the public task protocol

## Evaluator-Only Data

Agents must not see:

- `FAIL_TO_PASS`
- `PASS_TO_PASS`
- hidden test names
- hidden test patches
- gold implementation patches
- issue hints from the dataset
- candidate-pool or selection-rule metadata
- environment setup commits, unless required for reproducible setup outside the
  model prompt

## Enforcement

`src/task.rs` defines the model-facing task boundary. Prompt builders and
worktree task files must use `agent_visible_task_value` or
`write_agent_visible_task_file`.

The canonical experiment task may keep full metadata for audit and evaluation,
but any file inside an agent worktree named `task.json` must be sanitized before
the model or worker runs.

Worker tasks and revision tasks are also built from sanitized task data. They
must not link back to an unsanitized source task because the worker can inspect
its own artifacts.

## Comparability

For public comparisons, report the scaffold:

- dataset and split
- model and reasoning effort
- prompt template
- tool access
- whether local workers were used
- whether any benchmark metadata was visible

If the scaffold differs from a public leaderboard scaffold, describe the result
as a SWE-bench-based internal evaluation rather than a leaderboard-equivalent
score.
