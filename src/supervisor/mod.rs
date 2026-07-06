use std::collections::BTreeSet;

use crate::harness::codex::{CodexAppServer, CodexSandbox, CodexTurnResult};
use crate::*;

mod prompts;

pub(crate) use prompts::{
    codex_only_prompt, supervisor_feedback_prompt, supervisor_feedback_repair_prompt,
    supervisor_feedback_repair_retry_prompt, supervisor_worker_brief_prompt,
    supervisor_worker_brief_repair_prompt, supervisor_worker_brief_repair_retry_prompt,
};

pub(crate) struct SupervisorFeedbackTurn {
    pub(crate) feedback: Value,
    pub(crate) verdict: String,
    pub(crate) worker_mode: String,
    pub(crate) patch_decision: String,
    pub(crate) hint: String,
    pub(crate) revision_handoff: RevisionHandoff,
    pub(crate) focus_files: Vec<String>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RevisionHandoff {
    pub(crate) worker_turn_shape: Option<String>,
    pub(crate) turn_goal: Option<String>,
    pub(crate) exact_edits: Vec<String>,
    pub(crate) deferred_checks: Vec<String>,
    pub(crate) defer_checks_until_patch_exists: Option<bool>,
    pub(crate) completion_gate: Option<String>,
    pub(crate) forbidden_actions: Vec<String>,
}

impl RevisionHandoff {
    pub(crate) fn from_feedback(feedback: &SupervisorFeedback) -> Self {
        Self {
            worker_turn_shape: feedback.worker_turn_shape.clone(),
            turn_goal: feedback.turn_goal.clone(),
            exact_edits: feedback.exact_edits.clone(),
            deferred_checks: feedback.deferred_checks.clone(),
            defer_checks_until_patch_exists: feedback.defer_checks_until_patch_exists,
            completion_gate: feedback.completion_gate.clone(),
            forbidden_actions: feedback.forbidden_actions.clone(),
        }
    }

    pub(crate) fn is_small_patch_slice(&self) -> bool {
        self.worker_turn_shape
            .as_deref()
            .is_some_and(|shape| shape.trim() == "small_patch_slice")
    }
}

#[derive(Debug)]
pub(crate) struct SupervisorBriefTurn {
    pub(crate) record: Value,
    pub(crate) brief: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Clone)]
pub(crate) struct SupervisorUsageSample {
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    input_bytes: u64,
    output_bytes: u64,
    thread_id: String,
    turn_id: String,
}

