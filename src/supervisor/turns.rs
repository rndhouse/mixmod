use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};

use crate::*;

use super::codex::SupervisorCodexSession;
use super::normalize::{
    normalize_feedback_value, normalize_patch_decision_kind, normalize_worker_mode,
    normalize_worker_mode_kind, parse_feedback_json,
};
use super::prompts::{
    supervisor_direct_finish_prompt, supervisor_feedback_approval_consistency_repair_prompt,
    supervisor_feedback_prompt, supervisor_worker_brief_prompt,
};
use super::types::{
    RevisionHandoff, SupervisorBriefTurn, SupervisorCompactionTurn, SupervisorContextTelemetry,
    SupervisorDirectTurn, SupervisorFeedbackTurn, SupervisorVerdict,
};

pub(crate) fn run_supervisor_brief_turn(
    session: &mut SupervisorCodexSession,
    work_dir: &Path,
    default_dir: &Path,
    task_path: &Path,
    worker_guidance: &WorkerSupervisorGuidance,
    init_mode: SupervisorInitMode,
) -> Result<SupervisorBriefTurn> {
    let prompt = supervisor_worker_brief_prompt(work_dir, task_path, worker_guidance, init_mode)?;
    let result = session.run_turn(default_dir, "worker-brief", &prompt)?;
    let parsed_brief = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "handoff": "blocked",
            "message_to_worker": "The supervisor did not return parseable handoff JSON.",
            "risk": truncate_for_report(&result.last_message, 160)
        })
    });
    let final_brief = parsed_brief;
    let record = json!({
        "label": "worker-brief",
        "timestamp": Utc::now().to_rfc3339(),
        "supervisor_init": init_mode.as_str(),
        "brief": final_brief,
        "repair": Value::Null,
        "codex_exit_status": result.exit_status,
        "supervisor_model": result.model.clone(),
        "supervisor_reasoning_effort": result.reasoning_effort.clone(),
        "supervisor_input_tokens": result.usage.input_tokens,
        "supervisor_output_tokens": result.usage.output_tokens,
        "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
        "supervisor_total_tokens": result.usage.total_tokens,
        "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
        "supervisor_token_usage_scope": if result.token_usage_comparable { "turn_group_delta" } else { "incomplete" },
        "supervisor_token_usage_comparable": result.token_usage_comparable,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": result.thread_id.clone(),
        "codex_app_server_turn_id": result.turn_id.clone()
    });
    Ok(SupervisorBriefTurn {
        record,
        brief: final_brief,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id: result.thread_id,
        turn_id: result.turn_id,
        token_usage_comparable: result.token_usage_comparable,
    })
}

pub(super) fn approval_consistency_rejection(feedback: &Value) -> Option<String> {
    let (feedback, verdict) = normalize_feedback_value(feedback.clone());
    let typed_feedback = SupervisorFeedback::from_value(&feedback);
    approval_consistency_rejection_reason(verdict, &typed_feedback)
}

pub(super) fn approval_consistency_repair_is_accepted(feedback: &Value) -> bool {
    let (feedback, verdict) = normalize_feedback_value(feedback.clone());
    matches!(
        verdict,
        SupervisorVerdict::Approve | SupervisorVerdict::Revise
    ) && approval_consistency_rejection(&feedback).is_none()
}

fn approval_consistency_rejection_reason(
    verdict: SupervisorVerdict,
    feedback: &SupervisorFeedback,
) -> Option<String> {
    if verdict != SupervisorVerdict::Approve {
        return None;
    }
    let pending = pending_approval_check_items(feedback);
    if pending.is_empty() {
        None
    } else {
        Some(format!(
            "action=approve cannot include pending check/gate field(s): {}",
            pending.join("; ")
        ))
    }
}

fn pending_approval_check_items(feedback: &SupervisorFeedback) -> Vec<String> {
    let mut pending = Vec::new();
    pending.extend(
        feedback
            .required_checks
            .iter()
            .filter(|check| !check.trim().is_empty())
            .map(|check| format!("required_checks: {}", check.trim())),
    );
    pending.extend(
        feedback
            .deferred_checks
            .iter()
            .filter(|check| !check.trim().is_empty())
            .map(|check| format!("deferred_checks: {}", check.trim())),
    );
    if let Some(gate) = feedback
        .completion_gate
        .as_deref()
        .map(str::trim)
        .filter(|gate| !gate.is_empty())
    {
        pending.push(format!("completion_gate: {gate}"));
    }
    pending
}

