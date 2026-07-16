use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

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
    Ok(DefaultRevisionPreparation {
        worker_decision,
        previous_patch_source,
        created_internal_baseline,
    })
}

/// Resolve the worker session to resume for a revision turn.
pub(crate) fn default_revision_resume_session_id(
    decision: &SupervisorFeedbackTurn,
    active_session_id: &Option<String>,
    previous_worker_run_dir: &Path,
) -> Result<Option<String>> {
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
}
