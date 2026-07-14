# Archived Benchmark Runs

Archived on 2026-07-14 while returning to the supervised-worker branch from
`pre-worker-role-pivot`.

These records predate the current baseline/accounting cleanup or came from
tool-runner/routing experiments. They should not be used as current benchmark
comparisons without checking the exact run semantics and token accounting.

Committed archive contents:

- `swebench/`: previously tracked SWE-bench result state and official evaluator
  summaries from older experiment series.
- `docs/latest-benchmark.md`: former headline benchmark report. It is archived
  because the accounting and execution setup have changed.

Local ignored runtime state was also moved out of active paths to:

- `.mixmod/archive/bench-runs-20260714/experiments/`
- `.mixmod/archive/bench-runs-20260714/swebench/`
- `.mixmod/archive/bench-runs-20260714/logs/run_evaluation/`

After this archive, active benchmark result directories should be created fresh
under the current branch semantics.
