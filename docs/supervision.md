# Supervision

Mixmod supervision is a loop where a supervisor model directs and reviews a
worker through Mixmod. The current implementation uses Codex as the supervisor
runner and OpenCode as the worker runner. Mixmod is the mediator between them.

The supervisor does not directly edit the repository in the default strategy. It
also does not talk directly to the worker. The supervisor returns structured
instructions and decisions to Mixmod; Mixmod turns those decisions into worker
tasks, runs the worker, captures artifacts, and asks the supervisor what to do
next.

## Roles

**Supervisor**

- Reads the agent-visible task and limited file context.
- Produces a compact worker brief.
- Reviews worker artifacts after each run.
- Returns `approve`, `revise`, or `stop`.

**Mixmod mediator**

- Builds worker tasks and revision tasks.
- Runs the worker.
- Captures patches, tests, reports, receipts, metrics, and logs.
- Enforces supervisor decisions.

**Worker**

- Receives the Mixmod worker task.
- Inspects, edits, and tests the local repository.
- Produces local output that Mixmod converts into compact artifacts.

## Loop

1. **Prepare the task**

   Mixmod prepares the task for the supervisor/worker loop.

2. **Ask the supervisor for a worker brief**

   The supervisor receives the agent-visible task and small file excerpts. It returns
   minified JSON with a compact handoff for OpenCode. The brief can include:

   - `handoff`: `as_given`, `focused`, `guided`, or `blocked`
   - `expect_patch`
   - `message_to_worker`
   - `files`
   - `checks`
   - `avoid`
   - `risk`

3. **Run the worker**

   Mixmod writes `worker-task.json` from the task plus supervisor brief, then runs
   the worker locally. The worker can inspect files, edit the repository, and run
   checks.

4. **Capture artifacts**

   After the worker run, Mixmod captures compact review artifacts:

   - `receipt.json`
   - `report.md`
   - `worktree.patch`
   - `changes.patch`
   - `tests.json`
   - `metrics.json`

   `worktree.patch` is the accumulated current repository diff and is
   authoritative when the supervisor decides whether a patch exists. `changes.patch` is
   the latest worker-run delta and may be empty after a verification-only
   revision.

5. **Ask the supervisor to review**

   The supervisor reviews those artifacts in a read-only turn. It must return
   JSON matching this shape:

   ```json
   {
     "action": "approve|revise|stop",
     "worker_mode": "continue|context_focus",
     "message_to_worker": "short instruction for the next worker attempt",
     "focus_files": [],
     "required_checks": [],
     "risk": "short risk note"
   }
   ```

6. **Apply the decision, then loop or exit**

   - `approve`: Mixmod exits the loop and accepts the worker result.
   - `revise`: Mixmod writes a revision task from the supervisor review JSON, runs
     OpenCode again, then returns to step 4 to capture and review the new
     artifacts.
   - `stop`: Mixmod exits the loop and records the result as blocked or
     inconclusive.

## Revision Modes

When the supervisor returns `revise`, it also chooses how Mixmod should run the next
worker attempt.

`worker_mode=continue` means Mixmod should resume the previous OpenCode session
when it can resolve the session id. This preserves the worker's local session
context.

`worker_mode=context_focus` means Mixmod should start a fresh OpenCode session
on the same worktree. The repository changes remain, but previous worker chat
context is discarded unless the supervisor repeats the relevant context in
`message_to_worker`.

## Current Implementation Notes

- Supervisor turns currently run through Codex `app-server` in read-only mode.
- The current default strategy records these as app-server-per-turn supervisor
  calls.
- Worker execution is currently direct `opencode run`, with session resume used
  for `worker_mode=continue` when an OpenCode session id is available.
- Mixmod remains the only bridge between the supervisor and worker.
