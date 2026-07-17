use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::create_patch_baseline_checkpoint;
use crate::{
    BASELINE_ACTIVE_PATCH, METRICS_JSON, PREVIOUS_WORKTREE_PATCH, PatchDecision,
    SupervisorFeedbackTurn, WORKTREE_PATCH, WorkerMode, restore_previous_patch_checkpoint,
};

/// Filesystem preparation completed before a revision worker turn.
pub(crate) struct DefaultRevisionPreparation {
    /// Supervisor decision to use for the worker task.
    pub(crate) worker_decision: SupervisorFeedbackTurn,
    /// Patch artifact to compare against after the revision turn.
    pub(crate) previous_patch_source: PathBuf,
    /// True when an internal baseline checkpoint commit was created.
    pub(crate) created_internal_baseline: bool,
}

/// Apply rollback semantics for a revise-previous worker turn.
pub(crate) fn prepare_default_revision_decision(
    root: &Path,
    previous_worker_run_dir: &Path,
    decision: &SupervisorFeedbackTurn,
) -> Result<DefaultRevisionPreparation> {
    let mut worker_decision = decision.clone();
    let mut created_internal_baseline = false;
    let previous_patch_source = match worker_decision.patch_decision_kind() {
        PatchDecision::RevisePrevious => {
            let previous_patch = previous_worker_run_dir.join(PREVIOUS_WORKTREE_PATCH);
            if !previous_patch.exists() {
                worker_decision.patch_decision = PatchDecision::ReviseCurrent.as_str().to_string();
                return Ok(DefaultRevisionPreparation {
                    worker_decision,
                    previous_patch_source: previous_worker_run_dir.join(WORKTREE_PATCH),
                    created_internal_baseline,
                });
            }
            restore_previous_patch_checkpoint(root, previous_worker_run_dir)?;
            worker_decision.worker_mode = WorkerMode::ContextFocus.as_str().to_string();
            previous_patch
        }
        PatchDecision::AcceptCurrentBaseline => {
            let receipt = create_patch_baseline_checkpoint(root, previous_worker_run_dir)?;
            created_internal_baseline = receipt.status == "checkpointed";
            worker_decision.worker_mode = WorkerMode::ContextFocus.as_str().to_string();
            previous_worker_run_dir.join(BASELINE_ACTIVE_PATCH)
        }
        PatchDecision::AcceptCurrent | PatchDecision::ReviseCurrent => {
            previous_worker_run_dir.join(WORKTREE_PATCH)
        }
    };
    force_fresh_session_for_revision_policy(&mut worker_decision);
    Ok(DefaultRevisionPreparation {
        worker_decision,
        previous_patch_source,
        created_internal_baseline,
    })
}

/// Force context-focused worker session policy for structured session boundaries.
pub(crate) fn force_fresh_session_for_revision_policy(
    decision: &mut SupervisorFeedbackTurn,
) -> bool {
    let Some(reason) = decision.fresh_worker_session_reason() else {
        return false;
    };
    if decision.worker_mode_kind() != WorkerMode::Continue {
        return false;
    }

    decision.worker_mode = WorkerMode::ContextFocus.as_str().to_string();
    record_fresh_session_hygiene(&mut decision.feedback, reason);
    true
}

/// Resolve the worker session to resume for a revision turn.
pub(crate) fn default_revision_resume_session_id(
    decision: &SupervisorFeedbackTurn,
    active_session_id: &Option<String>,
    previous_worker_run_dir: &Path,
) -> Result<Option<String>> {
    if decision.requires_fresh_worker_session() {
        return Ok(None);
    }
    if decision.worker_mode_kind() != WorkerMode::Continue {
        return Ok(None);
    }
    active_session_id.clone().map(Some).ok_or_else(|| {
        anyhow!(
            "The supervisor requested worker_mode=continue, but Mixmod could not resolve the previous worker session id from {}",
            previous_worker_run_dir.join(METRICS_JSON).display()
        )
    })
}

