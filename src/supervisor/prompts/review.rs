use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::*;

use super::common::{
    DEBUG_PROFILE_FIT_ENV, render_worker_guidance, supervisor_artifact_index,
    supervisor_implementation_slice_policy, supervisor_worker_shape_contract,
    supervisor_worktree_policy,
};

pub(crate) fn supervisor_feedback_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
) -> Result<String> {
    supervisor_feedback_prompt_inner(
        work_dir,
        artifact_paths,
        instruction,
        worker_guidance,
        context_telemetry,
        strategy,
        env_bool(DEBUG_PROFILE_FIT_ENV).unwrap_or(false),
    )
}

pub(crate) fn supervisor_feedback_approval_consistency_repair_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    previous_feedback: &Value,
    rejection_reason: &str,
) -> Result<String> {
    let previous_feedback = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize inconsistent supervisor feedback")?;
    let instruction = format!(
        r#"Your previous supervisor JSON was internally inconsistent: {rejection_reason}

Previous JSON:
```json
{previous_feedback}
```

Repair only the supervisor decision. Return either:
- action=approve with approval_state=ready_to_approve, required_checks=[], deferred_checks=[], no completion_gate, and approval_contract rows whose statuses are passed, covered_by_existing_test, or not_applicable with compact artifact/source/check evidence; or
- action=revise with patch_decision=revise_current and a verification-focused message_to_worker that asks the worker to run the smallest pending task-derived check and make only targeted fixes if it fails.

Do not approve while listing checks that still need to run or approval_contract rows without deterministic evidence. Do not solve by authoring source changes."#
    );

    supervisor_feedback_prompt(
        work_dir,
        artifact_paths,
        &instruction,
        worker_guidance,
        context_telemetry,
        strategy,
    )
}

pub(crate) fn supervisor_spin_out_feedback_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    review_packet: &Value,
) -> Result<String> {
    supervisor_spin_out_feedback_prompt_inner(
        work_dir,
        artifact_paths,
        instruction,
        worker_guidance,
        context_telemetry,
        strategy,
        review_packet,
        env_bool(DEBUG_PROFILE_FIT_ENV).unwrap_or(false),
    )
}

pub(crate) fn supervisor_spin_out_feedback_approval_consistency_repair_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    review_packet: &Value,
    previous_feedback: &Value,
    rejection_reason: &str,
) -> Result<String> {
    let previous_feedback = serde_json::to_string_pretty(previous_feedback)
        .context("failed to serialize inconsistent supervisor feedback")?;
    let instruction = format!(
        r#"Your previous supervisor JSON was internally inconsistent: {rejection_reason}

Previous JSON:
```json
{previous_feedback}
```

Repair only the supervisor decision using the same REVIEW_PACKET. Return either:
- action=approve with approval_state=ready_to_approve, required_checks=[], deferred_checks=[], no completion_gate, and approval_contract rows whose statuses are passed, covered_by_existing_test, or not_applicable with compact packet/source/check evidence; or
- action=revise with patch_decision=revise_current and a verification-focused message_to_worker that asks the worker to run the smallest pending task-derived check and make only targeted fixes if it fails.

Do not approve while listing checks that still need to run or approval_contract rows without deterministic evidence. Do not solve by authoring source changes."#
    );

    supervisor_spin_out_feedback_prompt(
        work_dir,
        artifact_paths,
        &instruction,
        worker_guidance,
        context_telemetry,
        strategy,
        review_packet,
    )
}

#[cfg(test)]
pub(crate) fn supervisor_feedback_prompt_with_debug_profile_fit(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
) -> Result<String> {
    supervisor_feedback_prompt_inner(
        work_dir,
        artifact_paths,
        instruction,
        worker_guidance,
        context_telemetry,
        strategy,
        true,
    )
}

#[cfg(test)]
pub(crate) fn supervisor_spin_out_feedback_prompt_with_debug_profile_fit(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    review_packet: &Value,
) -> Result<String> {
    supervisor_spin_out_feedback_prompt_inner(
        work_dir,
        artifact_paths,
        instruction,
        worker_guidance,
        context_telemetry,
        strategy,
        review_packet,
        true,
    )
}