impl SupervisorFeedbackTurn {
    pub(crate) fn usage_sample(&self) -> SupervisorUsageSample {
        SupervisorUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

impl SupervisorBriefTurn {
    pub(crate) fn usage_sample(&self) -> SupervisorUsageSample {
        SupervisorUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct SupervisorUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) turn_count: u64,
    pub(crate) thread_ids: Vec<String>,
    pub(crate) turn_ids: Vec<String>,
}

impl SupervisorUsage {
    pub(crate) fn thread_count(&self) -> u64 {
        self.thread_ids
            .iter()
            .filter(|id| !id.is_empty())
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .len() as u64
    }

    pub(crate) fn thread_reuse_count(&self) -> u64 {
        let observed_thread_ids = self.thread_ids.iter().filter(|id| !id.is_empty()).count() as u64;
        let thread_count = self.thread_count();
        observed_thread_ids.saturating_sub(thread_count)
    }

    pub(crate) fn session_reused(&self) -> bool {
        self.thread_reuse_count() > 0
    }
}

pub(crate) fn run_supervisor_brief_turn(
    work_dir: &Path,
    default_dir: &Path,
    task_path: &Path,
    supervisor: &SupervisorConfig,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
) -> Result<SupervisorBriefTurn> {
    let prompt = supervisor_worker_brief_prompt(work_dir, task_path, worker_guidance, init_mode)?;
    let result = run_codex_app_server_turn(
        work_dir,
        default_dir,
        "worker-brief",
        &prompt,
        supervisor,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_brief = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "handoff": "blocked",
            "message_to_worker": "The supervisor did not return parseable handoff JSON.",
            "risk": truncate_for_report(&result.last_message, 160)
        })
    });
    let mut final_brief = parsed_brief;
    let mut repair_record = None;
    let mut input_tokens = result.usage.input_tokens;
    let mut output_tokens = result.usage.output_tokens;
    let mut reasoning_tokens = result.usage.reasoning_tokens;
    let mut total_tokens = result.usage.total_tokens;
    let mut cached_input_tokens = result.usage.cached_input_tokens;
    let mut input_bytes = result.input_bytes;
    let mut output_bytes = result.output_bytes;
    let mut thread_id = result.thread_id.clone();
    let mut turn_id = result.turn_id.clone();
    if worker_brief_needs_small_slice_repair(&final_brief, worker_guidance) {
        let repair_prompt = supervisor_worker_brief_repair_prompt(
            work_dir,
            task_path,
            worker_guidance,
            &final_brief,
        )?;
        let repair = run_codex_app_server_turn(
            work_dir,
            default_dir,
            "worker-brief-repair",
            &repair_prompt,
            supervisor,
            CodexSandbox::ReadOnly,
        )?;
        let mut repaired_brief = parse_feedback_json(&repair.last_message);
        let mut repair_accepted =
            repaired_brief_is_accepted(repaired_brief.as_ref(), worker_guidance);
        let mut retry_record = None;
        if !repair_accepted {
            let rejection_reason =
                worker_brief_repair_rejection_reason(repaired_brief.as_ref(), worker_guidance);
            let rejected_repair = repaired_brief.clone().unwrap_or_else(|| {
                json!({
                    "unparseable": truncate_for_report(&repair.last_message, 240)
                })
            });
            let retry_prompt = supervisor_worker_brief_repair_retry_prompt(
                work_dir,
                task_path,
                worker_guidance,
                &final_brief,
                &rejected_repair,
                &rejection_reason,
            )?;
            let retry = run_codex_app_server_turn(
                work_dir,
                default_dir,
                "worker-brief-repair-2",
                &retry_prompt,
                supervisor,
                CodexSandbox::ReadOnly,
            )?;
            let retry_brief = parse_feedback_json(&retry.last_message);
            let retry_accepted = repaired_brief_is_accepted(retry_brief.as_ref(), worker_guidance);
            retry_record = Some(json!({
                "label": "worker-brief-repair-2",
                "trigger": "previous worker-brief repair was rejected",
                "rejection_reason": rejection_reason,
                "accepted": retry_accepted,
                "codex_exit_status": retry.exit_status,
                "supervisor_model": retry.model.clone(),
                "supervisor_reasoning_effort": retry.reasoning_effort.clone(),
                "supervisor_input_tokens": retry.usage.input_tokens,
                "supervisor_output_tokens": retry.usage.output_tokens,
                "supervisor_reasoning_tokens": retry.usage.reasoning_tokens,
                "supervisor_total_tokens": retry.usage.total_tokens,
                "supervisor_cached_input_tokens": retry.usage.cached_input_tokens,
                "input_bytes": retry.input_bytes,
                "output_bytes": retry.output_bytes,
                "codex_app_server_thread_id": retry.thread_id.clone(),
                "codex_app_server_turn_id": retry.turn_id.clone(),
            }));
            if retry_accepted {
                repaired_brief = retry_brief;
                repair_accepted = true;
            }
            input_tokens += retry.usage.input_tokens;
            output_tokens += retry.usage.output_tokens;
            reasoning_tokens += retry.usage.reasoning_tokens;
            total_tokens += retry.usage.total_tokens;
            cached_input_tokens += retry.usage.cached_input_tokens;
            input_bytes += retry.input_bytes;
            output_bytes += retry.output_bytes;
            thread_id = retry.thread_id;
            turn_id = retry.turn_id;
        }
        let retry_ran = retry_record.is_some();
        repair_record = Some(json!({
            "label": "worker-brief-repair",
            "trigger": "expected-patch handoff for selected worker was missing or broad small_patch_slice",
            "accepted": repair_accepted,
            "retry": retry_record,
            "codex_exit_status": repair.exit_status,
            "supervisor_model": repair.model.clone(),
            "supervisor_reasoning_effort": repair.reasoning_effort.clone(),
            "supervisor_input_tokens": repair.usage.input_tokens,
            "supervisor_output_tokens": repair.usage.output_tokens,
            "supervisor_reasoning_tokens": repair.usage.reasoning_tokens,
            "supervisor_total_tokens": repair.usage.total_tokens,
            "supervisor_cached_input_tokens": repair.usage.cached_input_tokens,
            "input_bytes": repair.input_bytes,
            "output_bytes": repair.output_bytes,
            "codex_app_server_thread_id": repair.thread_id.clone(),
            "codex_app_server_turn_id": repair.turn_id.clone(),
        }));
        if repair_accepted && let Some(repaired_brief) = repaired_brief {
            final_brief = repaired_brief;
        }
        input_tokens += repair.usage.input_tokens;
        output_tokens += repair.usage.output_tokens;
        reasoning_tokens += repair.usage.reasoning_tokens;
        total_tokens += repair.usage.total_tokens;
        cached_input_tokens += repair.usage.cached_input_tokens;
        input_bytes += repair.input_bytes;
        output_bytes += repair.output_bytes;
        if !retry_ran {
            thread_id = repair.thread_id;
            turn_id = repair.turn_id;
        }
    }
    let record = json!({
        "label": "worker-brief",
        "timestamp": Utc::now().to_rfc3339(),
        "supervisor_init": init_mode.as_str(),
        "brief": final_brief,
        "repair": repair_record,
        "codex_exit_status": result.exit_status,
        "supervisor_model": result.model.clone(),
        "supervisor_reasoning_effort": result.reasoning_effort.clone(),
        "supervisor_input_tokens": input_tokens,
        "supervisor_output_tokens": output_tokens,
        "supervisor_reasoning_tokens": reasoning_tokens,
        "supervisor_total_tokens": total_tokens,
        "supervisor_cached_input_tokens": cached_input_tokens,
        "input_bytes": input_bytes,
        "output_bytes": output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": thread_id.clone(),
        "codex_app_server_turn_id": turn_id.clone()
    });
    Ok(SupervisorBriefTurn {
        record,
        brief: final_brief,
        input_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
        cached_input_tokens,
        input_bytes,
        output_bytes,
        thread_id,
        turn_id,
    })
}

