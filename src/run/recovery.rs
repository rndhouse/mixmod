use crate::*;

use super::context::worker_context_signals;
use super::prompts::{
    build_empty_patch_followup_instruction, build_revision_noop_followup_instruction,
    build_worker_self_review_instruction, revision_focus_files,
};

const WORKER_SELF_REVIEW_TOKEN_PEAK_LIMIT: u64 = 24_000;
const WORKER_SELF_REVIEW_CHANGED_LINE_LIMIT: usize = 500;

#[derive(Debug)]
pub(super) struct EmptyPatchFollowup {
    pub(super) triggered: bool,
    pub(super) performed: bool,
    pub(super) patch_created: bool,
    pub(super) reason: Option<String>,
    pub(super) run_dir: Option<String>,
}

impl EmptyPatchFollowup {
    pub(super) fn new() -> Self {
        Self {
            triggered: false,
            performed: false,
            patch_created: false,
            reason: None,
            run_dir: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct RevisionNoopFollowup {
    pub(super) delta_expected: bool,
    pub(super) triggered: bool,
    pub(super) performed: bool,
    pub(super) patch_created: bool,
    pub(super) reason: Option<String>,
    pub(super) run_dir: Option<String>,
}

impl RevisionNoopFollowup {
    pub(super) fn new(delta_expected: bool) -> Self {
        Self {
            delta_expected,
            triggered: false,
            performed: false,
            patch_created: false,
            reason: None,
            run_dir: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct WorkerSelfReview {
    pub(super) enabled: bool,
    pub(super) triggered: bool,
    pub(super) performed: bool,
    pub(super) patch_changed: bool,
    pub(super) reason: Option<String>,
    pub(super) run_dir: Option<String>,
}

impl WorkerSelfReview {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            triggered: false,
            performed: false,
            patch_changed: false,
            reason: None,
            run_dir: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct RevisionNoopContext {
    pub(super) delta_expected: bool,
    pub(super) message_to_worker: String,
    pub(super) revision_handoff: RevisionHandoff,
    pub(super) focus_files: Vec<String>,
    pub(super) required_checks: Vec<String>,
    pub(super) worker_mode: String,
    pub(super) patch_decision: String,
}

impl RevisionNoopContext {
    pub(super) fn from_task(task: &Value) -> Option<Self> {
        let revision = task.get("context")?.get("revision")?;
        let delta_expected = get_bool(revision, "delta_expected").unwrap_or_else(|| {
            let patch_decision = get_str(revision, "patch_decision").unwrap_or("");
            matches!(patch_decision, "revise_current" | "revise_previous")
        });
        if !delta_expected {
            return None;
        }
        Some(Self {
            delta_expected,
            message_to_worker: get_str(revision, "message_to_worker")
                .unwrap_or("")
                .trim()
                .to_string(),
            revision_handoff: RevisionHandoff {
                expect_patch: get_bool(revision, "expect_patch"),
                worker_turn_shape: get_str(revision, "worker_turn_shape").map(ToOwned::to_owned),
                turn_goal: get_str(revision, "turn_goal").map(ToOwned::to_owned),
                exact_edits: get_string_array(revision, "exact_edits"),
                edit_plan: get_string_array(revision, "edit_plan"),
                deferred_checks: get_string_array(revision, "deferred_checks"),
                defer_checks_until_patch_exists: get_bool(
                    revision,
                    "defer_checks_until_patch_exists",
                ),
                completion_gate: get_str(revision, "completion_gate").map(ToOwned::to_owned),
                forbidden_actions: get_string_array(revision, "forbidden_actions"),
            },
            focus_files: get_string_array(revision, "focus_files"),
            required_checks: get_string_array(revision, "required_checks"),
            worker_mode: get_str(revision, "worker_mode")
                .unwrap_or("continue")
                .trim()
                .to_string(),
            patch_decision: get_str(revision, "patch_decision")
                .unwrap_or("")
                .trim()
                .to_string(),
        })
    }
}

pub(super) struct RevisionNoopFollowupRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) mode: DelegationMode,
    pub(super) task: &'a TaskSpec,
    pub(super) task_path: &'a Path,
    pub(super) out_dir: &'a Path,
    pub(super) runner: &'a dyn AgentHarness,
    pub(super) require_local: bool,
    pub(super) original_request: &'a AgentRequest,
    pub(super) output: &'a AgentOutput,
    pub(super) revision: &'a RevisionNoopContext,
}

pub(super) fn run_revision_noop_followup(
    request: RevisionNoopFollowupRequest<'_>,
) -> Result<(AgentOutput, PathBuf)> {
    let RevisionNoopFollowupRequest {
        root,
        mode,
        task,
        task_path,
        out_dir,
        runner,
        require_local,
        original_request,
        output,
        revision,
    } = request;
    let resume_session_id = output.session_id.clone().ok_or_else(|| {
        anyhow!("cannot run revision no-op follow-up without a worker session id")
    })?;
    let followup_dir = out_dir.join("revision-noop-followup");
    fs::create_dir_all(&followup_dir).with_context(|| {
        format!(
            "failed to create revision no-op follow-up dir {}",
            followup_dir.display()
        )
    })?;
    let patch_request = revision.revision_handoff.is_patch_request();
    let acceptance = if patch_request {
        revision
            .revision_handoff
            .completion_gate
            .as_deref()
            .map(str::trim)
            .filter(|gate| !gate.is_empty())
            .map(|gate| vec![gate.to_string()])
            .unwrap_or_default()
    } else {
        vec![
            "Make a new repository diff relative to the previous candidate patch, or return BLOCKED with a concrete reason."
                .to_string(),
        ]
    };
    let followup_task = json!({
        "title": format!("Revision no-op follow-up: {}", task.title),
        "expect_patch": true,
        "instructions": "The previous revision worker turn exited successfully, but Mixmod captured no new delta against the existing candidate patch.",
        "files": revision_focus_files(task, revision),
        "tests": if patch_request { Vec::<String>::new() } else { revision.required_checks.clone() },
        "constraints": [
            "Do not only inspect files.",
            "Apply the exact supervisor revision or report BLOCKED."
        ],
        "acceptance": acceptance,
        "context": {
            "source_task": task_path.to_string_lossy(),
            "revision_noop_followup": true,
            "revision": {
                "message_to_worker": revision.message_to_worker,
                "worker_mode": revision.worker_mode,
                "patch_decision": revision.patch_decision,
                "worker_turn_shape": revision.revision_handoff.worker_turn_shape,
                "turn_goal": revision.revision_handoff.turn_goal,
                "exact_edits": revision.revision_handoff.exact_edits,
                "deferred_checks": revision.revision_handoff.deferred_checks,
                "defer_checks_until_patch_exists": revision.revision_handoff.defer_checks_until_patch_exists,
                "completion_gate": revision.revision_handoff.completion_gate,
                "forbidden_actions": revision.revision_handoff.forbidden_actions,
                "focus_files": revision.focus_files,
                "required_checks": revision.required_checks
            }
        }
    });
    let followup_task_path = followup_dir.join(TASK_JSON);
    write_pretty_json(
        &followup_task_path,
        &followup_task,
        "revision no-op follow-up task",
    )?;

    let instruction = build_revision_noop_followup_instruction(mode, task, revision);
    let instruction_path = followup_dir.join(OPENCODE_INSTRUCTIONS_MD);
    atomic_write(&instruction_path, instruction.as_bytes())?;
    let followup_request = AgentRequest {
        root: root.to_path_buf(),
        mode,
        task_path: followup_task_path,
        out_dir: followup_dir.clone(),
        instruction_path,
        instruction,
        session_id: original_request.session_id.clone(),
        resume_session_id: Some(resume_session_id),
        require_local,
        supervisor_advisor: original_request.supervisor_advisor.clone(),
    };
    let followup_output = runner.run(&followup_request)?;
    Ok((followup_output, followup_dir))
}

pub(super) struct EmptyPatchFollowupRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) mode: DelegationMode,
    pub(super) task: &'a TaskSpec,
    pub(super) task_path: &'a Path,
    pub(super) out_dir: &'a Path,
    pub(super) runner: &'a dyn AgentHarness,
    pub(super) require_local: bool,
    pub(super) original_request: &'a AgentRequest,
    pub(super) output: &'a AgentOutput,
}

pub(super) fn run_empty_patch_followup(
    request: EmptyPatchFollowupRequest<'_>,
) -> Result<(AgentOutput, PathBuf)> {
    let EmptyPatchFollowupRequest {
        root,
        mode,
        task,
        task_path,
        out_dir,
        runner,
        require_local,
        original_request,
        output,
    } = request;
    let resume_session_id = output
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("cannot run empty-patch follow-up without a worker session id"))?;
    let followup_dir = out_dir.join("empty-patch-followup");
    fs::create_dir_all(&followup_dir).with_context(|| {
        format!(
            "failed to create empty-patch follow-up dir {}",
            followup_dir.display()
        )
    })?;
    let followup_task = json!({
        "title": format!("Empty-patch follow-up: {}", task.title),
        "expect_patch": true,
        "instructions": "The previous local-worker run exited successfully, but Mixmod captured no repository diff.",
        "files": &task.files,
        "tests": &task.tests,
        "constraints": [
            "Do not restart broad exploration.",
            "Resolve the empty-patch mismatch compactly."
        ],
        "acceptance": [
            "Either make the intended edits, explain why no patch is needed, or explain the blocker."
        ],
        "context": {
            "source_task": task_path.to_string_lossy(),
            "empty_patch_followup": true
        }
    });
    let followup_task_path = followup_dir.join(TASK_JSON);
    write_pretty_json(
        &followup_task_path,
        &followup_task,
        "empty-patch follow-up task",
    )?;

    let instruction = build_empty_patch_followup_instruction(mode, task, task_path, &followup_dir);
    let instruction_path = followup_dir.join(OPENCODE_INSTRUCTIONS_MD);
    atomic_write(&instruction_path, instruction.as_bytes())?;
    let followup_request = AgentRequest {
        root: root.to_path_buf(),
        mode,
        task_path: followup_task_path,
        out_dir: followup_dir.clone(),
        instruction_path,
        instruction,
        session_id: original_request.session_id.clone(),
        resume_session_id: Some(resume_session_id),
        require_local,
        supervisor_advisor: original_request.supervisor_advisor.clone(),
    };
    let followup_output = runner.run(&followup_request)?;
    Ok((followup_output, followup_dir))
}

pub(super) struct WorkerSelfReviewRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) mode: DelegationMode,
    pub(super) task: &'a TaskSpec,
    pub(super) task_path: &'a Path,
    pub(super) out_dir: &'a Path,
    pub(super) runner: &'a dyn AgentHarness,
    pub(super) require_local: bool,
    pub(super) original_request: &'a AgentRequest,
    pub(super) output: &'a AgentOutput,
}