fn supervisor_feedback_prompt_inner(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    debug_profile_fit: bool,
) -> Result<String> {
    let artifact_index = supervisor_artifact_index(work_dir, artifact_paths);
    let review_context = supervisor_feedback_review_context(artifact_paths);
    let shape_contract = supervisor_worker_shape_contract(worker_guidance);
    let session_context_economics = supervisor_feedback_session_context_economics();
    let approval_contract_policy = supervisor_feedback_approval_contract_policy();
    let context_telemetry = serde_json::to_string_pretty(&context_telemetry.to_prompt_json())
        .context("failed to serialize supervisor context telemetry")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    let worktree_policy = supervisor_worktree_policy();
    let slice_sizing_policy = supervisor_implementation_slice_policy();
    let strategy_policy = supervisor_feedback_strategy_policy(strategy);
    let decision_debug = supervisor_feedback_decision_debug(strategy, debug_profile_fit);
    let action_schema = supervisor_feedback_action_schema(strategy, decision_debug.json_field);
    Ok(format!(
        r#"You are a terse supervisor reviewing a local worker.
{worktree_policy}
Do not ask the user for approval.
Inspect the listed core artifact files directly before deciding. Do not rely on this prompt as artifact content; it only names where the review evidence lives.
Treat supervisor input tokens as scarce. Inspect only the artifacts needed for the next decision and stop reading once the next action is clear.
For ordinary worker-turn review, start with task context, receipt/report, review-signals.json, loop summary, and changes.patch. review-signals.json names conditional diagnostic artifacts; open them only for the stated use_when case. Inspect the full active diff only when considering approval, rollback, or an integration question that depends on cross-turn state.
{worker_guidance}
Worker shape contract:
{shape_contract}

{slice_sizing_policy}

{session_context_economics}

{approval_contract_policy}

{strategy_policy}
{decision_debug_requirements}

Supervisor context telemetry:
```json
{context_telemetry}
```

If you choose revise, shape the worker request yourself before emitting JSON.
Always include context_recommendation. Use action="compact_now" only at a clean semantic boundary where the next supervisor turn can rely on compacted history plus fresh artifacts. Use action="compact_after_next_worker" when the next worker turn should happen first but the following supervisor review should start from compacted history. Otherwise use action="continue". Mixmod makes the final compaction decision from this recommendation and hard telemetry.
Return only JSON matching this schema:
{action_schema}
Use "expect_patch":false with worker_turn_shape="planning_probe" when the next useful worker turn should only inspect bounded repo context and propose the next patch request. After a planning_probe result, approve or trim its proposal by issuing a normal revise implementation turn; do not approve the whole task merely because the plan is reasonable.
{review_context}
Working repo: {work_dir}
Instruction: {instruction}

Artifact index:
{artifact_index}
"#,
        work_dir = work_dir.display(),
        worktree_policy = worktree_policy,
        slice_sizing_policy = slice_sizing_policy,
        strategy_policy = strategy_policy,
        approval_contract_policy = approval_contract_policy,
        decision_debug_requirements = decision_debug.requirements,
        action_schema = action_schema,
    ))
}

