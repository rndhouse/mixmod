use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use chrono::Utc;
use serde_json::{Value, json};

use super::compaction::{SupervisorCompactionRequest, SupervisorCompactionState};
use super::policy::default_strategy_review_instruction;
use super::revision::{DefaultRevisionPreparation, prepare_default_revision_decision};
use crate::{
    DefaultStrategyMode, LiveSupervisorAdvisor, Receipt, RevisionHandoff, SUPERVISOR_CONTROL_LOG,
    SupervisorAdvisor, SupervisorCodexSession, SupervisorCompactionTurn,
    SupervisorContextTelemetry, SupervisorFeedbackTurn, SupervisorUsageSample, SupervisorVerdict,
    WorkerMode, append_jsonl, append_patch_checkpoint_artifacts, display_path, get_str,
    run_spin_out_supervisor_feedback_turn, run_supervisor_compaction, run_supervisor_feedback_turn,
    supervisor_review_artifact_paths,
};

/// Convert the optional live supervisor into the generic harness advisor trait.
pub(crate) fn live_supervisor_advisor(
    advisor: &Option<Arc<LiveSupervisorAdvisor>>,
) -> Option<Arc<dyn SupervisorAdvisor>> {
    advisor
        .as_ref()
        .map(|advisor| Arc::clone(advisor) as Arc<dyn SupervisorAdvisor>)
}

/// Result of a supervisor review turn plus any deferred compaction request.
pub(crate) struct DefaultSupervisorReview {
    /// Parsed supervisor decision.
    pub(crate) decision: SupervisorFeedbackTurn,
    /// Compaction request to run after recording the decision, when needed.
    pub(crate) compaction_request: Option<SupervisorCompactionRequest>,
}

/// Run a supervisor compaction turn with the shared session lock protocol.
pub(crate) fn run_default_supervisor_compaction(
    supervisor_session: &Arc<Mutex<SupervisorCodexSession>>,
    strategy_dir: &Path,
    label: &str,
    trigger: &str,
    recommendation: &Value,
    telemetry: &SupervisorContextTelemetry,
) -> Result<SupervisorCompactionTurn> {
    let mut supervisor_session = supervisor_session
        .lock()
        .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
    run_supervisor_compaction(
        &mut supervisor_session,
        strategy_dir,
        label,
        trigger,
        recommendation,
        telemetry,
    )
}

/// Record a compaction turn into artifacts, metrics samples, and context state.
pub(crate) fn record_default_supervisor_compaction(
    feedback_path: &Path,
    supervisor_samples: &mut Vec<SupervisorUsageSample>,
    supervisor_context: &mut SupervisorCompactionState,
    supervisor_compactions: &mut Vec<Value>,
    compact: &SupervisorCompactionTurn,
) -> Result<()> {
    append_jsonl(feedback_path, &compact.record)?;
    supervisor_samples.push(compact.usage_sample());
    supervisor_context.record_compaction(compact);
    supervisor_compactions.push(compact.record.clone());
    Ok(())
}

/// Run a normal supervisor feedback turn and update context accounting.
pub(crate) fn run_default_supervisor_review(
    supervisor_session: &Arc<Mutex<SupervisorCodexSession>>,
    supervisor: &crate::SupervisorConfig,
    root: &Path,
    strategy_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    worker_guidance: &crate::WorkerSupervisorGuidance,
    supervisor_context: &mut SupervisorCompactionState,
    supervisor_samples: &mut Vec<SupervisorUsageSample>,
    strategy: DefaultStrategyMode,
    spin_out_supervisor_review: bool,
) -> Result<DefaultSupervisorReview> {
    let context_telemetry = supervisor_context.telemetry(artifact_paths);
    let decision = if spin_out_supervisor_review {
        run_spin_out_supervisor_feedback_turn(
            supervisor,
            root,
            strategy_dir,
            label,
            artifact_paths,
            default_strategy_review_instruction(strategy),
            worker_guidance,
            &context_telemetry,
            strategy,
        )?
    } else {
        let mut supervisor_session = supervisor_session
            .lock()
            .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
        run_supervisor_feedback_turn(
            &mut supervisor_session,
            root,
            strategy_dir,
            label,
            artifact_paths,
            default_strategy_review_instruction(strategy),
            worker_guidance,
            &context_telemetry,
            strategy,
        )?
    };
    supervisor_samples.push(decision.usage_sample());
    supervisor_context.record_feedback(&decision);
    let compaction_request = supervisor_context.request_after_feedback(&decision);
    Ok(DefaultSupervisorReview {
        decision,
        compaction_request,
    })
}

