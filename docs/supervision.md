# Supervision

Mixmod supervision is a loop where Codex directs and reviews a local OpenCode
worker through Mixmod. Codex is the supervisor, OpenCode is the worker, and
Mixmod is the mediator between them.

Codex does not directly edit the repository in the default strategy. It also
does not talk directly to OpenCode. Codex returns structured instructions and
decisions to Mixmod; Mixmod turns those decisions into worker tasks, runs
OpenCode, captures artifacts, and asks Codex what to do next.

## Roles

**Codex supervisor**

- Reads the agent-visible task and limited file context.
- Produces a compact worker brief for OpenCode.
- Reviews worker artifacts after each run.
- Returns `approve`, `revise`, or `stop`.

**Mixmod mediator**

- Builds worker tasks and revision tasks.
- Runs OpenCode.
- Captures patches, tests, reports, receipts, metrics, and logs.
- Enforces Codex decisions.

**OpenCode worker**

- Receives the Mixmod worker task.
- Inspects, edits, and tests the local repository.
- Produces local output that Mixmod converts into compact artifacts.

## Loop

1. **Prepare the task**

   Mixmod prepares the task for the supervisor/worker loop.

2. **Ask Codex for a worker brief**

   Codex receives the agent-visible task and small file excerpts. It returns
   minified JSON with a compact handoff for OpenCode. The brief can include:

   - `handoff`: `as_given`, `focused`, `guided`, or `blocked`
   - `expect_patch`
   - `message_to_worker`
   - `files`
   - `checks`
   - `avoid`
   - `risk`

3. **Run the worker**

   Mixmod writes `worker-task.json` from the task plus Codex brief, then runs
   OpenCode locally. OpenCode can inspect files, edit the repository, and run
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
   authoritative when Codex decides whether a patch exists. `changes.patch` is
   the latest worker-run delta and may be empty after a verification-only
   revision.

5. **Ask Codex to review**

   Codex reviews those artifacts in a read-only supervisor turn. It must return
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

6. **Apply the decision**

   - `approve`: Mixmod accepts the worker result.
   - `revise`: Mixmod writes a revision task and runs OpenCode again.
   - `stop`: Mixmod records the result as blocked or inconclusive.

## Revision Modes

When Codex returns `revise`, it also chooses how Mixmod should run the next
worker attempt.

`worker_mode=continue` means Mixmod should resume the previous OpenCode session
when it can resolve the session id. This preserves the worker's local session
context.

`worker_mode=context_focus` means Mixmod should start a fresh OpenCode session
on the same worktree. The repository changes remain, but previous worker chat
context is discarded unless Codex repeats the relevant context in
`message_to_worker`.

## Current Implementation Notes

- Codex supervisor turns run through Codex `app-server` in read-only mode.
- The current default strategy records these as app-server-per-turn supervisor
  calls.
- Worker execution is currently direct `opencode run`, with session resume used
  for `worker_mode=continue` when an OpenCode session id is available.
- Mixmod remains the only bridge between Codex and OpenCode.