pub(super) fn verification_revision_for_inconsistent_approval(feedback: &Value) -> Value {
    let typed_feedback = SupervisorFeedback::from_value(feedback);
    let mut checks = Vec::new();
    checks.extend(
        typed_feedback
            .required_checks
            .iter()
            .filter(|check| !check.trim().is_empty())
            .cloned(),
    );
    checks.extend(
        typed_feedback
            .deferred_checks
            .iter()
            .filter(|check| !check.trim().is_empty())
            .cloned(),
    );
    if let Some(gate) = typed_feedback
        .completion_gate
        .as_deref()
        .map(str::trim)
        .filter(|gate| !gate.is_empty())
    {
        checks.push(format!("Completion gate: {gate}"));
    }
    checks.sort();
    checks.dedup();
    let check_summary = if checks.is_empty() {
        "Run the smallest focused task-derived verification check.".to_string()
    } else {
        format!("Run pending check(s): {}", checks.join("; "))
    };

    json!({
        "action": "revise",
        "verdict": "revise",
        "worker_mode": normalize_worker_mode(typed_feedback.worker_mode.as_deref()),
        "patch_decision": "revise_current",
        "message_to_worker": "Run the pending focused checks. If any fail, make only targeted fixes for the original task; otherwise report passing evidence.",
        "focus_files": typed_feedback.focus_files,
        "required_checks": checks,
        "risk": "Supervisor approval listed pending checks without evidence.",
        "worker_turn_shape": "default",
        "turn_goal": "verify accumulated patch before approval",
        "edit_plan": [
            check_summary,
            "If a check fails, make the smallest targeted source or test fix needed for the original task."
        ],
        "forbidden_actions": ["inspect verifier internals"]
    })
}

fn merge_repair_record(repair_record: &mut Option<Value>, key: &str, value: Value) {
    match repair_record {
        Some(Value::Object(map)) => {
            map.insert(key.to_string(), value);
        }
        Some(existing) => {
            let previous = std::mem::replace(existing, Value::Null);
            let mut map = serde_json::Map::new();
            map.insert("previous".to_string(), previous);
            map.insert(key.to_string(), value);
            *existing = Value::Object(map);
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert(key.to_string(), value);
            *repair_record = Some(Value::Object(map));
        }
    }
}