fn supervisor_spin_out_feedback_prompt_inner(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
    review_packet: &Value,
    debug_profile_fit: bool,
) -> Result<String> {
    let review_packet = serde_json::to_string_pretty(review_packet)
        .context("failed to serialize supervisor review packet")?;
    let review_context = supervisor_feedback_review_context(artifact_paths);
    let shape_contract = supervisor_worker_shape_contract(worker_guidance);
    let session_context_economics = supervisor_feedback_session_context_economics();
    let approval_contract_policy = supervisor_feedback_approval_contract_policy();
    let context_telemetry = serde_json::to_string_pretty(&context_telemetry.to_prompt_json())
        .context("failed to serialize supervisor context telemetry")?;
    let worker_guidance = render_worker_guidance(worker_guidance);
    let slice_sizing_policy = supervisor_implementation_slice_policy();
    let strategy_policy = supervisor_feedback_strategy_policy(strategy);
    let decision_debug = supervisor_feedback_decision_debug(strategy, debug_profile_fit);
    let action_schema = supervisor_feedback_action_schema(strategy, decision_debug.json_field);
    Ok(format!(
        r#"You are a one-shot spin-out supervisor reviewer for Mixmod.
Use only REVIEW_PACKET below. Do not run commands, inspect files, browse the repository, or use tools. Paths in the packet are labels for evidence already collected by Mixmod.
If the packet is insufficient, do not fetch more context. Return a revise or stop decision with "insufficient_context":true and "requested_context":["small bounded missing evidence"].
Do not ask the user for approval.
Treat supervisor input tokens as scarce. Decide from the packet and stop once the next action is clear.
Existing review policy may say "inspect", "open", or "read" artifacts; in this spin-out mode that means use packet excerpts only.
{worker_guidance}
Worker shape contract:
{shape_contract}

{slice_sizing_policy}

{session_context_economics}

{approval_contract_policy}

{strategy_policy}
{decision_debug_requirements}

Supervisor context telemetry:
```json
{context_telemetry}
```

If you choose revise, shape the worker request yourself before emitting JSON.
Always include context_recommendation. Use action="compact_now" only at a clean semantic boundary where the next supervisor turn can rely on compacted history plus fresh artifacts. Use action="compact_after_next_worker" when the next worker turn should happen first but the following supervisor review should start from compacted history. Otherwise use action="continue". Mixmod makes the final compaction decision from this recommendation and hard telemetry.
Return only JSON matching this schema, plus optional insufficient_context/requested_context fields when the packet lacks needed evidence:
{action_schema}
Use "expect_patch":false with worker_turn_shape="planning_probe" when the next useful worker turn should only inspect bounded repo context and propose the next patch request. After a planning_probe result, approve or trim its proposal by issuing a normal revise implementation turn; do not approve the whole task merely because the plan is reasonable.
{review_context}
Working repo: {work_dir}
Instruction: {instruction}

REVIEW_PACKET:
```json
{review_packet}
```
"#,
        work_dir = work_dir.display(),
        slice_sizing_policy = slice_sizing_policy,
        strategy_policy = strategy_policy,
        approval_contract_policy = approval_contract_policy,
        decision_debug_requirements = decision_debug.requirements,
        action_schema = action_schema,
    ))
}

struct FeedbackDecisionDebugPrompt {
    requirements: &'static str,
    json_field: &'static str,
}

fn supervisor_feedback_decision_debug(
    strategy: DefaultStrategyMode,
    enabled: bool,
) -> FeedbackDecisionDebugPrompt {
    if !enabled || !default_strategy_policy(strategy).debug_delegation_decision {
        return FeedbackDecisionDebugPrompt {
            requirements: "",
            json_field: "",
        };
    }
    FeedbackDecisionDebugPrompt {
        requirements: r#"
Debug delegation-decision audit:
- Include "delegation_decision" on every review decision.
- delegation_decision.next_owner must be "worker", "supervisor", or "none".
- delegation_decision.work_type must be "construction", "correction", "verification", "approval", or "blocked".
- delegation_decision.why must explain why the next step belongs with the normal worker or a supervisor_direct_edit patch under worker-build-supervisor-fix.
- If action=revise, delegation_decision.worker_fit must name the broad construction slice that remains worker-scale; if you cannot name one, choose action=supervisor_direct_edit instead.
- If action=supervisor_direct_edit, delegation_decision.direct_fit must name the localized correction the fresh GPT patch session will make."#,
        json_field: r#","delegation_decision":{"next_owner":"worker|supervisor|none","work_type":"construction|correction|verification|approval|blocked","why":"debug-only","worker_fit":"debug-only when revising","direct_fit":"debug-only when using supervisor_direct_edit"}"#,
    }
}

fn supervisor_feedback_session_context_economics() -> &'static str {
    r#"Worker session context economics:
- worker_mode=continue reuses useful recent file/tool context and can avoid uncached rereads, but later worker calls replay the accumulated session. Cached input tokens are cheaper than uncached input, but a large cached session can still dominate cost and latency.
- worker_mode=context_focus starts a fresh worker session on the same source tree. This avoids replaying stale or broad context, but the worker may spend uncached input rereading files, so restate the current patch state and next goal compactly.
- Prefer worker_mode=continue when the next edit depends on recent worker context and worker_session_token_peak/context pressure are modest.
- Prefer worker_mode=context_focus after broad investigation, large tool-call bursts, stale context, context overflow, high worker_session_token_peak, or a phase boundary where the next slice can be restated compactly.
- Use patch_decision=accept_current_baseline with worker_mode=context_focus when useful incomplete progress should remain in the source tree but the next worker turn should start with a clean active diff and fresh session context."#
}

fn supervisor_feedback_approval_contract_policy() -> &'static str {
    r#"Approval contract policy:
- On every review after a non-empty patch, maintain approval_contract rows derived from the original task and likely regression risks. Use generic categories: required behavior, default/disabled behavior, boundary cases, error behavior, integration/API surface, and focused regression checks when relevant.
- Do not copy a fixed checklist across tasks. Choose rows from the actual task, changed source surface, and worker evidence.
- Set approval_state to not_close, broad_work_remaining, approval_possible_after_verification, ready_to_approve, or blocked. Start checking whether approval is possible as soon as any useful patch exists; keep it cheap until approval_possible_after_verification.
- Use approval_possible_after_verification when broad construction appears complete but deterministic evidence is still missing. The next action should normally be a fixed task-derived smoke check, a targeted repair from a failing check, or supervisor_direct_edit only for a surgical known fix.
- Use ready_to_approve only when each approval_contract row is passed, covered_by_existing_test with evidence, or not_applicable with a reason. Approval requires deterministic artifact/source/check evidence, not worker confidence or summary text alone."#
}

#[derive(Default)]
struct SupervisorFeedbackPromptSignals {
    artifact_names: BTreeSet<String>,
    worker_turn_shape: Option<String>,
    context_overflow: bool,
    context_pressure: bool,
    latest_delta_empty: bool,
    supervisor_control_seen: bool,
    tool_events_available: bool,
    patch_request_progress_streak: bool,
}