fn worker_brief_needs_small_slice_repair(
    brief: &Value,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    if !worker_guidance
        .guidance
        .iter()
        .any(|item| item.contains("worker_turn_shape=small_patch_slice"))
    {
        return false;
    }
    let typed = WorkerBrief::from_value(brief);
    let handoff = typed.handoff.as_deref().unwrap_or("guided");
    if handoff == "blocked" {
        return false;
    }
    let expect_patch = typed.expect_patch.unwrap_or(handoff != "as_given");
    let small_patch_slice = typed
        .worker_turn_shape
        .as_deref()
        .is_some_and(|shape| shape.trim() == "small_patch_slice");
    if !expect_patch {
        return false;
    }
    if !small_patch_slice {
        return true;
    }
    if typed.exact_edits.len() > 2 {
        return true;
    }
    let Some(first_edit) = typed
        .exact_edits
        .iter()
        .find(|edit| !edit.trim().is_empty())
    else {
        return true;
    };
    first_revision_edit_is_too_broad(first_edit)
}

fn supervisor_feedback_needs_revision_slice_repair(
    feedback: &Value,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    if !worker_guidance.guidance.iter().any(|item| {
        item.contains("worker_turn_shape=small_patch_slice")
            || item.contains("one immediate source edit")
    }) {
        return false;
    }
    let raw_verdict = get_str(feedback, "verdict")
        .or_else(|| get_str(feedback, "action"))
        .unwrap_or("revise");
    if normalize_supervisor_verdict(raw_verdict) != "revise" {
        return false;
    }
    let typed = SupervisorFeedback::from_value(feedback);
    let handoff = RevisionHandoff::from_feedback(&typed);
    if !handoff.is_small_patch_slice() {
        return false;
    }
    let Some(first_edit) = handoff
        .exact_edits
        .iter()
        .find(|edit| !edit.trim().is_empty())
    else {
        return true;
    };
    first_revision_edit_is_too_broad(first_edit)
}

