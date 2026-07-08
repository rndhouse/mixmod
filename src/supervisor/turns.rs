use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};

use crate::*;

use super::codex::{run_codex_app_server_turn, supervisor_codex_sandbox_from_env};
use super::normalize::{
    normalize_feedback_value, normalize_patch_decision, normalize_worker_mode, parse_feedback_json,
};
use super::prompts::{
    supervisor_feedback_prompt, supervisor_feedback_repair_prompt,
    supervisor_feedback_repair_retry_prompt, supervisor_worker_brief_prompt,
    supervisor_worker_brief_repair_prompt, supervisor_worker_brief_repair_retry_prompt,
};
use super::repair::{
    repaired_brief_is_accepted, repaired_feedback_is_accepted,
    supervisor_feedback_needs_revision_slice_repair, supervisor_feedback_repair_rejection_reason,
    worker_brief_needs_small_slice_repair, worker_brief_repair_rejection_reason,
};
use super::types::{RevisionHandoff, SupervisorBriefTurn, SupervisorFeedbackTurn};

pub(crate) fn run_supervisor_brief_turn(
    work_dir: &Path,
    default_dir: &Path,
    task_path: &Path,
    supervisor: &SupervisorConfig,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
) -> Result<SupervisorBriefTurn> {
    let sandbox = supervisor_codex_sandbox_from_env()?;
    let prompt = supervisor_worker_brief_prompt(work_dir, task_path, worker_guidance, init_mode)?;
    let result = run_codex_app_server_turn(
        work_dir,
        default_dir,
        "worker-brief",
        &prompt,
        supervisor,
        sandbox,
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
            sandbox,
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
                sandbox,
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
            "trigger": "expected-patch handoff for selected worker was missing small_patch_slice shape",
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

pub(crate) fn run_supervisor_feedback_turn(
    work_dir: &Path,
    budgeted_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    instruction: &str,
    supervisor: &SupervisorConfig,
    worker_guidance: &WorkerSupervisorGuidance,
) -> Result<SupervisorFeedbackTurn> {
    let sandbox = supervisor_codex_sandbox_from_env()?;
    let prompt =
        supervisor_feedback_prompt(work_dir, artifact_paths, instruction, worker_guidance)?;
    let result =
        run_codex_app_server_turn(work_dir, budgeted_dir, label, &prompt, supervisor, sandbox)?;
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
            sandbox,
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
                sandbox,
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
            "trigger": "revision feedback was missing expected small_patch_slice handoff shape",
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