pub(crate) fn run_supervisor_feedback_turn(
    session: &mut SupervisorCodexSession,
    work_dir: &Path,
    budgeted_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    instruction: &str,
    worker_guidance: &WorkerSupervisorGuidance,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
) -> Result<SupervisorFeedbackTurn> {
    let prompt = supervisor_feedback_prompt(
        work_dir,
        artifact_paths,
        instruction,
        worker_guidance,
        context_telemetry,
        strategy,
    )?;
    let result = session.run_turn(budgeted_dir, label, &prompt)?;
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
    let (parsed_feedback, _) = normalize_feedback_value(parsed_feedback);
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
    let mut token_usage_comparable = result.token_usage_comparable;
    let (mut parsed_feedback, mut verdict) = normalize_feedback_value(parsed_feedback);
    let mut typed_feedback = SupervisorFeedback::from_value(&parsed_feedback);
    if let Some(rejection_reason) = approval_consistency_rejection_reason(verdict, &typed_feedback)
    {
        let repair_prompt = supervisor_feedback_approval_consistency_repair_prompt(
            work_dir,
            artifact_paths,
            worker_guidance,
            context_telemetry,
            strategy,
            &parsed_feedback,
            &rejection_reason,
        )?;
        let repair = session.run_turn(
            budgeted_dir,
            &format!("{label}-approval-repair"),
            &repair_prompt,
        )?;
        let repaired_feedback = parse_feedback_json(&repair.last_message)
            .map(|feedback| normalize_feedback_value(feedback).0);
        let repair_accepted = repaired_feedback
            .as_ref()
            .is_some_and(approval_consistency_repair_is_accepted);
        let fallback_applied = !repair_accepted;
        let repair_record_value = json!({
            "label": format!("{label}-approval-repair"),
            "trigger": "approval listed pending checks or a completion gate",
            "rejection_reason": rejection_reason,
            "accepted": repair_accepted,
            "fallback_applied": fallback_applied,
            "codex_exit_status": repair.exit_status,
            "supervisor_model": repair.model.clone(),
            "supervisor_reasoning_effort": repair.reasoning_effort.clone(),
            "supervisor_input_tokens": repair.usage.input_tokens,
            "supervisor_output_tokens": repair.usage.output_tokens,
            "supervisor_reasoning_tokens": repair.usage.reasoning_tokens,
            "supervisor_total_tokens": repair.usage.total_tokens,
            "supervisor_cached_input_tokens": repair.usage.cached_input_tokens,
            "supervisor_token_usage_source": repair.token_usage_source.clone(),
            "supervisor_token_usage_scope": repair.token_usage_scope.clone(),
            "supervisor_token_usage_comparable": repair.token_usage_comparable,
            "input_bytes": repair.input_bytes,
            "output_bytes": repair.output_bytes,
            "codex_app_server_thread_id": repair.thread_id.clone(),
            "codex_app_server_turn_id": repair.turn_id.clone(),
        });
        merge_repair_record(
            &mut repair_record,
            "approval_consistency",
            repair_record_value,
        );
        if repair_accepted {
            parsed_feedback = repaired_feedback.expect("repair_accepted requires parsed feedback");
        } else {
            parsed_feedback = verification_revision_for_inconsistent_approval(&parsed_feedback);
        }
        input_tokens += repair.usage.input_tokens;
        output_tokens += repair.usage.output_tokens;
        reasoning_tokens += repair.usage.reasoning_tokens;
        total_tokens += repair.usage.total_tokens;
        cached_input_tokens += repair.usage.cached_input_tokens;
        input_bytes += repair.input_bytes;
        output_bytes += repair.output_bytes;
        token_usage_comparable &= repair.token_usage_comparable;
        thread_id = repair.thread_id;
        turn_id = repair.turn_id;
        let normalized = normalize_feedback_value(parsed_feedback);
        parsed_feedback = normalized.0;
        verdict = normalized.1;
        typed_feedback = SupervisorFeedback::from_value(&parsed_feedback);
    }
    let worker_mode = normalize_worker_mode_kind(typed_feedback.worker_mode.as_deref());
    let patch_decision = normalize_patch_decision_kind(typed_feedback.patch_decision.as_deref());
    let revision_handoff = RevisionHandoff::from_feedback(&typed_feedback);
    if let Value::Object(map) = &mut parsed_feedback {
        map.insert("worker_mode".to_string(), json!(worker_mode.as_str()));
        map.insert("patch_decision".to_string(), json!(patch_decision.as_str()));
    }
    let turn = SupervisorFeedbackTurn {
        verdict: verdict.as_str().to_string(),
        worker_mode: worker_mode.as_str().to_string(),
        patch_decision: patch_decision.as_str().to_string(),
        hint: typed_feedback
            .message_to_worker
            .or(typed_feedback.hint)
            .unwrap_or_default(),
        revision_handoff,
        focus_files: typed_feedback.focus_files,
        required_checks: typed_feedback.required_checks,
        takeover_reason: typed_feedback.takeover_reason,
        direct_plan: typed_feedback.direct_plan,
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
            "supervisor_token_usage_scope": if token_usage_comparable { "turn_group_delta" } else { "incomplete" },
            "supervisor_token_usage_comparable": token_usage_comparable,
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
        token_usage_comparable,
    };
    Ok(turn)
}

