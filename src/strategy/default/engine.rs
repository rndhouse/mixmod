use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::*;

/// Stop controls shared by default strategy adapters.
pub(crate) struct DefaultStrategyStopOptions {
    /// Stop after the proposal worker turn and skip supervisor review.
    pub(crate) stop_after_first_worker: bool,
    /// Stop after the first supervisor review and leave artifacts for inspection.
    pub(crate) stop_after_first_review: bool,
    /// Stop after this many completed worker turns, before the next review.
    pub(crate) stop_after_worker_turns: Option<u64>,
}

/// Shared options for running the default strategy round engine.
pub(crate) struct DefaultStrategyEngineOptions<'a> {
    /// Repository root used by worker and supervisor turns.
    pub(crate) root: &'a Path,
    /// Directory where default-strategy artifacts are written.
    pub(crate) strategy_dir: &'a Path,
    /// Agent-visible task JSON path.
    pub(crate) task_file: &'a Path,
    /// Configured worker harness.
    pub(crate) runner: &'a dyn AgentHarness,
    /// Supervisor Codex configuration.
    pub(crate) supervisor: &'a SupervisorConfig,
    /// Initial supervisor handoff mode.
    pub(crate) supervisor_init: SupervisorInitMode,
    /// Active default strategy mode.
    pub(crate) strategy: DefaultStrategyMode,
    /// Worker-model guidance used by supervisor prompts and live control.
    pub(crate) worker_guidance: WorkerSupervisorGuidance,
    /// Live supervision configuration for this run.
    pub(crate) live_supervision: LiveSupervisionConfig,
    /// Optional worker session to resume for the proposal turn.
    pub(crate) proposal_resume_session: Option<String>,
    /// Whether worker turns should require local inference verification.
    pub(crate) require_local: bool,
    /// Whether worker self-review is enabled for worker turns.
    pub(crate) worker_self_review: bool,
    /// Whether worker auto-followups are enabled for this strategy run.
    pub(crate) worker_auto_followups: bool,
    /// Whether context-overflow recovery should force context_focus revisions.
    pub(crate) worker_forced_context_focus: bool,
    /// Stop controls for debug or benchmark slicing.
    pub(crate) stop: DefaultStrategyStopOptions,
    /// Output directory for the initial proposal worker run.
    pub(crate) proposal_out: PathBuf,
    /// Label included in generated revision tasks.
    pub(crate) revision_task_label: &'a str,
    /// Return the worker-turn output directory for a revision index.
    pub(crate) revision_out_path: Box<dyn Fn(u64) -> PathBuf + 'a>,
    /// Adapter-specific worker-turn verification and blocker handling.
    pub(crate) verify_worker_run: Box<dyn FnMut(&Receipt, &Path) -> Result<()> + 'a>,
}

/// Result of the shared default strategy round engine.
pub(crate) struct DefaultStrategyEngineOutput {
    /// Supervisor's initial worker brief turn.
    pub(crate) worker_brief: SupervisorBriefTurn,
    /// Initial worker task generated from the supervisor brief.
    pub(crate) worker_task: PathBuf,
    /// Worker run directories in execution order.
    pub(crate) worker_run_dirs: Vec<PathBuf>,
    /// Final worker run directory considered by metrics/reporting.
    pub(crate) final_out: PathBuf,
    /// Number of worker delegations performed.
    pub(crate) opencode_calls: u64,
    /// Worker modes requested for revision turns.
    pub(crate) worker_modes: Vec<String>,
    /// Number of internal patch baseline checkpoints created.
    pub(crate) internal_patch_baselines: u64,
    /// Supervisor token usage samples collected during the engine run.
    pub(crate) supervisor_samples: Vec<SupervisorUsageSample>,
    /// Supervisor compaction records written during the engine run.
    pub(crate) supervisor_compactions: Vec<Value>,
    /// Supervisor feedback that selected direct takeover, when any.
    pub(crate) supervisor_takeover_decision: Option<Value>,
    /// Direct supervisor finish turn, when strategy selected takeover.
    pub(crate) supervisor_direct_finish: Option<SupervisorDirectTurn>,
    /// Final supervisor decision observed by the engine.
    pub(crate) final_decision: Option<SupervisorFeedbackTurn>,
}