fn first_revision_edit_is_too_broad(edit: &str) -> bool {
    let actionable = actionable_revision_edit_text(edit);
    let trimmed = actionable.trim();
    if trimmed.len() > 320 {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    let broad_pairs = [
        ("pack", "unpack"),
        ("serializ", "deserializ"),
        ("parse", "emit"),
        ("validate", "convert"),
        ("prefix", "rename"),
        ("read", "write"),
    ];
    broad_pairs.iter().any(|(left, right)| {
        contains_broad_term(&lower, left) && contains_broad_term(&lower, right)
    })
}

fn actionable_revision_edit_text(edit: &str) -> String {
    edit.replace(". Do not ", "; Do not ")
        .replace(". do not ", "; do not ")
        .split(';')
        .map(str::trim)
        .filter(|clause| {
            let lower = clause.to_ascii_lowercase();
            !lower.starts_with("do not ")
                && !lower.starts_with("don't ")
                && !lower.starts_with("no ")
                && !lower.starts_with("avoid ")
                && !lower.starts_with("without ")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn contains_broad_term(text: &str, needle: &str) -> bool {
    let mut search_start = 0;
    while let Some(relative_index) = text[search_start..].find(needle) {
        let index = search_start + relative_index;
        let before = text[..index].chars().next_back();
        if before.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_') {
            return true;
        }
        search_start = index + needle.len();
    }
    false
}

pub(crate) fn run_supervisor_feedback_turn(
    work_dir: &Path,
    budgeted_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    instruction: &str,
    supervisor: &SupervisorConfig,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<SupervisorFeedbackTurn> {
    let prompt =
        supervisor_feedback_prompt(work_dir, artifact_paths, instruction, worker_guidance)?;
    let result = run_codex_app_server_turn(
        work_dir,
        budgeted_dir,
        label,
        &prompt,
        supervisor,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_feedback = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "action": if result.exit_status == Some(0) { "approve" } else { "revise" },
            "worker_mode": "continue",
            "message_to_worker": truncate_for_report(&result.last_message, 180),
            "focus_files": [],
            "required_checks": [],
            "risk": if result.exit_status == Some(0) { "none recorded" } else { "codex feedback command failed" }
        })
    });
    let (mut parsed_feedback, _) = normalize_feedback_value(parsed_feedback);
    let mut repair_record = None;
    let mut input_tokens = result.usage.input_tokens;
    let mut output_tokens = result.usage.output_tokens;
    let mut reasoning_tokens = result.usage.reasoning_tokens;
    let mut total_tokens = result.usage.total_tokens;
    let mut cached_input_tokens = result.usage.cached_input_tokens;
    let mut input_bytes = result.input_bytes;
    let mut output_bytes = result.output_bytes;
    let mut thread_id = result.thread_id.clone();
    let mut turn_id = result.turn_id.clone();
    if supervisor_feedback_needs_revision_slice_repair(&parsed_feedback, worker_guidance) {
        let repair_prompt = supervisor_feedback_repair_prompt(
            work_dir,
            artifact_paths,
            worker_guidance,
            &parsed_feedback,
        )?;
        let repair = run_codex_app_server_turn(
            work_dir,
            budgeted_dir,
            &format!("{label}-repair"),
            &repair_prompt,
            supervisor,
            CodexSandbox::ReadOnly,
        )?;
        let mut repaired_feedback = parse_feedback_json(&repair.last_message)
            .map(|feedback| normalize_feedback_value(feedback).0);
        let mut repair_accepted = repaired_feedback_is_accepted(
            &parsed_feedback,
            repaired_feedback.as_ref(),
            worker_guidance,
        );
        let mut retry_record = None;
        if !repair_accepted {
            let rejection_reason = supervisor_feedback_repair_rejection_reason(
                &parsed_feedback,
                repaired_feedback.as_ref(),
                worker_guidance,
            );
            let rejected_repair = repaired_feedback.clone().unwrap_or_else(|| {
                json!({
                    "unparseable": truncate_for_report(&repair.last_message, 240)
                })
            });
            let retry_prompt = supervisor_feedback_repair_retry_prompt(
                work_dir,
                artifact_paths,
                worker_guidance,
                &parsed_feedback,
                &rejected_repair,
                &rejection_reason,
            )?;
            let retry = run_codex_app_server_turn(
                work_dir,
                budgeted_dir,
                &format!("{label}-repair-2"),
                &retry_prompt,
                supervisor,
                CodexSandbox::ReadOnly,
            )?;
            let retry_feedback = parse_feedback_json(&retry.last_message)
                .map(|feedback| normalize_feedback_value(feedback).0);
            let retry_accepted = repaired_feedback_is_accepted(
                &parsed_feedback,
                retry_feedback.as_ref(),
                worker_guidance,
            );
            retry_record = Some(json!({
                "label": format!("{label}-repair-2"),
                "trigger": "previous revision repair was rejected",
                "rejection_reason": rejection_reason,
                "accepted": retry_accepted,
                "codex_exit_status": retry.exit_status,
                "supervisor_model": retry.model.clone(),
                "supervisor_reasoning_effort": retry.reasoning_effort.clone(),
                "supervisor_input_tokens": retry.usage.input_tokens,
                "supervisor_output_tokens": retry.usage.output_tokens,
                "supervisor_reasoning_tokens": retry.usage.reasoning_tokens,
                "supervisor_total_tokens": retry.usage.total_tokens,
                "supervisor_cached_input_tokens": retry.usage.cached_input_tokens,
                "input_bytes": retry.input_bytes,
                "output_bytes": retry.output_bytes,
                "codex_app_server_thread_id": retry.thread_id.clone(),
                "codex_app_server_turn_id": retry.turn_id.clone(),
            }));
            if retry_accepted {
                repaired_feedback = retry_feedback;
                repair_accepted = true;
            }
            input_tokens += retry.usage.input_tokens;
            output_tokens += retry.usage.output_tokens;
            reasoning_tokens += retry.usage.reasoning_tokens;
            total_tokens += retry.usage.total_tokens;
            cached_input_tokens += retry.usage.cached_input_tokens;
            input_bytes += retry.input_bytes;
            output_bytes += retry.output_bytes;
            thread_id = retry.thread_id;
            turn_id = retry.turn_id;
        }
        let retry_ran = retry_record.is_some();
        repair_record = Some(json!({
            "label": format!("{label}-repair"),
            "trigger": "revision small_patch_slice first exact edit was too broad",
            "accepted": repair_accepted,
            "retry": retry_record,
            "codex_exit_status": repair.exit_status,
            "supervisor_model": repair.model.clone(),
            "supervisor_reasoning_effort": repair.reasoning_effort.clone(),
            "supervisor_input_tokens": repair.usage.input_tokens,
            "supervisor_output_tokens": repair.usage.output_tokens,
            "supervisor_reasoning_tokens": repair.usage.reasoning_tokens,
            "supervisor_total_tokens": repair.usage.total_tokens,
            "supervisor_cached_input_tokens": repair.usage.cached_input_tokens,
            "input_bytes": repair.input_bytes,
            "output_bytes": repair.output_bytes,
            "codex_app_server_thread_id": repair.thread_id.clone(),
            "codex_app_server_turn_id": repair.turn_id.clone(),
        }));
        if repair_accepted && let Some(repaired_feedback) = repaired_feedback {
            parsed_feedback = repaired_feedback;
        }
        input_tokens += repair.usage.input_tokens;
        output_tokens += repair.usage.output_tokens;
        reasoning_tokens += repair.usage.reasoning_tokens;
        total_tokens += repair.usage.total_tokens;
        cached_input_tokens += repair.usage.cached_input_tokens;
        input_bytes += repair.input_bytes;
        output_bytes += repair.output_bytes;
        if !retry_ran {
            thread_id = repair.thread_id;
            turn_id = repair.turn_id;
        }
    }
    let (mut parsed_feedback, verdict) = normalize_feedback_value(parsed_feedback);
    let typed_feedback = SupervisorFeedback::from_value(&parsed_feedback);
    let worker_mode = normalize_worker_mode(typed_feedback.worker_mode.as_deref());
    let patch_decision = normalize_patch_decision(typed_feedback.patch_decision.as_deref());
    let revision_handoff = RevisionHandoff::from_feedback(&typed_feedback);
    if let Value::Object(map) = &mut parsed_feedback {
        map.insert("worker_mode".to_string(), json!(worker_mode.clone()));
        map.insert("patch_decision".to_string(), json!(patch_decision.clone()));
    }
    let turn = SupervisorFeedbackTurn {
        verdict,
        worker_mode,
        patch_decision,
        hint: typed_feedback
            .message_to_worker
            .or(typed_feedback.hint)
            .unwrap_or_default(),
        revision_handoff,
        focus_files: typed_feedback.focus_files,
        required_checks: typed_feedback.required_checks,
        feedback: json!({
            "label": label,
            "timestamp": Utc::now().to_rfc3339(),
            "feedback": parsed_feedback,
            "repair": repair_record,
            "codex_exit_status": result.exit_status,
            "supervisor_model": result.model.clone(),
            "supervisor_reasoning_effort": result.reasoning_effort.clone(),
            "supervisor_input_tokens": input_tokens,
            "supervisor_output_tokens": output_tokens,
            "supervisor_reasoning_tokens": reasoning_tokens,
            "supervisor_total_tokens": total_tokens,
            "supervisor_cached_input_tokens": cached_input_tokens,
            "input_bytes": input_bytes,
            "output_bytes": output_bytes,
            "auth_copied_then_removed": result.auth_copied_then_removed,
            "codex_app_server_thread_id": thread_id.clone(),
            "codex_app_server_turn_id": turn_id.clone()
        }),
        input_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
        cached_input_tokens,
        input_bytes,
        output_bytes,
        thread_id,
        turn_id,
    };
    Ok(turn)
}

fn revision_repair_preserves_focus(previous: &Value, repaired: &Value) -> bool {
    let previous_feedback = SupervisorFeedback::from_value(previous);
    let previous_focus = previous_feedback
        .focus_files
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    if previous_focus.len() != 1 {
        return true;
    }

    let target = previous_focus[0].as_str();
    let repaired_feedback = SupervisorFeedback::from_value(repaired);
    repaired_feedback
        .focus_files
        .iter()
        .any(|path| path == target)
        || repaired_feedback
            .exact_edits
            .iter()
            .any(|edit| edit.contains(target))
}

fn repaired_brief_is_accepted(
    repaired_brief: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    repaired_brief
        .is_some_and(|brief| !worker_brief_needs_small_slice_repair(brief, worker_guidance))
}

fn worker_brief_repair_rejection_reason(
    repaired_brief: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> String {
    match repaired_brief {
        None => "The repaired handoff was not parseable JSON.".to_string(),
        Some(brief) if worker_brief_needs_small_slice_repair(brief, worker_guidance) => {
            "The repaired handoff still does not satisfy the expected small_patch_slice shape: use one concrete source edit as a string exact_edits item.".to_string()
        }
        Some(_) => "The repaired handoff was rejected by structural repair checks.".to_string(),
    }
}

fn repaired_feedback_is_accepted(
    previous_feedback: &Value,
    repaired_feedback: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    repaired_feedback.is_some_and(|feedback| {
        !supervisor_feedback_needs_revision_slice_repair(feedback, worker_guidance)
            && revision_repair_preserves_focus(previous_feedback, feedback)
    })
}

fn supervisor_feedback_repair_rejection_reason(
    previous_feedback: &Value,
    repaired_feedback: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> String {
    match repaired_feedback {
        None => "The repaired revision decision was not parseable JSON.".to_string(),
        Some(feedback)
            if supervisor_feedback_needs_revision_slice_repair(feedback, worker_guidance) =>
        {
            "The repaired revision still does not satisfy the expected small_patch_slice shape: use one concrete source edit as a string exact_edits item.".to_string()
        }
        Some(feedback) if !revision_repair_preserves_focus(previous_feedback, feedback) => {
            "The repaired revision changed away from the previous single focus file; preserve that target unless the artifacts prove it is wrong.".to_string()
        }
        Some(_) => "The repaired revision was rejected by structural repair checks.".to_string(),
    }
}

pub(crate) fn run_codex_app_server_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    supervisor: &SupervisorConfig,
    sandbox: CodexSandbox,
) -> Result<CodexTurnResult> {
    let mut server = CodexAppServer::start(work_dir, supervisor, sandbox)?;
    server.run_turn(artifact_dir, label, prompt)
}

fn parse_feedback_json(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        return Some(normalize_supervisor_json_value(value));
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end])
        .ok()
        .map(normalize_supervisor_json_value)
}