pub(crate) fn run_supervisor_direct_finish_turn(
    session: &mut SupervisorCodexSession,
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    takeover_decision: &SupervisorFeedbackTurn,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
) -> Result<SupervisorDirectTurn> {
    let prompt = supervisor_direct_finish_prompt(
        work_dir,
        artifact_paths,
        takeover_decision,
        context_telemetry,
        strategy,
    )?;
    let result = session.run_turn(artifact_dir, label, &prompt)?;
    let parsed_decision = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "action": "stop",
            "summary": truncate_for_report(&result.last_message, 180),
            "checks": [],
            "risk": "direct supervisor turn did not return parseable JSON"
        })
    });
    let action = normalize_direct_finish_action(get_str(&parsed_decision, "action"));
    let mut final_decision = parsed_decision;
    if let Value::Object(map) = &mut final_decision {
        map.insert("action".to_string(), json!(action.clone()));
    }
    let surgical_contract = normalize_direct_finish_surgical_contract(&final_decision);
    let record = json!({
        "label": label,
        "timestamp": Utc::now().to_rfc3339(),
        "type": "supervisor_direct_finish",
        "decision": final_decision,
        "takeover_feedback": takeover_decision.feedback,
        "surgical_contract": surgical_contract,
        "codex_exit_status": result.exit_status,
        "supervisor_model": result.model.clone(),
        "supervisor_reasoning_effort": result.reasoning_effort.clone(),
        "supervisor_input_tokens": result.usage.input_tokens,
        "supervisor_output_tokens": result.usage.output_tokens,
        "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
        "supervisor_total_tokens": result.usage.total_tokens,
        "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
        "supervisor_token_usage_scope": if result.token_usage_comparable { "turn_group_delta" } else { "incomplete" },
        "supervisor_token_usage_comparable": result.token_usage_comparable,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": result.thread_id.clone(),
        "codex_app_server_turn_id": result.turn_id.clone()
    });
    Ok(SupervisorDirectTurn {
        record,
        action,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id: result.thread_id,
        turn_id: result.turn_id,
        token_usage_comparable: result.token_usage_comparable,
    })
}

fn normalize_direct_finish_action(value: Option<&str>) -> String {
    match value
        .unwrap_or("stop")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "approve" | "approved" | "done" | "success" => "approve".to_string(),
        _ => "stop".to_string(),
    }
}

pub(super) fn normalize_direct_finish_surgical_contract(decision: &Value) -> Value {
    let contract = decision.get("surgical_contract").unwrap_or(&Value::Null);
    let target_files = {
        let files = get_string_array(contract, "target_files");
        if files.is_empty() {
            get_string_array(decision, "changed_files")
        } else {
            files
        }
    };
    let commands_used = get_bool(contract, "commands_used").unwrap_or_else(|| {
        direct_finish_checks_indicate_commands(&get_string_array(decision, "checks"))
    });

    json!({
        "why_direct": get_str(contract, "why_direct")
            .map(|value| truncate_for_report(value, 160))
            .unwrap_or_default(),
        "target_files": target_files,
        "expected_patch_lines": get_str(contract, "expected_patch_lines")
            .unwrap_or("unknown"),
        "commands_used": commands_used,
        "command_justification": get_str(contract, "command_justification")
            .map(|value| truncate_for_report(value, 120))
            .unwrap_or_default(),
        "broad_work_required": get_bool(contract, "broad_work_required").unwrap_or(false)
    })
}

fn direct_finish_checks_indicate_commands(checks: &[String]) -> bool {
    checks.iter().any(|check| {
        let check = check.trim().to_ascii_lowercase();
        !check.is_empty() && check != "none" && !check.contains("not run")
    })
}

pub(crate) fn run_supervisor_compaction(
    session: &mut SupervisorCodexSession,
    artifact_dir: &Path,
    label: &str,
    trigger: &str,
    recommendation: &Value,
    telemetry: &SupervisorContextTelemetry,
) -> Result<SupervisorCompactionTurn> {
    let result = session.compact(artifact_dir, label)?;
    let telemetry = telemetry.to_prompt_json();
    let record = json!({
        "label": label,
        "timestamp": Utc::now().to_rfc3339(),
        "type": "supervisor_compaction",
        "trigger": trigger,
        "context_recommendation": recommendation,
        "context_telemetry": telemetry,
        "codex_exit_status": result.exit_status,
        "supervisor_model": result.model.clone(),
        "supervisor_reasoning_effort": result.reasoning_effort.clone(),
        "supervisor_input_tokens": result.usage.input_tokens,
        "supervisor_output_tokens": result.usage.output_tokens,
        "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
        "supervisor_total_tokens": result.usage.total_tokens,
        "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
        "supervisor_token_usage_source": result.token_usage_source.clone(),
        "supervisor_token_usage_scope": result.token_usage_scope.clone(),
        "supervisor_token_usage_comparable": result.token_usage_comparable,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": result.thread_id.clone(),
        "codex_app_server_turn_id": result.turn_id.clone()
    });
    Ok(SupervisorCompactionTurn {
        record,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id: result.thread_id,
        turn_id: result.turn_id,
        token_usage_comparable: result.token_usage_comparable,
    })
}