/// Prepare a fresh GPT patch request from a supervisor direct-edit plan.
pub(crate) fn prepare_default_supervisor_direct_edit(
    root: &Path,
    final_out: &Path,
    supervisor_direct_edit_decision: &SupervisorFeedbackTurn,
) -> Result<DefaultRevisionPreparation> {
    let mut preparation =
        prepare_default_revision_decision(root, final_out, supervisor_direct_edit_decision)?;
    preparation.worker_decision =
        supervisor_direct_edit_worker_decision(&preparation.worker_decision);
    Ok(preparation)
}

/// Build a strict worker handoff for executing a known supervisor direct edit.
pub(crate) fn supervisor_direct_edit_worker_decision(
    supervisor_direct_edit_decision: &SupervisorFeedbackTurn,
) -> SupervisorFeedbackTurn {
    let direct_plan = non_empty_strings(&supervisor_direct_edit_decision.direct_plan);
    let exact_edits = if direct_plan.is_empty() {
        non_empty_strings(&supervisor_direct_edit_decision.revision_handoff.exact_edits)
    } else {
        direct_plan.clone()
    };
    let fallback_goal = supervisor_direct_edit_decision
        .supervisor_direct_edit_reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("apply the supervisor-directed surgical direct edit");
    let turn_goal = supervisor_direct_edit_decision
        .revision_handoff
        .turn_goal
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "apply supervisor-directed surgical direct edit".to_string());
    let hint = if supervisor_direct_edit_decision.hint.trim().is_empty() {
        format!("Apply only the supervisor-directed surgical direct edit: {fallback_goal}.")
    } else {
        format!(
            "Apply only the supervisor-directed surgical direct edit: {}",
            supervisor_direct_edit_decision.hint.trim()
        )
    };
    let mut forbidden_actions = supervisor_direct_edit_decision
        .revision_handoff
        .forbidden_actions
        .clone();
    for rule in [
        "broaden beyond the supervisor direct_plan",
        "run broad test suites unless explicitly listed by the supervisor",
        "inspect Mixmod state or artifact directories",
        "continue a previous worker session",
    ] {
        if !forbidden_actions.iter().any(|existing| existing == rule) {
            forbidden_actions.push(rule.to_string());
        }
    }
    let deferred_checks = supervisor_direct_edit_decision
        .revision_handoff
        .deferred_checks
        .clone();
    let supervisor_direct_edit_feedback = supervisor_direct_edit_decision
        .feedback
        .get("feedback")
        .unwrap_or(&supervisor_direct_edit_decision.feedback);
    let feedback = json!({
        "type": "supervisor_direct_edit_handoff",
        "feedback": {
            "action": "revise",
            "expect_patch": true,
            "worker_mode": "context_focus",
            "patch_decision": supervisor_direct_edit_decision.patch_decision.clone(),
            "message_to_worker": hint.clone(),
            "focus_files": supervisor_direct_edit_decision.focus_files.clone(),
            "required_checks": supervisor_direct_edit_decision.required_checks.clone(),
            "risk": get_str(supervisor_direct_edit_feedback, "risk")
                .unwrap_or("supervisor_direct_edit pending"),
            "worker_turn_shape": "patch_request",
            "turn_goal": turn_goal.clone(),
            "exact_edits": exact_edits.clone(),
            "deferred_checks": deferred_checks.clone(),
            "defer_checks_until_patch_exists": true,
            "stop_condition": "Stop after applying the named surgical direct edit and any explicitly listed focused checks; do not continue into broad repair.",
            "completion_gate": "The patch implements only the supervisor direct_plan in the named target files.",
            "forbidden_actions": forbidden_actions.clone(),
            "supervisor_direct_edit_reason": supervisor_direct_edit_decision.supervisor_direct_edit_reason.clone(),
            "direct_plan": direct_plan.clone()
        }
    });

    SupervisorFeedbackTurn {
        feedback,
        verdict: SupervisorVerdict::Revise.as_str().to_string(),
        worker_mode: WorkerMode::ContextFocus.as_str().to_string(),
        patch_decision: supervisor_direct_edit_decision.patch_decision.clone(),
        hint,
        revision_handoff: RevisionHandoff {
            expect_patch: Some(true),
            worker_turn_shape: Some("patch_request".to_string()),
            turn_goal: Some(turn_goal),
            exact_edits,
            deferred_checks,
            defer_checks_until_patch_exists: Some(true),
            stop_condition: Some("Stop after applying the named surgical direct edit and any explicitly listed focused checks; do not continue into broad repair.".to_string()),
            completion_gate: Some(
                "The patch implements only the supervisor direct_plan in the named target files."
                    .to_string(),
            ),
            forbidden_actions,
            ..RevisionHandoff::default()
        },
        focus_files: supervisor_direct_edit_decision.focus_files.clone(),
        required_checks: supervisor_direct_edit_decision.required_checks.clone(),
        supervisor_direct_edit_reason: supervisor_direct_edit_decision.supervisor_direct_edit_reason.clone(),
        direct_plan,
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_comparable: true,
    }
}