fn normalize_supervisor_json_value(mut value: Value) -> Value {
    for key in ["exact_edits", "edit_plan", "implementation_steps"] {
        normalize_mixed_instruction_array(&mut value, key);
    }
    value
}

fn normalize_mixed_instruction_array(value: &mut Value, key: &str) {
    let Some(field) = value.get_mut(key) else {
        return;
    };
    let normalized = match &*field {
        Value::Array(items) => items
            .iter()
            .filter_map(mixed_instruction_item_to_string)
            .collect::<Vec<_>>(),
        other => mixed_instruction_item_to_string(other)
            .into_iter()
            .collect(),
    };
    if !normalized.is_empty() {
        *field = json!(normalized);
    }
}

fn mixed_instruction_item_to_string(value: &Value) -> Option<String> {
    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let object = value.as_object()?;
    let instruction = first_object_string(
        object,
        &["instruction", "edit", "description", "message", "action"],
    );
    let file = first_object_string(object, &["file", "path", "target_file"]);
    let symbol = first_object_string(object, &["symbol", "function", "method", "target"]);

    match (file, symbol, instruction) {
        (Some(file), Some(symbol), Some(instruction)) => Some(format!(
            "In {file}, update {symbol}: {instruction}",
            file = file.trim(),
            symbol = symbol.trim(),
            instruction = instruction.trim()
        )),
        (Some(file), None, Some(instruction)) => Some(format!(
            "In {file}: {instruction}",
            file = file.trim(),
            instruction = instruction.trim()
        )),
        (None, Some(symbol), Some(instruction)) => Some(format!(
            "Update {symbol}: {instruction}",
            symbol = symbol.trim(),
            instruction = instruction.trim()
        )),
        (None, None, Some(instruction)) => Some(instruction.trim().to_string()),
        _ => serde_json::to_string(value).ok(),
    }
}

