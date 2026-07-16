use std::path::Path;

use anyhow::{Context, Result};

use crate::*;

use super::common::{
    render_worker_guidance, supervisor_implementation_slice_policy, supervisor_worktree_policy,
};

pub(crate) fn supervisor_live_control_prompt(
    work_dir: &Path,
    snapshot: &LiveWorkerSnapshot,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<String> {
    let snapshot_json = serde_json::to_string_pretty(snapshot)
        .context("failed to serialize live worker snapshot")?;
    let session_context_economics = supervisor_live_session_context_economics();
    let slice_sizing_policy = supervisor_implementation_slice_policy();
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    Ok(format!(
        r#"You are the Mixmod supervisor inspecting a worker turn while it is still running.
{worktree_policy}
Do not run tests. Do not ask the user for approval.
{worker_guidance}
{session_context_economics}
{slice_sizing_policy}

Return only JSON matching this schema:
{{"action":"wait|interrupt_continue|interrupt_context_focus|abort_worker_turn","worker_mode":"continue|context_focus","message_to_worker":"max 80 words","focus_files":[],"required_checks":[],"risk":"max 25 words","worker_turn_shape":"patch_request|bounded_feature_slice|default","turn_goal":"optional next request goal","exact_edits":["optional concrete edit"],"deferred_checks":["optional checks after patch exists"],"defer_checks_until_patch_exists":true,"completion_gate":"optional patch gate","forbidden_actions":["optional worker limits"]}}
Treat applicable worker-model guidance as context for shaping message_to_worker if you choose to interrupt.
Available actions:
- wait: let the worker continue.
- interrupt_continue: stop the current worker process and resume the same worker session with a new instruction.
- interrupt_context_focus: stop the current worker process and start a fresh worker session on the same worktree with a focused instruction.
- abort_worker_turn: stop only the active worker process and return to the ordinary supervisor review.
Base the action on the live evidence. Do not assume an intervention is required because a risk signal is present.
Use new_delta_bytes, stdout_log_path, stderr_log_path, tool_events_path, context_overflow_count, worker_session_token_peak, worker_backend_telemetry, elapsed time, and last output age only as evidence for worker progress, confusion, or blockage.
If you need detailed stdout, stderr, or tool-call history, inspect stdout_log_path, stderr_log_path, or tool_events_path yourself. Do not pass those artifact paths to the worker.
If you interrupt, keep message_to_worker bounded to worker_instruction_excerpt, the live evidence, and the selected worker guidance. For patch_request workers, keep any interrupt patch-first: focused repo files, one concrete implementation slice, deferred checks, and no broad feature instruction.
If the current worker_instruction_excerpt is a planning_probe and you choose interrupt_context_focus, restate enough of the original task goal, focused files, and planning questions in message_to_worker for a fresh worker session to answer without prior context.
Do not solve the task yourself by editing source. Your job is process control: decide whether to keep waiting, interrupt, or abort the worker turn.
The worker can read and edit only the working repo. It cannot read Mixmod task, state, log, or artifact paths.
Do not mention worker-task.json, revision task files, /tmp/mixmod*, /tmp/mixmod-state, or artifact/log paths in message_to_worker or focus_files.
Put only repo source/test paths in focus_files. If you interrupt, restate the next repo edit directly instead of telling the worker to inspect a task or artifact file.
Keep every intervention anchored to worker_instruction_excerpt, which is the current worker task.
Do not invent a different cleanup, bug, or objective from log or tool event artifacts.
Working repo: {work_dir}

Live worker snapshot:
```json
{snapshot_json}
```
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
        slice_sizing_policy = slice_sizing_policy
    ))
}

fn supervisor_live_session_context_economics() -> &'static str {
    r#"Worker session context economics:
- wait or interrupt_continue preserves useful recent file/tool context and can avoid uncached rereads, but later worker calls replay the accumulated session. Cached input tokens are cheaper than uncached input, but a large cached session can still dominate cost and latency.
- interrupt_context_focus starts a fresh worker session on the same source tree. This avoids replaying stale or broad context, but the worker may spend uncached input rereading files, so restate the current patch state and next goal compactly.
- Prefer wait or interrupt_continue when the worker is making progress, the next edit depends on recent context, and worker_session_token_peak/context pressure are modest.
- Prefer interrupt_context_focus after broad investigation, large tool-call bursts, stale context, context overflow, high worker_session_token_peak, or a phase boundary where the next slice can be restated compactly.
- Choose abort_worker_turn when the right next decision requires patch_decision, checkpoint review, or ordinary supervisor artifact review rather than an immediate same-turn intervention."#
}