/// Record that a supervisor direct edit was executed by a GPT patch turn.
pub(crate) fn supervisor_direct_edit_record(
    root: &Path,
    decision_index: u64,
    supervisor_direct_edit_decision: &SupervisorFeedbackTurn,
    worker_decision: &SupervisorFeedbackTurn,
    worker_task: &Path,
    worker_run_dir: &Path,
    receipt: &Receipt,
) -> Value {
    json!({
        "label": format!("supervisor-direct-edit-{decision_index}"),
        "timestamp": Utc::now().to_rfc3339(),
        "type": "supervisor_direct_edit",
        "trigger": "supervisor_direct_edit",
        "supervisor_direct_edit_feedback": supervisor_direct_edit_decision.feedback.clone(),
        "worker_handoff": worker_decision.feedback.clone(),
        "worker_task": display_path(root, worker_task),
        "worker_run_dir": display_path(root, worker_run_dir),
        "worker_receipt_status": receipt.status.clone(),
        "worker_receipt_summary": receipt.summary.clone(),
        "patch": receipt.patch.clone(),
        "worktree_patch": receipt.worktree_patch.clone(),
    })
}

fn non_empty_strings(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Build the artifact list reviewed by the supervisor after a worker run.
pub(crate) fn default_strategy_review_artifacts(
    strategy_dir: &Path,
    worker_run_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let mut artifact_paths = supervisor_review_artifact_paths(strategy_dir, worker_run_dir);
    let supervisor_control_path = worker_run_dir.join(SUPERVISOR_CONTROL_LOG);
    if supervisor_control_path.exists() {
        artifact_paths.push(supervisor_control_path);
    }
    append_patch_checkpoint_artifacts(worker_run_dir, &mut artifact_paths)?;
    Ok(artifact_paths)
}

/// Build the supervisor-review label for a default-strategy loop index.
pub(crate) fn default_review_label(decision_index: u64) -> String {
    if decision_index == 1 {
        "critique".to_string()
    } else {
        format!("critique-{decision_index}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supervisor_direct_edit_feedback() -> SupervisorFeedbackTurn {
        SupervisorFeedbackTurn {
            feedback: json!({
                "feedback": {
                    "action": "supervisor_direct_edit",
                    "risk": "shadowed binding remains wrong"
                }
            }),
            verdict: "supervisor_direct_edit".to_string(),
            worker_mode: "context_focus".to_string(),
            patch_decision: "revise_current".to_string(),
            hint: "Fix scoped constraint lookup only.".to_string(),
            revision_handoff: RevisionHandoff {
                turn_goal: Some("fix scoped constraint lookup".to_string()),
                deferred_checks: vec!["go test ./vm -run TestTyped".to_string()],
                forbidden_actions: vec!["touch parser files".to_string()],
                ..RevisionHandoff::default()
            },
            focus_files: vec![
                "env/envValues.go".to_string(),
                "vm/vmLetExpr.go".to_string(),
            ],
            required_checks: Vec::new(),
            supervisor_direct_edit_reason: Some("Known scoped binding defect.".to_string()),
            direct_plan: vec![
                "Add value-owner scoped constraint lookup.".to_string(),
                "Use it for identifier assignment.".to_string(),
            ],
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: 0,
            input_bytes: 0,
            output_bytes: 0,
            thread_id: String::new(),
            turn_id: String::new(),
            token_usage_comparable: true,
        }
    }

    #[test]
    fn supervisor_direct_edit_worker_decision_builds_fresh_patch_request() {
        let decision = supervisor_direct_edit_worker_decision(&supervisor_direct_edit_feedback());

        assert_eq!(decision.verdict, "revise");
        assert_eq!(decision.worker_mode, "context_focus");
        assert_eq!(decision.patch_decision, "revise_current");
        assert_eq!(
            decision.revision_handoff.worker_turn_shape.as_deref(),
            Some("patch_request")
        );
        assert_eq!(decision.revision_handoff.expect_patch, Some(true));
        assert_eq!(
            decision.revision_handoff.exact_edits,
            vec![
                "Add value-owner scoped constraint lookup.",
                "Use it for identifier assignment."
            ]
        );
        assert!(
            decision
                .revision_handoff
                .forbidden_actions
                .iter()
                .any(|rule| rule.contains("continue a previous worker session"))
        );
        assert_eq!(
            get_str(&decision.feedback["feedback"], "risk"),
            Some("shadowed binding remains wrong")
        );
        assert_eq!(
            get_str(&decision.feedback["feedback"], "worker_turn_shape"),
            Some("patch_request")
        );
    }
}