fn record_fresh_session_hygiene(feedback: &mut Value, reason: &str) {
    let hygiene = json!({
        "worker_mode_forced": true,
        "from": "continue",
        "to": "context_focus",
        "reason": reason,
    });

    let Value::Object(map) = feedback else {
        return;
    };
    map.insert("session_hygiene".to_string(), hygiene);
    map.insert("worker_mode".to_string(), json!("context_focus"));
    if let Some(Value::Object(inner)) = map.get_mut("feedback") {
        inner.insert("worker_mode".to_string(), json!("context_focus"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RevisionHandoff, get_str};
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn feedback(context_recommendation: serde_json::Value) -> SupervisorFeedbackTurn {
        SupervisorFeedbackTurn {
            feedback: json!({
                "feedback": {
                    "action": "revise",
                    "context_recommendation": context_recommendation
                }
            }),
            verdict: "revise".to_string(),
            worker_mode: "continue".to_string(),
            patch_decision: "accept_current".to_string(),
            hint: String::new(),
            revision_handoff: RevisionHandoff::default(),
            focus_files: Vec::new(),
            required_checks: Vec::new(),
            takeover_reason: None,
            direct_plan: Vec::new(),
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

    fn resume_id(decision: &SupervisorFeedbackTurn) -> Option<String> {
        default_revision_resume_session_id(
            decision,
            &Some("ses_active".to_string()),
            Path::new("/tmp/previous-run"),
        )
        .unwrap()
    }

    #[test]
    fn revise_previous_without_checkpoint_falls_back_to_current_patch() {
        let temp = TempDir::new().unwrap();
        let previous_run = temp.path().join("proposal");
        fs::create_dir_all(&previous_run).unwrap();
        fs::write(previous_run.join(WORKTREE_PATCH), b"").unwrap();
        let mut decision = feedback(json!("continue"));
        decision.patch_decision = PatchDecision::RevisePrevious.as_str().to_string();

        let preparation =
            prepare_default_revision_decision(temp.path(), &previous_run, &decision).unwrap();

        assert_eq!(
            preparation.previous_patch_source,
            previous_run.join(WORKTREE_PATCH)
        );
        assert_eq!(
            preparation.worker_decision.patch_decision,
            PatchDecision::ReviseCurrent.as_str()
        );
        assert!(!preparation.created_internal_baseline);
        assert_eq!(
            get_str(&decision.feedback["feedback"], "action"),
            Some("revise")
        );
    }

    #[test]
    fn fresh_session_policy_forces_context_focus_for_no_patch_boundaries() {
        let mut decision = feedback(json!("continue"));
        decision.revision_handoff = RevisionHandoff {
            expect_patch: Some(false),
            worker_turn_shape: Some("planning_probe".to_string()),
            ..RevisionHandoff::default()
        };

        assert!(force_fresh_session_for_revision_policy(&mut decision));
        assert_eq!(decision.worker_mode, "context_focus");
        assert_eq!(resume_id(&decision), None);
        assert_eq!(
            get_str(&decision.feedback["feedback"], "worker_mode"),
            Some("context_focus")
        );
        assert_eq!(
            get_str(&decision.feedback["session_hygiene"], "reason"),
            Some("planning_probe")
        );
    }

    #[test]
    fn fresh_session_policy_does_not_resume_verification_only_turns() {
        let mut decision = feedback(json!("continue"));
        decision.hint =
            "Verification-only: run cargo test -p mixmod and report evidence.".to_string();
        decision.revision_handoff = RevisionHandoff {
            expect_patch: Some(true),
            turn_goal: Some("verification-only confidence check".to_string()),
            ..RevisionHandoff::default()
        };

        assert!(force_fresh_session_for_revision_policy(&mut decision));
        assert_eq!(decision.worker_mode, "context_focus");
        assert_eq!(resume_id(&decision), None);
        assert_eq!(
            get_str(&decision.feedback["session_hygiene"], "reason"),
            Some("verification_only")
        );
    }

    #[test]
    fn fresh_session_policy_does_not_resume_forbidden_action_boundaries() {
        let mut decision = feedback(json!("continue"));
        decision.revision_handoff = RevisionHandoff {
            expect_patch: Some(true),
            forbidden_actions: vec!["Do not run commands in this turn.".to_string()],
            ..RevisionHandoff::default()
        };

        assert!(force_fresh_session_for_revision_policy(&mut decision));
        assert_eq!(decision.worker_mode, "context_focus");
        assert_eq!(resume_id(&decision), None);
        assert_eq!(
            get_str(&decision.feedback["session_hygiene"], "reason"),
            Some("forbidden_action_session_boundary")
        );
    }

    #[test]
    fn fresh_session_policy_does_not_resume_supervisor_control_turns() {
        let mut decision = feedback(json!("continue"));
        decision.feedback = json!({
            "label": "supervisor-control",
            "feedback": {
                "action": "revise",
                "worker_mode": "continue"
            }
        });

        assert!(force_fresh_session_for_revision_policy(&mut decision));
        assert_eq!(decision.worker_mode, "context_focus");
        assert_eq!(resume_id(&decision), None);
        assert_eq!(
            get_str(&decision.feedback["session_hygiene"], "reason"),
            Some("supervisor_control")
        );
    }

    #[test]
    fn implementation_continuation_still_reuses_active_session() {
        let decision = feedback(json!("continue"));

        assert_eq!(resume_id(&decision), Some("ses_active".to_string()));
    }
}
