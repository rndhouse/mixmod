use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::compaction::{SupervisorCompactionRequest, SupervisorCompactionState};
use super::policy::default_strategy_review_instruction;
use super::revision::{DefaultRevisionPreparation, prepare_default_revision_decision};
use crate::{
    DefaultStrategyMode, LiveSupervisorAdvisor, SUPERVISOR_CONTROL_LOG, SupervisorAdvisor,
    SupervisorCodexSession, SupervisorCompactionTurn, SupervisorContextTelemetry,
    SupervisorDirectTurn, SupervisorFeedbackTurn, SupervisorUsageSample, append_jsonl,
    append_patch_checkpoint_artifacts, run_supervisor_compaction,
    run_supervisor_direct_finish_turn, run_supervisor_feedback_turn,
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

/// Result of a supervisor takeover/direct-finish sequence.
pub(crate) struct DefaultSupervisorTakeover {
    /// Patch-state preparation performed before direct supervisor editing.
    pub(crate) preparation: DefaultRevisionPreparation,
    /// Direct supervisor finish turn.
    pub(crate) direct_finish: SupervisorDirectTurn,
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
    root: &Path,
    strategy_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    worker_guidance: &crate::WorkerSupervisorGuidance,
    supervisor_context: &mut SupervisorCompactionState,
    supervisor_samples: &mut Vec<SupervisorUsageSample>,
    strategy: DefaultStrategyMode,
) -> Result<DefaultSupervisorReview> {
    let context_telemetry = supervisor_context.telemetry(artifact_paths);
    let decision = {
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

/// Run patch preparation, optional pre-takeover compaction, and direct finish.
pub(crate) fn run_default_supervisor_takeover(
    supervisor_session: &Arc<Mutex<SupervisorCodexSession>>,
    root: &Path,
    strategy_dir: &Path,
    feedback_path: &Path,
    final_out: &Path,
    decision_index: u64,
    takeover_decision: &SupervisorFeedbackTurn,
    supervisor_context: &mut SupervisorCompactionState,
    supervisor_samples: &mut Vec<SupervisorUsageSample>,
    supervisor_compactions: &mut Vec<Value>,
    strategy: DefaultStrategyMode,
) -> Result<DefaultSupervisorTakeover> {
    let preparation = prepare_default_revision_decision(root, final_out, takeover_decision)?;
    let artifact_paths = default_strategy_review_artifacts(strategy_dir, final_out)?;
    let context_telemetry = supervisor_context.telemetry(&artifact_paths);
    if context_telemetry.compaction_enabled {
        let compact = run_default_supervisor_compaction(
            supervisor_session,
            strategy_dir,
            &format!("supervisor-compact-before-takeover-{decision_index}"),
            "supervisor_takeover",
            &json!({
                "action": "compact_now",
                "reason": format!("{} supervisor takeover", strategy.as_str())
            }),
            &context_telemetry,
        )?;
        record_default_supervisor_compaction(
            feedback_path,
            supervisor_samples,
            supervisor_context,
            supervisor_compactions,
            &compact,
        )?;
    }

    let artifact_paths = default_strategy_review_artifacts(strategy_dir, final_out)?;
    let context_telemetry = supervisor_context.telemetry(&artifact_paths);
    let direct_finish = {
        let mut supervisor_session = supervisor_session
            .lock()
            .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
        run_supervisor_direct_finish_turn(
            &mut supervisor_session,
            root,
            strategy_dir,
            &format!("supervisor-direct-finish-{decision_index}"),
            &artifact_paths,
            takeover_decision,
            &context_telemetry,
            strategy,
        )?
    };
    append_jsonl(feedback_path, &direct_finish.record)?;
    supervisor_samples.push(direct_finish.usage_sample());
    Ok(DefaultSupervisorTakeover {
        preparation,
        direct_finish,
    })
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
