use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::{
    BASELINE_ACTIVE_PATCH, LiveSupervisorAdvisor, METRICS_JSON, PREVIOUS_WORKTREE_PATCH,
    PatchDecision, SUPERVISOR_CONTROL_LOG, SupervisorAdvisor, SupervisorFeedbackTurn,
    SupervisorVerdict, WORKTREE_PATCH, WorkerMode, append_patch_checkpoint_artifacts,
    create_patch_baseline_checkpoint, restore_previous_patch_checkpoint,
    supervisor_review_artifact_paths,
};

/// Normalized terminal outcome fields shared by default-strategy metrics.
pub(crate) struct DefaultStrategyOutcome {
    /// Final supervisor action written to metrics.
    pub(crate) final_verdict: String,
    /// Final worker mode written to metrics.
    pub(crate) final_worker_mode: String,
    /// Stable final-status label written to metrics.
    pub(crate) final_status: &'static str,
}

/// Token usage labels shared by default-strategy metrics.
pub(crate) struct SupervisorTokenUsageLabels {
    /// Source label for supervisor token usage.
    pub(crate) source: &'static str,
    /// Scope label for supervisor token usage.
    pub(crate) scope: &'static str,
}

/// Filesystem preparation completed before a revision worker turn.
pub(crate) struct DefaultRevisionPreparation {
    /// Supervisor decision to use for the worker task.
    pub(crate) worker_decision: SupervisorFeedbackTurn,
    /// Patch artifact to compare against after the revision turn.
    pub(crate) previous_patch_source: PathBuf,
    /// True when an internal baseline checkpoint commit was created.
    pub(crate) created_internal_baseline: bool,
}

/// Convert the optional live supervisor into the generic harness advisor trait.
pub(crate) fn live_supervisor_advisor(
    advisor: &Option<Arc<LiveSupervisorAdvisor>>,
) -> Option<Arc<dyn SupervisorAdvisor>> {
    advisor
        .as_ref()
        .map(|advisor| Arc::clone(advisor) as Arc<dyn SupervisorAdvisor>)
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
            restore_previous_patch_checkpoint(root, previous_worker_run_dir)?;
            worker_decision.worker_mode = WorkerMode::ContextFocus.as_str().to_string();
            previous_worker_run_dir.join(PREVIOUS_WORKTREE_PATCH)
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
    active_session_id
        .clone()
        .map(Some)
        .ok_or_else(|| {
            anyhow!(
                "The supervisor requested worker_mode=continue, but Mixmod could not resolve the previous worker session id from {}",
                previous_worker_run_dir.join(METRICS_JSON).display()
            )
        })
}

/// Build the supervisor-review label for a default-strategy loop index.
pub(crate) fn default_review_label(decision_index: u64) -> String {
    if decision_index == 1 {
        "critique".to_string()
    } else {
        format!("critique-{decision_index}")
    }
}

/// Build the final metrics outcome for a default-strategy run.
pub(crate) fn default_strategy_outcome(
    final_decision: Option<&SupervisorFeedbackTurn>,
    stop_after_first_worker: bool,
    stop_after_first_review: bool,
    stop_after_worker_turns: Option<u64>,
    completed_worker_turns: u64,
) -> DefaultStrategyOutcome {
    let final_verdict = final_decision
        .map(|decision| decision.verdict.clone())
        .unwrap_or_else(|| "not_requested".to_string());
    let final_worker_mode = final_decision
        .map(|decision| decision.worker_mode.clone())
        .unwrap_or_else(|| "not_requested".to_string());
    let final_status = if stop_after_first_worker {
        "stopped_after_first_worker"
    } else if stop_after_first_review {
        "stopped_after_first_review"
    } else if final_decision.is_none()
        && stop_after_worker_turns.is_some_and(|limit| completed_worker_turns >= limit)
    {
        "stopped_after_worker_turn_limit"
    } else {
        match final_decision.map(SupervisorFeedbackTurn::verdict_kind) {
            Some(SupervisorVerdict::Approve) => "approved_by_codex",
            Some(SupervisorVerdict::Stop) => "stopped_by_codex",
            _ => "needs_review",
        }
    };
    DefaultStrategyOutcome {
        final_verdict,
        final_worker_mode,
        final_status,
    }
}

/// Return token usage labels for default-strategy supervisor metrics.
pub(crate) fn supervisor_token_usage_labels(
    token_usage_comparable: bool,
) -> SupervisorTokenUsageLabels {
    if token_usage_comparable {
        SupervisorTokenUsageLabels {
            source: "codex_app_server_total_token_usage",
            scope: "cumulative",
        }
    } else {
        SupervisorTokenUsageLabels {
            source: "incomplete_or_noncomparable",
            scope: "incomplete",
        }
    }
}