fn supervisor_feedback_review_context(artifact_paths: &[PathBuf]) -> String {
    let signals = supervisor_feedback_prompt_signals(artifact_paths);
    let mut sections = vec![supervisor_feedback_core_context(&signals)];

    if signals.worker_turn_shape.as_deref() == Some("patch_request") {
        sections.push(
            r#"Patch request context:
- Treat a first non-empty delta as progress, not proof of completion.
- If more work is needed, make the next revision one implementation slice with a bounded patch goal; include exact_edits only when precision saves supervisor output.
- A useful incomplete active diff may be a baseline candidate before the next slice; apply the session economics policy when choosing worker_mode and patch_decision.
- Otherwise revise from the current worktree and preserve useful existing edits.
- Put checks in deferred_checks until a non-empty patch exists unless artifacts show the edit already exists."#
                .to_string(),
        );
    }

    if signals.worker_turn_shape.as_deref() == Some("patch_request") && signals.latest_delta_empty {
        sections.push(
            r#"No-diff patch-request context:
- The latest changes.patch appears empty.
- If worker artifacts show broad reading, generated-file inspection, tests-before-edit, invalid or unavailable tool churn, or no clear external blocker, treat the prior request as likely too broad or under-anchored.
- A revise must shrink at least one dimension: fewer files, fewer implementation layers, fewer checks, or more concrete anchors/exact_edits.
- Do not resend the same broad patch_request just because the worker reported tool problems."#
                .to_string(),
        );
    }

    if signals.worker_turn_shape.as_deref() == Some("bounded_feature_slice") {
        sections.push(
            r#"Bounded-feature context:
- Treat a useful incomplete patch as progress.
- If more work is needed, ask for the next coherent implementation path rather than one mechanical edit.
- A bounded revision may include related source, API, and focused check work when those edits belong together."#
                .to_string(),
        );
    }

    if signals.context_overflow || signals.context_pressure {
        sections.push(
            r#"Context-pressure context:
- The worker artifacts indicate context overflow or high token pressure.
- If another revision is needed, shrink the next request.
- This is a context_focus-favored signal under the session economics policy."#
                .to_string(),
        );
    }

    if signals.supervisor_control_seen {
        sections.push(
            r#"Live-control context:
- Supervisor control already intervened during a worker turn.
- Judge whether the previous request was too broad, too vague, or stale before issuing another revision.
- Prefer one focused repair or verification step over adding a new feature concern in the same handoff."#
                .to_string(),
        );
    }

    if signals.patch_request_progress_streak {
        sections.push(
            r#"Slice-sizing context:
- Multiple recent patch-request turns produced non-empty deltas without context overflow.
- If the current source state is coherent but incomplete, the next patch_request may cover one larger anchored implementation slice.
- Keep the selected worker profile's preferred shape unless the profile explicitly supports broadening."#
                .to_string(),
        );
    }

    if signals.has_any(&[
        PATCH_COMPARISON,
        PREVIOUS_WORKTREE_PATCH,
        PATCH_BASELINE_JSON,
        BASELINE_ACCEPTED_PATCH,
        BASELINE_ACTIVE_PATCH,
        PATCH_ROLLBACK_JSON,
        ROLLBACK_CURRENT_PATCH,
        ROLLBACK_RESTORED_PATCH,
    ]) {
        sections.push(
            r#"Patch checkpoint context:
- Treat patch-comparison.json as neutral structural telemetry; Mixmod is not judging patch quality.
- Choose patch_decision yourself from the task, current patch, and checkpoint artifacts.
- accept_current_baseline creates an internal checkpoint commit, puts accepted progress in the source tree, clears the active diff for the next turn, and reconstructs the final benchmark patch from the original base.
- revise_previous restores the previous candidate patch before the next worker turn; tell the worker only the focused follow-up edit."#
                .to_string(),
        );
    }

    sections.join("\n\n")
}

fn supervisor_feedback_core_context(signals: &SupervisorFeedbackPromptSignals) -> String {
    let tool_evidence = if signals.has_any(&[TOOL_EVENTS_JSONL]) || signals.tool_events_available {
        "- Use tool-events.jsonl only as command/tool-call evidence when checking worker claims."
    } else {
        "- If command evidence is unavailable, rely on report/review-signals cautiously and revise for verification when important."
    };
    format!(
        r#"Core review contract:
- Prefer latest-turn evidence first: receipt/report/review-signals.json and changes.patch.
- Treat review-signals.json as the routing layer for diagnostics. Do not open full metrics, reasoning traces, tool events, interventions, or full active diff unless a specific use_when case applies.
- worktree.patch is the active current diff; changes.patch is only the latest worker-turn delta. Avoid opening worktree.patch or running broad git diff unless approval, rollback, or integration with prior edits depends on it.
{tool_evidence}
- Minimize supervisor input tokens: do not inspect more artifacts, logs, or diff content once the next action is clear.
- For generated-output diffs, inspect authored-source changes and patch stats first. Avoid opening whole generated files; judge whether generated changes are bounded expected outputs and free of transient tool sidecars.
- Approve only when the current source state appears to satisfy the original task and no worker action or check remains. Before approving, inspect task.json and enough source/diff state to verify completion.
- Treat a false approval as a terminal correctness failure. If approval_contract evidence is missing for the main requested behavior or a likely edge case, choose revise for a targeted verification or repair turn.
- On approve, required_checks and deferred_checks must be empty and completion_gate must be absent or empty.
- Revise when a useful worker path remains; message_to_worker must be concrete and worker-executable.
- Stop only for a blocked or inconclusive worker result when no useful worker path remains.
- The worker owns implementation. Do not author task-solving source changes.
- Put only repo source/test paths in focus_files. Do not ask the worker to inspect Mixmod artifacts.
- Prefer patch_decision for checkpoint control; use direct git restore/apply only for state management, not to create a solution patch."#
    )
}

