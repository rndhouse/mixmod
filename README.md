# Mixmod

Mixmod is an experimental CLI harness for testing whether a frontier coding model can reduce frontier token usage by supervising a local GPU-backed worker.

Domain placeholder: `mixmod.tech`.

Architecture:

```text
Codex + repo-local hooks/instructions -> Mixmod CLI -> OpenCode -> local GPU model
```

The prototype is built for repeatable comparisons:

1. Codex-only: Codex makes a small code change directly.
2. Mixmod default strategy: Codex emits a compact executable worker handoff, OpenCode implements locally from the original task plus that handoff, and Codex reviews compact artifacts until it approves or stops the loop.

The first success criterion is an experiment loop that compares token exposure, byte/character proxies when exact telemetry is unavailable, patch size, tests, and final quality.

## Install Locally

```sh
cargo build
cargo install --path .
```

Required user tools:

- `codex`
- `mixmod`
- `opencode`

OpenCode is invoked through its non-interactive `opencode run [message..]` mode by default. `mixmod init` also writes a repo-local `opencode.json` that exposes `local-ollama/qwen3.6:27b` without changing global OpenCode config. Override the command in `.mixmod/config.toml` or with:

```sh
MIXMOD_OPENCODE_COMMAND=/path/to/opencode
MIXMOD_OPENCODE_ARGS='run {instruction}'
```

Supported placeholders are `{instruction}`, `{instruction_file}`, `{task_file}`, `{mode}`, `{out_dir}`, `{provider}`, `{model}`, and `{model_arg}`.

## Safety Model

Mixmod keeps integration project-local:

- Global Codex config is treated as read-only.
- `~/.codex` is treated as read-only.
- Generated state lives under `.mixmod/` and `.codex/` in the current repo.
- `mixmod init` writes repo-local instructions, config metadata, repo-local hook config, and a conservative pass-through hook.
- Existing repo-local files are backed up under `.mixmod/backups/` before Mixmod overwrites them.
- `mixmod uninstall` removes only Mixmod-managed integration files and restores backups where applicable.
- OpenCode output is untrusted until Codex reviews the compact artifacts and patch.

## Commands

```sh
mixmod init
mixmod status
mixmod doctor
mixmod uninstall
```

Run one delegated task:

```sh
mixmod delegate --task example.task.json --out .mixmod/runs/example --require-local
```

`mixmod delegate` starts a live-supervised OpenCode worker in the background.
Codex should poll `mixmod live status` and steer with `mixmod live control`.
For this experiment strategy, Codex should not fall back to direct source edits;
if the local worker cannot recover, record the run as blocked or inconclusive.

## Experiment Workflow

```sh
mixmod experiment init parser-fix
mixmod experiment record-codex-only parser-fix --task .mixmod/experiments/parser-fix/task.md
mixmod experiment run-default parser-fix --require-local
mixmod experiment recover parser-fix --require-local
mixmod experiment report parser-fix
```

Seed an experiment from a deterministic fixture and create isolated work directories:

```sh
mixmod experiment init checkout-brief --fixture fixtures/python-checkout
mixmod experiment record-codex-only checkout-brief --task .mixmod/experiments/checkout-brief/task.json
mixmod experiment run-default checkout-brief --require-local
mixmod experiment report checkout-brief
```

The report answers:

- Did both approaches produce a working patch?
- Which tests were run?
- How many files and lines changed?
- How much Codex-visible text was involved?
- How much local-worker text was generated?
- Did Codex need to read the full Mixmod session?
- Did Mixmod appear to reduce frontier context exposure?

## Run Artifact Layout

Each `mixmod run` writes:

```text
receipt.json
task.json
report.md
session.jsonl
changes.patch
tests.json
metrics.json
logs/opencode.stdout.txt
logs/opencode.stderr.txt
logs/heartbeat.jsonl
```

For the default strategy, Codex first emits `worker-brief.json`. The brief is
supplemental: OpenCode receives the original task JSON, so Codex should not
restate it. Mixmod is optimizing for lower frontier output tokens, not less
frontier thinking: Codex may inspect the repo and reason freely, then emit one
compact executable handoff.