fn first_object_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
}

pub(crate) fn normalize_feedback_value(mut value: Value) -> (Value, String) {
    let raw = get_str(&value, "verdict")
        .or_else(|| get_str(&value, "action"))
        .unwrap_or("revise")
        .to_string();
    let verdict = normalize_supervisor_verdict(&raw);
    if let Value::Object(map) = &mut value {
        if raw != verdict {
            map.insert("raw_verdict".to_string(), json!(raw));
        }
        map.insert("verdict".to_string(), json!(verdict.clone()));
        map.insert("action".to_string(), json!(verdict.clone()));
    }
    (value, verdict)
}

fn normalize_supervisor_verdict(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => "approve".to_string(),
        "stop" | "stopped" | "halt" | "done" | "needs_user" | "needs-user" => "stop".to_string(),
        "revise" | "revision" | "needs_revision" | "needs-review" | "needs_review" | "reject"
        | "rejected" => "revise".to_string(),
        _ => "revise".to_string(),
    }
}

fn normalize_patch_decision(value: Option<&str>) -> String {
    match value
        .unwrap_or("accept_current")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "revise_previous" | "previous" | "keep_previous" | "restore_previous"
        | "recover_previous" => "revise_previous".to_string(),
        "revise_current" | "current_revision" | "continue_current" => "revise_current".to_string(),
        _ => "accept_current".to_string(),
    }
}