/// Run the shared default strategy worker/supervisor round engine.
pub(crate) fn run_default_strategy_engine(
    mut options: DefaultStrategyEngineOptions<'_>,
) -> Result<DefaultStrategyEngineOutput> {
    let feedback_path = options.strategy_dir.join(SUPERVISOR_FEEDBACK_JSONL);
    let supervisor_session = Arc::new(Mutex::new(SupervisorCodexSession::start(
        options.root,
        options.supervisor,
    )?));
    let live_supervisor = options.live_supervision.enabled.then(|| {
        Arc::new(LiveSupervisorAdvisor::new(
            options.root,
            options.strategy_dir,
            &feedback_path,
            Arc::clone(&supervisor_session),
            options.worker_guidance.clone(),
            options.live_supervision.clone(),
        ))
    });
    let worker_brief = {
        let mut supervisor_session = supervisor_session
            .lock()
            .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
        run_supervisor_brief_turn(
            &mut supervisor_session,
            options.root,
            options.strategy_dir,
            options.task_file,
            &options.worker_guidance,
            options.supervisor_init,
        )?
    };
    write_pretty_json(
        &options.strategy_dir.join(WORKER_BRIEF_JSON),
        &worker_brief.brief,
        "worker brief",
    )?;
    append_jsonl(&feedback_path, &worker_brief.record)?;

    let worker_task = write_worker_brief_task(
        options.root,
        options.task_file,
        &worker_brief.brief,
        options.strategy_dir,
    )?;
    let proposal_receipt = run_worker_turn_with_options(
        options.root,
        DelegationMode::Patch,
        &worker_task,
        &options.proposal_out,
        options.runner,
        options.require_local,
        WorkerTurnOptions {
            resume_session_id: options.proposal_resume_session.clone(),
            allow_auto_followups: options.worker_auto_followups
                && !(options.stop.stop_after_first_worker
                    || options.stop.stop_after_first_review
                    || options.stop.stop_after_worker_turns.is_some()),
            worker_self_review: options.worker_self_review,
            supervisor_advisor: live_supervisor_advisor(&live_supervisor),
        },
    )?;
    (options.verify_worker_run)(&proposal_receipt, &options.proposal_out)?;

    let mut opencode_calls = 1_u64;
    let mut worker_run_dirs = vec![options.proposal_out.clone()];
    write_supervision_loop_summary(options.strategy_dir, &worker_run_dirs)?;
    let mut worker_modes = Vec::new();
    let mut active_opencode_session_id =
        read_opencode_session_id_from_metrics(&options.proposal_out)?;
    let mut pending_supervisor_control =
        supervisor_control_decision_from_metrics(&options.proposal_out)?;
    let mut final_out = options.proposal_out;
    let mut internal_patch_baselines = 0_u64;
    let mut supervisor_samples = vec![worker_brief.usage_sample()];
    let mut supervisor_context = SupervisorCompactionState::default();
    supervisor_context.record_brief(&worker_brief);
    let mut supervisor_compactions = Vec::new();
    let mut supervisor_takeover_decision = None;
    let mut supervisor_direct_finish = None;
    let mut final_decision = None;
    if !should_stop_before_next_review(&options.stop, opencode_calls) {
        loop {
            if should_stop_before_next_review(&options.stop, opencode_calls) {
                break;
            }
            let decision_index = opencode_calls;
            if let Some(request) = supervisor_context.take_before_review_request() {
                let label = format!(
                    "supervisor-compact-before-{}",
                    default_review_label(decision_index)
                );
                let compact = run_default_supervisor_compaction(
                    &supervisor_session,
                    options.strategy_dir,
                    &label,
                    &request.trigger,
                    &request.recommendation,
                    &request.telemetry,
                )?;
                record_default_supervisor_compaction(
                    &feedback_path,
                    &mut supervisor_samples,
                    &mut supervisor_context,
                    &mut supervisor_compactions,
                    &compact,
                )?;
            }
            let mut compaction_request = None;
            let mut decision = if let Some(decision) = pending_supervisor_control.take() {
                decision
            } else {
                let label = default_review_label(decision_index);
                let artifact_paths =
                    default_strategy_review_artifacts(options.strategy_dir, &final_out)?;
                let review = run_default_supervisor_review(
                    &supervisor_session,
                    options.root,
                    options.strategy_dir,
                    &label,
                    &artifact_paths,
                    &options.worker_guidance,
                    &mut supervisor_context,
                    &mut supervisor_samples,
                    options.strategy,
                )?;
                compaction_request = review.compaction_request;
                review.decision
            };
            if options.stop.stop_after_first_review && decision_index == 1 {
                append_jsonl(&feedback_path, &decision.feedback)?;
                final_decision = Some(decision);
                break;
            }

            if options.worker_forced_context_focus {
                force_context_focus_after_worker_context_overflow(&mut decision, &final_out)?;
            }
            append_jsonl(&feedback_path, &decision.feedback)?;

            if decision.verdict_kind() == SupervisorVerdict::TakeOver
                && default_strategy_policy(options.strategy)
                    .direct_finish
                    .allows_takeover()
            {
                let takeover_decision = decision.clone();
                let takeover = run_default_supervisor_takeover(
                    &supervisor_session,
                    options.root,
                    options.strategy_dir,
                    &feedback_path,
                    &final_out,
                    decision_index,
                    &takeover_decision,
                    &mut supervisor_context,
                    &mut supervisor_samples,
                    &mut supervisor_compactions,
                    options.strategy,
                )?;
                if takeover.preparation.created_internal_baseline {
                    internal_patch_baselines += 1;
                }
                supervisor_takeover_decision = Some(takeover_decision.feedback.clone());
                supervisor_direct_finish = Some(takeover.direct_finish);
                final_decision = Some(takeover_decision);
                break;
            } else if decision.verdict_kind().is_terminal() {
                final_decision = Some(decision);
                break;
            } else {
                if let Some(request) = compaction_request {
                    let label = format!("supervisor-compact-{decision_index}");
                    let compact = run_default_supervisor_compaction(
                        &supervisor_session,
                        options.strategy_dir,
                        &label,
                        &request.trigger,
                        &request.recommendation,
                        &request.telemetry,
                    )?;
                    record_default_supervisor_compaction(
                        &feedback_path,
                        &mut supervisor_samples,
                        &mut supervisor_context,
                        &mut supervisor_compactions,
                        &compact,
                    )?;
                }
                let revision_preparation =
                    prepare_default_revision_decision(options.root, &final_out, &decision)?;
                let worker_decision = revision_preparation.worker_decision;
                let previous_patch_source = revision_preparation.previous_patch_source;
                if revision_preparation.created_internal_baseline {
                    internal_patch_baselines += 1;
                }
                worker_modes.push(worker_decision.worker_mode.clone());
                let resume_session_id = default_revision_resume_session_id(
                    &worker_decision,
                    &active_opencode_session_id,
                    &final_out,
                )?;
                let revision_task = write_revision_task(
                    options.root,
                    options.task_file,
                    options.strategy_dir,
                    options.revision_task_label,
                    &worker_decision,
                    decision_index,
                )?;
                final_out = (options.revision_out_path)(decision_index);
                let revision_receipt = run_worker_turn_with_options(
                    options.root,
                    DelegationMode::Patch,
                    &revision_task,
                    &final_out,
                    options.runner,
                    options.require_local,
                    WorkerTurnOptions {
                        resume_session_id,
                        allow_auto_followups: options.worker_auto_followups
                            && options.stop.stop_after_worker_turns.is_none(),
                        worker_self_review: options.worker_self_review,
                        supervisor_advisor: live_supervisor_advisor(&live_supervisor),
                    },
                )?;
                (options.verify_worker_run)(&revision_receipt, &final_out)?;
                write_patch_checkpoint_comparison_from_patch(
                    &previous_patch_source,
                    &final_out,
                    &worker_decision,
                )?;
                opencode_calls += 1;
                worker_run_dirs.push(final_out.clone());
                write_supervision_loop_summary(options.strategy_dir, &worker_run_dirs)?;
                active_opencode_session_id = read_opencode_session_id_from_metrics(&final_out)?;
                pending_supervisor_control = supervisor_control_decision_from_metrics(&final_out)?;
            }
        }
    }

    if let Some(live_supervisor) = &live_supervisor {
        supervisor_samples.extend(live_supervisor.drain_usage_samples());
    }
    Ok(DefaultStrategyEngineOutput {
        worker_brief,
        worker_task,
        worker_run_dirs,
        final_out,
        opencode_calls,
        worker_modes,
        internal_patch_baselines,
        supervisor_samples,
        supervisor_compactions,
        supervisor_takeover_decision,
        supervisor_direct_finish,
        final_decision,
    })
}

fn should_stop_before_next_review(options: &DefaultStrategyStopOptions, worker_turns: u64) -> bool {
    options.stop_after_first_worker
        || options
            .stop_after_worker_turns
            .is_some_and(|limit| worker_turns >= limit)
}