The default handoff should be `guided`: assume the local worker is capable but
prone to setup rabbit holes, broad exploration, and delayed edits. Keep guided
terse and executable: one command-style message, likely files, and at most one
or two checks. Omit `risk` and `avoid` unless a short phrase prevents a likely
wrong patch. A normal guided brief should target roughly 120 output tokens or
less.

Use `as_given` only when the original task already names the relevant files,
desired behavior, and checks clearly enough for the local worker:

```json
{"handoff":"as_given"}
```

Mixmod passes guided messages through directly:

```json
{
  "handoff": "guided",
  "message_to_worker": "Fix checkout totals with the smallest safe edit.",
  "files": ["checkout.py"],
  "checks": ["python -m pytest test_checkout.py -q"]
}
```

`files` and `checks` are transport metadata for OpenCode's structured task
sections; `message_to_worker` is the primary handoff. Empty fields should be
omitted. Codex should then read compact artifacts first:

1. `worker-brief.json`
2. `receipt.json`
3. `report.md`
4. `changes.patch`
5. `tests.json`
6. `metrics.json`

Read `session.jsonl` and raw logs only when needed.

Frontier Codex turns use the repo-local Mixmod config. New projects default to:

```toml
[frontier]
model = "gpt-5.5"
reasoning_effort = "high"
```

Mixmod passes this to isolated `codex exec` calls as `--model` and
`model_reasoning_effort`. Allowed reasoning effort values are `minimal`, `low`,
`medium`, `high`, and `xhigh`.

## Worker Supervision And Recovery

Mixmod supervises OpenCode as an untrusted local worker:

- streams OpenCode stdout/stderr to disk while the worker is still running
- writes `logs/heartbeat.jsonl` with elapsed time, output byte counts, GPU/backend observations, and terminal status
- enforces configurable worker and idle timeouts
- captures `partial.patch` when a worker timeout leaves recoverable worktree changes
- records timeout and heartbeat fields in `metrics.json`
- injects static local-worker support instructions, including a completion
  self-check for intended edits/checks, without spending Codex output tokens
- honors structured `expect_patch` metadata from the Codex worker brief; when a
  patch-mode worker exits normally with no captured diff and `expect_patch=true`,
  Mixmod performs one same-session empty-patch follow-up before Codex review

Defaults live in `.mixmod/config.toml`:

```toml
[opencode]
heartbeat_seconds = 10
worker_timeout_seconds = 600
idle_timeout_seconds = 300
```

Per-command overrides are available for experiments:

```sh
MIXMOD_OPENCODE_WORKER_TIMEOUT_SECONDS=120 \
MIXMOD_OPENCODE_IDLE_TIMEOUT_SECONDS=60 \
MIXMOD_OPENCODE_HEARTBEAT_SECONDS=5 \
mixmod experiment run-default parser-fix --require-local
```

If a default-strategy worker stalls after `worker-task.json` exists, restart local work without rerunning the Codex worker-brief phase:

```sh
mixmod experiment recover parser-fix --require-local
```

## Stable File Formats

Task JSON:

```json
{
  "title": "Short task name",
  "instructions": "Bounded task instructions.",
  "files": ["src/lib.rs"],
  "tests": ["cargo test"],
  "constraints": ["Keep the patch focused."],
  "acceptance": ["The named test passes."]
}
```

Receipt JSON:

```json
{
  "run_id": "...",
  "status": "success | failed | needs_supervisor",
  "mode": "patch",
  "summary": "...",
  "changed_files": [],
  "report": "...",
  "patch": "...",
  "session": "...",
  "tests": "...",
  "metrics": "...",
  "logs": "..."
}
```

Metrics JSON captures timestamps, duration, OpenCode command and exit status, stdout/stderr bytes, artifact sizes, test status, changed files/lines, token telemetry when available, byte proxies when not, artifacts read by Codex if observable, and notes about missing telemetry.

## Included Fixture

`fixtures/python-calculator` is a tiny Python `unittest` project with one deliberate failing test. It is used by the first end-to-end experiment stored under `.mixmod/experiments/first-result/`.

`fixtures/python-checkout` is the recommended larger fixture for the current default strategy. It has a multi-file checkout task covering catalog validation, line promotions, and order-level discounts.