pub(crate) fn normalize_worker_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or("continue")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "context_focus" | "focused" | "focus" | "fresh" | "reset" => "context_focus".to_string(),
        _ => "continue".to_string(),
    }
}

pub(crate) fn aggregate_supervisor_usage(turns: &[SupervisorUsageSample]) -> SupervisorUsage {
    let mut usage = SupervisorUsage::default();
    for turn in turns {
        usage.input_tokens += turn.input_tokens;
        usage.output_tokens += turn.output_tokens;
        usage.reasoning_tokens += turn.reasoning_tokens;
        usage.total_tokens += turn.total_tokens;
        usage.cached_input_tokens += turn.cached_input_tokens;
        usage.input_bytes += turn.input_bytes;
        usage.output_bytes += turn.output_bytes;
        usage.turn_count += 1;
        usage.thread_ids.push(turn.thread_id.clone());
        usage.turn_ids.push(turn.turn_id.clone());
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_slice_guidance() -> WorkerSupervisorGuidance {
        WorkerSupervisorGuidance {
            model: "qwen".to_string(),
            guidance: vec![
                "For broad expected-patch tasks, prefer worker_turn_shape=small_patch_slice."
                    .to_string(),
            ],
        }
    }

    #[test]
    fn broad_qwen_worker_brief_needs_small_slice_repair() {
        let brief = json!({
            "handoff": "guided",
            "expect_patch": true,
            "worker_turn_shape": "default",
            "message_to_worker": "Implement the feature."
        });

        assert!(worker_brief_needs_small_slice_repair(
            &brief,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn broad_small_slice_worker_brief_still_needs_repair() {
        let brief = json!({
            "handoff": "guided",
            "expect_patch": true,
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": [
                "In builder.py add flatten option helpers for flatten_prefix and flatten_rename validation.",
                "In builder.py add pack support.",
                "In builder.py add unpack support.",
                "Add tests for prefix and rename."
            ]
        });

        assert!(worker_brief_needs_small_slice_repair(
            &brief,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn small_slice_worker_brief_does_not_need_repair() {
        let brief = json!({
            "handoff": "guided",
            "expect_patch": true,
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": ["Edit helper.py."]
        });

        assert!(!worker_brief_needs_small_slice_repair(
            &brief,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn parse_feedback_json_normalizes_object_exact_edits() {
        let parsed = parse_feedback_json(
            r#"{
                "handoff":"guided",
                "expect_patch":true,
                "worker_turn_shape":"small_patch_slice",
                "exact_edits":[
                    {
                        "file":"src/lib.rs",
                        "symbol":"configure",
                        "instruction":"Add the new option to the returned metadata."
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            get_string_array(&parsed, "exact_edits"),
            vec!["In src/lib.rs, update configure: Add the new option to the returned metadata."]
        );
    }

    #[test]
    fn object_style_initial_metadata_seed_does_not_need_repair() {
        let parsed = parse_feedback_json(
            r#"{
                "handoff":"guided",
                "expect_patch":true,
                "worker_turn_shape":"small_patch_slice",
                "turn_goal":"add public field metadata options",
                "files":["src/helper.py"],
                "exact_edits":[
                    {
                        "file":"src/helper.py",
                        "symbol":"field_options",
                        "instruction":"Near the line containing \"def field_options(\", add keyword parameters for one metadata seed and store their values in the returned metadata/options dict. Do not implement validation, packing, unpacking, aliases, prefix behavior, rename behavior, serialization, deserialization, or tests in this slice."
                    }
                ]
            }"#,
        )
        .unwrap();

        assert!(!worker_brief_needs_small_slice_repair(
            &parsed,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn initial_api_seed_with_flatten_option_names_does_not_need_repair() {
        let brief = json!({
            "handoff": "guided",
            "expect_patch": true,
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": [
                "In mashumaro/helper.py, update symbol field_options near the line containing \"def field_options\" to accept flatten=False, flatten_prefix=None, and flatten_rename=None, then include exactly those three values in the returned metadata/options structure following the existing field_options style."
            ]
        });

        assert!(!worker_brief_needs_small_slice_repair(
            &brief,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn broad_small_slice_revision_feedback_needs_repair() {
        let feedback = json!({
            "action": "revise",
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": [
                "In builder.py, near the field loop, support flatten by adding pack and unpack behavior for nested dataclasses."
            ]
        });

        assert!(supervisor_feedback_needs_revision_slice_repair(
            &feedback,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn atomic_small_slice_revision_feedback_does_not_need_repair() {
        let feedback = json!({
            "action": "revise",
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": [
                "In builder.py, near the line containing `for field in fields`, collect flatten=True field names into flattened_fields."
            ]
        });

        assert!(!supervisor_feedback_needs_revision_slice_repair(
            &feedback,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn anchored_revision_edit_with_negative_limits_does_not_need_repair() {
        let feedback = json!({
            "action": "revise",
            "worker_turn_shape": "small_patch_slice",
            "exact_edits": [
                "In mashumaro/core/meta/code/builder.py, in symbol _add_pack_method_lines near the line containing \"packers = {}\", handle only metadata.get(\"flatten\") in the packing/kwargs-building branch: for a flattened field, update kwargs with the packed child dict instead of assigning kwargs[fname]; do not add unpacking, validation, prefix, rename, alias-collision, or test edits in this turn."
            ]
        });

        assert!(!supervisor_feedback_needs_revision_slice_repair(
            &feedback,
            &small_slice_guidance()
        ));
    }

    #[test]
    fn revision_repair_must_preserve_single_focus_file() {
        let previous = json!({
            "action": "revise",
            "focus_files": ["mashumaro/core/meta/code/builder.py"],
            "exact_edits": [
                "In mashumaro/core/meta/code/builder.py, implement a serialization-only flatten slice."
            ]
        });
        let repaired = json!({
            "action": "revise",
            "focus_files": ["mashumaro/helper.py"],
            "exact_edits": [
                "In mashumaro/helper.py, remove flatten_prefix and flatten_rename from field_options."
            ]
        });

        assert!(!revision_repair_preserves_focus(&previous, &repaired));
    }

    #[test]
    fn revision_repair_can_keep_single_focus_file() {
        let previous = json!({
            "action": "revise",
            "focus_files": ["mashumaro/core/meta/code/builder.py"],
            "exact_edits": [
                "In mashumaro/core/meta/code/builder.py, implement a serialization-only flatten slice."
            ]
        });
        let repaired = json!({
            "action": "revise",
            "focus_files": ["mashumaro/core/meta/code/builder.py"],
            "exact_edits": [
                "In mashumaro/core/meta/code/builder.py, near the line containing \"packers = {}\", collect field names with metadata flatten=True."
            ]
        });

        assert!(revision_repair_preserves_focus(&previous, &repaired));
    }

    #[test]
    fn broad_worker_brief_repair_gets_structural_rejection_reason() {
        let brief = json!({
            "handoff": "guided",
            "expect_patch": true,
            "worker_turn_shape": "default",
            "message_to_worker": "Implement the feature."
        });

        let reason = worker_brief_repair_rejection_reason(Some(&brief), &small_slice_guidance());

        assert!(reason.contains("small_patch_slice shape"));
        assert!(reason.contains("one concrete source edit"));
    }

    #[test]
    fn changed_focus_revision_repair_gets_structural_rejection_reason() {
        let previous = json!({
            "action": "revise",
            "worker_turn_shape": "small_patch_slice",
            "focus_files": ["src/builder.rs"],
            "exact_edits": ["In src/builder.rs, make one serialization edit."]
        });
        let repaired = json!({
            "action": "revise",
            "worker_turn_shape": "small_patch_slice",
            "focus_files": ["src/helper.rs"],
            "exact_edits": ["In src/helper.rs, make one helper edit."]
        });

        let reason = supervisor_feedback_repair_rejection_reason(
            &previous,
            Some(&repaired),
            &small_slice_guidance(),
        );

        assert!(reason.contains("changed away from the previous single focus file"));
    }

    #[test]
    fn blocked_worker_brief_does_not_need_repair() {
        let brief = json!({
            "handoff": "blocked",
            "expect_patch": false,
            "message_to_worker": "Cannot proceed."
        });

        assert!(!worker_brief_needs_small_slice_repair(
            &brief,
            &small_slice_guidance()
        ));
    }
}