pub(super) fn run_worker_self_review(
    request: WorkerSelfReviewRequest<'_>,
) -> Result<(AgentOutput, PathBuf)> {
    let WorkerSelfReviewRequest {
        root,
        mode,
        task,
        task_path,
        out_dir,
        runner,
        require_local,
        original_request,
        output,
    } = request;
    let resume_session_id = output
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("cannot run worker self-review without a worker session id"))?;
    let review_dir = out_dir.join("worker-self-review");
    fs::create_dir_all(&review_dir).with_context(|| {
        format!(
            "failed to create worker self-review dir {}",
            review_dir.display()
        )
    })?;
    let review_task = json!({
        "title": format!("Worker self-review cleanup: {}", task.title),
        "expect_patch": true,
        "instructions": "Review the current worker diff for obvious cleanup only before supervisor review.",
        "files": &task.files,
        "tests": Vec::<String>::new(),
        "constraints": [
            "Do not add new feature scope.",
            "Do not rewrite the solution.",
            "Do not commit."
        ],
        "acceptance": [
            "Either apply a small safe cleanup to the current diff, or leave the worktree unchanged and report no cleanup."
        ],
        "context": {
            "source_task": task_path.to_string_lossy(),
            "worker_self_review": true
        }
    });
    let review_task_path = review_dir.join(TASK_JSON);
    write_pretty_json(&review_task_path, &review_task, "worker self-review task")?;

    let instruction = build_worker_self_review_instruction(mode, task);
    let instruction_path = review_dir.join(OPENCODE_INSTRUCTIONS_MD);
    atomic_write(&instruction_path, instruction.as_bytes())?;
    let review_request = AgentRequest {
        root: root.to_path_buf(),
        mode,
        task_path: review_task_path,
        out_dir: review_dir.clone(),
        instruction_path,
        instruction,
        session_id: original_request.session_id.clone(),
        resume_session_id: Some(resume_session_id),
        require_local,
        supervisor_advisor: original_request.supervisor_advisor.clone(),
    };
    let review_output = runner.run(&review_request)?;
    Ok((review_output, review_dir))
}