fn supervisor_feedback_prompt_signals(
    artifact_paths: &[PathBuf],
) -> SupervisorFeedbackPromptSignals {
    let mut signals = SupervisorFeedbackPromptSignals::default();
    for path in artifact_paths {
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        signals.artifact_names.insert(name.to_string());
        match name {
            REVIEW_SIGNALS_JSON => signals.update_from_review_signals(path),
            METRICS_JSON => signals.update_from_metrics(path),
            SUPERVISION_LOOP_SUMMARY_JSON => signals.update_from_loop_summary(path),
            WORKER_BRIEF_JSON => signals.update_from_worker_brief(path),
            _ => {}
        }
        if name == CHANGES_PATCH {
            signals.latest_delta_empty |= fs::metadata(path)
                .map(|metadata| metadata.len() == 0)
                .unwrap_or(false);
        }
    }
    signals
}

impl SupervisorFeedbackPromptSignals {
    fn has_any(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.artifact_names.contains(*name))
    }

    fn set_worker_turn_shape(&mut self, value: Option<&str>) {
        if self.worker_turn_shape.is_none()
            && let Some(value) = value.filter(|value| !value.trim().is_empty())
        {
            self.worker_turn_shape = Some(value.to_string());
        }
    }

    fn update_from_metrics(&mut self, path: &Path) {
        let Ok(metrics) = read_json_file(path) else {
            return;
        };
        self.context_overflow |= get_u64(&metrics, "context_overflow_count").unwrap_or(0) > 0;
        self.context_pressure |=
            get_u64(&metrics, "worker_session_token_peak").is_some_and(|tokens| tokens >= 24_000);
        self.supervisor_control_seen |= get_bool(&metrics, "interrupted_by_supervisor")
            .unwrap_or(false)
            || metrics
                .get("supervisor_control_events")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
    }

    fn update_from_review_signals(&mut self, path: &Path) {
        let Ok(signals) = read_json_file(path) else {
            return;
        };
        self.context_overflow |= get_u64(&signals, "context_overflow_count").unwrap_or(0) > 0;
        self.context_pressure |=
            get_u64(&signals, "worker_session_token_peak").is_some_and(|tokens| tokens >= 24_000);
        self.supervisor_control_seen |= get_bool(&signals, "interrupted_by_supervisor")
            .unwrap_or(false)
            || get_str(&signals, "supervisor_control_action")
                .is_some_and(|action| !action.trim().is_empty() && action.trim() != "none");
        self.tool_events_available |= get_u64(&signals, "tool_event_count").unwrap_or(0) > 0;
    }

    fn update_from_loop_summary(&mut self, path: &Path) {
        let Ok(summary) = read_json_file(path) else {
            return;
        };
        self.context_overflow |= get_u64(&summary, "context_overflow_total").unwrap_or(0) > 0;
        self.context_pressure |= get_u64(&summary, "worker_session_token_peak_max")
            .is_some_and(|tokens| tokens >= 24_000);
        self.supervisor_control_seen |=
            get_u64(&summary, "supervisor_control_count").unwrap_or(0) > 0;
        self.patch_request_progress_streak |=
            get_u64(&summary, "patch_request_nonempty_delta_streak").unwrap_or(0) >= 2;

        if let Some(turns) = summary.get("turns").and_then(Value::as_array)
            && let Some(last) = turns.last()
        {
            self.set_worker_turn_shape(get_str(last, "worker_turn_shape"));
            self.context_overflow |= get_u64(last, "context_overflow_count").unwrap_or(0) > 0;
            self.context_pressure |=
                get_u64(last, "worker_session_token_peak").is_some_and(|tokens| tokens >= 24_000);
        }
    }

    fn update_from_worker_brief(&mut self, path: &Path) {
        let Ok(brief) = read_json_file(path) else {
            return;
        };
        self.set_worker_turn_shape(get_str(&brief, "worker_turn_shape"));
    }
}