pub(super) fn should_run_empty_patch_followup(
    mode: DelegationMode,
    expect_patch: bool,
    output: &AgentOutput,
    patch: &str,
) -> bool {
    mode == DelegationMode::Patch
        && expect_patch
        && output.success
        && !output.timed_out
        && !output.idle_timed_out
        && !output.interrupted_by_supervisor
        && patch.trim().is_empty()
}

pub(super) fn should_run_revision_noop_followup(
    mode: DelegationMode,
    expect_patch: bool,
    revision: Option<&RevisionNoopContext>,
    output: &AgentOutput,
    patch: &str,
) -> bool {
    mode == DelegationMode::Patch
        && expect_patch
        && revision.is_some_and(|context| context.delta_expected)
        && output.success
        && !output.timed_out
        && !output.idle_timed_out
        && !output.interrupted_by_supervisor
        && patch.trim().is_empty()
}

pub(super) fn worker_self_review_skip_reason(
    enabled: bool,
    mode: DelegationMode,
    expect_patch: bool,
    output: &AgentOutput,
    patch: &str,
) -> Option<String> {
    if !enabled {
        return Some("disabled".to_string());
    }
    if mode != DelegationMode::Patch {
        return Some("mode_not_patch".to_string());
    }
    if !expect_patch {
        return Some("patch_not_expected".to_string());
    }
    if !output.success {
        return Some("worker_failed".to_string());
    }
    if output.timed_out {
        return Some("worker_timed_out".to_string());
    }
    if output.idle_timed_out {
        return Some("worker_idle_timed_out".to_string());
    }
    if output.interrupted_by_supervisor {
        return Some("worker_interrupted_by_supervisor".to_string());
    }
    if output.session_id.is_none() {
        return Some("missing_worker_session_id".to_string());
    }
    if patch.trim().is_empty() {
        return Some("empty_patch".to_string());
    }
    let context = worker_context_signals(&output.stdout);
    if context.context_overflow_count > 0 {
        return Some("context_overflow_observed".to_string());
    }
    if worker_session_token_peak(&output.stdout)
        .is_some_and(|tokens| tokens >= WORKER_SELF_REVIEW_TOKEN_PEAK_LIMIT)
    {
        return Some("worker_context_token_peak_high".to_string());
    }
    let changed_lines = patch_stats(patch).changed_line_count;
    if changed_lines > WORKER_SELF_REVIEW_CHANGED_LINE_LIMIT {
        return Some("patch_too_large_for_self_review".to_string());
    }
    None
}

pub(super) fn merge_worker_outputs(
    mut first: AgentOutput,
    second: AgentOutput,
    label: &str,
    note: &str,
) -> AgentOutput {
    let marker = format!("<{}>", label.replace(' ', "-"));
    let stdout_header = format!("\n\n--- {label} stdout ---\n");
    let stderr_header = format!("\n\n--- {label} stderr ---\n");

    first.command_for_metrics.push(marker);
    first
        .command_for_metrics
        .extend(second.command_for_metrics.clone());
    first.segments.extend(second.segments.clone());
    first.exit_status = second.exit_status;
    first.success = second.success;
    first.stdout.extend_from_slice(stdout_header.as_bytes());
    first.stdout.extend_from_slice(&second.stdout);
    first.stderr.extend_from_slice(stderr_header.as_bytes());
    first.stderr.extend_from_slice(&second.stderr);
    first.session_id = second.session_id.or(first.session_id);
    first.resume_session_id = second.resume_session_id.or(first.resume_session_id);
    first.session_reused = first.session_reused || second.session_reused;
    first.interrupted_by_supervisor =
        first.interrupted_by_supervisor || second.interrupted_by_supervisor;
    first.supervisor_control_action = second
        .supervisor_control_action
        .or(first.supervisor_control_action);
    first
        .supervisor_control_events
        .extend(second.supervisor_control_events);
    first.timed_out = first.timed_out || second.timed_out;
    first.idle_timed_out = first.idle_timed_out || second.idle_timed_out;
    first.heartbeat_count += second.heartbeat_count;
    first.require_local = first.require_local || second.require_local;
    first.local_inference_verified = if first.require_local {
        first.local_inference_verified && second.local_inference_verified
    } else {
        first.local_inference_verified || second.local_inference_verified
    };
    first.gpu_activity_observed = first.gpu_activity_observed || second.gpu_activity_observed;
    first.backend_activity_observed =
        first.backend_activity_observed || second.backend_activity_observed;
    first.verification_notes.push(note.to_string());
    first.verification_notes.extend(second.verification_notes);
    first
}
