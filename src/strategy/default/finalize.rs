use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::engine::DefaultStrategyEngineOutput;
use crate::*;

/// Captured final patch text and derived patch statistics.
pub(crate) struct DefaultStrategyFinalPatch {
    /// Unified diff for the final worktree state.
    pub(crate) text: String,
    /// File and changed-line statistics for the final patch.
    pub(crate) stats: PatchStats,
}

/// Source for the `require_local` metric.
pub(crate) enum DefaultStrategyRequireLocal {
    /// Read the value reported by the final worker run metrics.
    FromFinalWorkerMetrics,
    /// Use an adapter-owned value.
    Fixed(bool),
}

/// Inputs required to build default-strategy run metrics.
pub(crate) struct DefaultStrategyMetricsInput<'a> {
    /// Display root used when rendering paths in metrics.
    pub(crate) display_root: &'a Path,
    /// Artifact directory for the default strategy run.
    pub(crate) strategy_dir: &'a Path,
    /// Run start timestamp.
    pub(crate) run_start: DateTime<Utc>,
    /// Wall-clock duration in milliseconds.
    pub(crate) wall_clock_ms: u128,
    /// Supervisor model configuration used for the run.
    pub(crate) supervisor: &'a SupervisorConfig,
    /// Initial supervisor handoff mode used for the run.
    pub(crate) supervisor_init: SupervisorInitMode,
    /// Default strategy mode used for the run.
    pub(crate) strategy: DefaultStrategyMode,
    /// Worker guidance applied to supervisor prompts.
    pub(crate) worker_guidance: &'a WorkerSupervisorGuidance,
    /// Whether worker self-review was enabled.
    pub(crate) worker_self_review: bool,
    /// Whether worker auto-followups were enabled.
    pub(crate) worker_auto_followups: bool,
    /// Whether context-overflow handling forced context-focus revisions.
    pub(crate) worker_forced_context_focus: bool,
    /// Stop after the proposal worker turn.
    pub(crate) stop_after_first_worker: bool,
    /// Stop after the first supervisor review.
    pub(crate) stop_after_first_review: bool,
    /// Stop after this many completed worker turns.
    pub(crate) stop_after_worker_turns: Option<u64>,
    /// Base commit used to restore final patches after internal checkpoints.
    pub(crate) original_patch_base: &'a str,
    /// Captured final patch and patch statistics.
    pub(crate) final_patch: &'a DefaultStrategyFinalPatch,
    /// Facts produced by the shared default-strategy round engine.
    pub(crate) engine: &'a DefaultStrategyEngineOutput,
    /// Source for the `require_local` metrics field.
    pub(crate) require_local: DefaultStrategyRequireLocal,
    /// Adapter-specific notes appended to the metrics note list.
    pub(crate) extra_notes: &'a [&'a str],
}

/// Capture the final worktree patch, restore it after internal baselines, and
/// write `final.patch` into the strategy artifact directory.
pub(crate) fn write_default_strategy_final_patch(
    root: &Path,
    strategy_dir: &Path,
    original_patch_base: &str,
    internal_patch_baselines: u64,
) -> Result<DefaultStrategyFinalPatch> {
    let text = if internal_patch_baselines > 0 {
        git_diff_from_base_with_untracked(root, original_patch_base)?
    } else {
        git_diff_with_untracked(root)?
    };
    if internal_patch_baselines > 0 {
        restore_final_patch_to_base(root, original_patch_base, &text)?;
    }
    atomic_write(&strategy_dir.join(FINAL_PATCH), text.as_bytes())?;
    let stats = patch_stats(&text);
    Ok(DefaultStrategyFinalPatch { text, stats })
}

/// Build the shared metrics JSON for a completed default-strategy run.
pub(crate) fn build_default_strategy_metrics(
    input: DefaultStrategyMetricsInput<'_>,
) -> Result<Value> {
    let worker_metrics = input
        .engine
        .worker_run_dirs
        .iter()
        .map(|dir| read_json_file(&dir.join(METRICS_JSON)))
        .collect::<Result<Vec<_>>>()?;
    let patch_checkpoint_metrics = patch_checkpoint_metrics(&input.engine.worker_run_dirs)?;
    let final_metrics = worker_metrics.last().cloned().unwrap_or_else(|| json!({}));
    let supervisor_usage = aggregate_supervisor_usage(&input.engine.supervisor_samples);
    let worker_summary = WorkerMetricsSummary::from_metrics(&worker_metrics);
    let outcome = default_strategy_outcome_with_direct_finish(
        input.engine.final_decision.as_ref(),
        input.engine.supervisor_direct_finish.as_ref(),
        input.stop_after_first_worker,
        input.stop_after_first_review,
        input.stop_after_worker_turns,
        input.engine.opencode_calls,
    );
    let supervisor_token_usage =
        supervisor_token_usage_labels(supervisor_usage.token_usage_comparable);
    let strategy_phases =
        default_strategy_phase_labels(input.engine.supervisor_direct_finish.is_some());
    let supervisor_direct_finish_record =
        default_strategy_direct_finish_record(input.engine.supervisor_direct_finish.as_ref());
    let worker_run_dirs = input
        .engine
        .worker_run_dirs
        .iter()
        .map(|dir| display_path(input.display_root, dir))
        .collect::<Vec<_>>();
    let require_local = match input.require_local {
        DefaultStrategyRequireLocal::FromFinalWorkerMetrics => {
            get_bool(&final_metrics, "require_local").unwrap_or(false)
        }
        DefaultStrategyRequireLocal::Fixed(value) => value,
    };
    let mut notes = vec![
        "Default strategy reused one supervisor app-server thread across worker handoff, review, repair, compaction, and live-control turns.".to_string(),
        default_strategy_note(input.strategy).to_string(),
        "The worker backend was selected through the Mixmod worker settings.".to_string(),
    ];
    notes.extend(input.extra_notes.iter().map(|note| (*note).to_string()));

    Ok(json!({
        "kind": "mixmod-default-strategy",
        "strategy": default_strategy_policy(input.strategy).id,
        "recorded_at": Utc::now().to_rfc3339(),
        "start_timestamp": input.run_start.to_rfc3339(),
        "end_timestamp": Utc::now().to_rfc3339(),
        "wall_clock_ms": input.wall_clock_ms,
        "supervisor_model": input.supervisor.model,
        "supervisor_init": input.supervisor_init.as_str(),
        "supervisor_reasoning_effort": input.supervisor.reasoning_effort,
        "supervisor_input_tokens": supervisor_usage.input_tokens,
        "supervisor_output_tokens": supervisor_usage.output_tokens,
        "supervisor_reasoning_tokens": supervisor_usage.reasoning_tokens,
        "supervisor_total_tokens": supervisor_usage.total_tokens,
        "supervisor_cached_input_tokens": supervisor_usage.cached_input_tokens,
        "supervisor_input_bytes_fallback": supervisor_usage.input_bytes,
        "supervisor_output_bytes_fallback": supervisor_usage.output_bytes,
        "codex_visible_bytes": supervisor_usage.input_bytes,
        "supervision_turn_count": supervisor_usage.turn_count,
        "codex_calls": supervisor_usage.turn_count,
        "codex_backend": "app-server-persistent",
        "codex_app_server_thread_ids": supervisor_usage.thread_ids.clone(),
        "codex_app_server_turn_ids": supervisor_usage.turn_ids.clone(),
        "codex_app_server_thread_count": supervisor_usage.thread_count(),
        "supervisor_token_usage_source": supervisor_token_usage.source,
        "supervisor_token_usage_scope": supervisor_token_usage.scope,
        "supervisor_token_usage_comparable": supervisor_usage.token_usage_comparable,
        "supervisor_session_reused": supervisor_usage.session_reused(),
        "supervisor_resume_count": supervisor_usage.thread_reuse_count(),
        "supervisor_compaction_count": input.engine.supervisor_compactions.len() as u64,
        "supervisor_compactions": input.engine.supervisor_compactions.clone(),
        "did_codex_read_full_mixmod_session": false,
        "did_codex_read_raw_logs": false,
        "artifact_files_read_by_codex": CODEX_REVIEW_ARTIFACTS,
        "strategy_phases": strategy_phases,
        "codex_loop_exit": outcome.final_verdict.clone(),
        "supervisor_takeover": input.engine.supervisor_takeover_decision.is_some(),
        "supervisor_takeover_decision": input.engine.supervisor_takeover_decision.clone(),
        "supervisor_direct_finish": supervisor_direct_finish_record,
        "final_worker_mode": outcome.final_worker_mode,
        "worker_modes": input.engine.worker_modes.clone(),
        "patch_checkpoints": patch_checkpoint_metrics,
        "internal_patch_baseline_count": input.engine.internal_patch_baselines,
        "original_patch_base": input.original_patch_base,
        "revision_attempts": input.engine.opencode_calls.saturating_sub(1),
        "stop_after_first_worker": input.stop_after_first_worker,
        "stop_after_first_review": input.stop_after_first_review,
        "stop_after_worker_turns": input.stop_after_worker_turns,
        "worker_self_review": input.worker_self_review,
        "worker_auto_followups": input.worker_auto_followups,
        "worker_forced_context_focus_after_overflow": input.worker_forced_context_focus,
        "worker_target_patch_lines": input.worker_guidance.target_patch_lines,
        "worker_max_patch_lines": input.worker_guidance.max_patch_lines,
        "worker_brief": WORKER_BRIEF_JSON,
        "worker_task": display_path(input.display_root, &input.engine.worker_task),
        "worker_brief_output_tokens": input.engine.worker_brief.output_tokens,
        "mixmod_delegations": input.engine.opencode_calls,
        "opencode_calls": input.engine.opencode_calls,
        "worker_backend": get_str(&final_metrics, "worker_backend").unwrap_or("unknown"),
        "opencode_command": get_string_array(&final_metrics, "opencode_command"),
        "opencode_exit_status": get_u64(&final_metrics, "opencode_exit_status"),
        "opencode_session_label": get_str(&final_metrics, "opencode_session_label").unwrap_or("unknown"),
        "opencode_session_id": get_str(&final_metrics, "opencode_session_id").unwrap_or("unknown"),
        "opencode_resume_session_id": get_str(&final_metrics, "opencode_resume_session_id"),
        "opencode_session_ids": worker_summary.opencode_session_ids,
        "opencode_session_labels": worker_summary.opencode_session_labels,
        "worker_session_reuse_count": worker_summary.worker_session_reuse_count,
        "worker_session_reused": get_bool(&final_metrics, "worker_session_reused").unwrap_or(false),
        "worker_run_dirs": worker_run_dirs,
        "final_worker_run_dir": display_path(input.display_root, &input.engine.final_out),
        "supervisor_control_count": worker_summary.supervisor_control_count,
        "supervisor_control_actions": worker_summary.supervisor_control_actions,
        "supervisor_control_risks": worker_summary.supervisor_control_risks,
        "supervisor_control_interrupts": worker_summary.supervisor_control_interrupts,
        "interrupted_by_supervisor": get_bool(&final_metrics, "interrupted_by_supervisor").unwrap_or(false),
        "supervisor_control_action": get_str(&final_metrics, "supervisor_control_action"),
        "opencode_timed_out": get_bool(&final_metrics, "opencode_timed_out").unwrap_or(false),
        "opencode_idle_timed_out": get_bool(&final_metrics, "opencode_idle_timed_out").unwrap_or(false),
        "heartbeat_count": get_u64(&final_metrics, "heartbeat_count").unwrap_or(0),
        "opencode_provider": get_str(&final_metrics, "opencode_provider").unwrap_or("unknown"),
        "opencode_model": get_str(&final_metrics, "opencode_model").unwrap_or("unknown"),
        "opencode_model_arg": get_str(&final_metrics, "opencode_model_arg").unwrap_or("unknown"),
        "require_local": require_local,
        "local_inference_verified": worker_summary.local_inference_verified,
        "gpu_activity_observed": worker_summary.gpu_activity_observed,
        "backend_activity_observed": worker_summary.backend_activity_observed,
        "local_worker_stdout_bytes": worker_summary.local_stdout_bytes,
        "local_worker_stderr_bytes": worker_summary.local_stderr_bytes,
        "local_worker_text_bytes": worker_summary.local_stdout_bytes + worker_summary.local_stderr_bytes,
        "local_worker_reasoning_trace_bytes": worker_summary.local_reasoning_trace_bytes,
        "local_worker_reasoning_trace_event_count": worker_summary.local_reasoning_trace_event_count,
        "local_worker_tool_events_bytes": worker_summary.local_tool_events_bytes,
        "local_worker_tool_event_count": worker_summary.local_tool_event_count,
        "worker_input_tokens": worker_summary.worker_input_tokens,
        "worker_cached_input_tokens": worker_summary.worker_cached_input_tokens,
        "worker_cache_write_tokens": worker_summary.worker_cache_write_tokens,
        "worker_output_tokens": worker_summary.worker_output_tokens,
        "worker_reasoning_tokens": worker_summary.worker_reasoning_tokens,
        "worker_total_tokens": worker_summary.worker_total_tokens,
        "worker_reported_cost_usd": worker_summary.worker_reported_cost_usd,
        "worker_token_step_count": worker_summary.worker_token_step_count,
        "worker_token_usage_source": "opencode_step_finish_tokens",
        "worker_token_usage_scope": "worker_turn_step_sum",
        "worker_token_usage_comparable": worker_summary.worker_token_usage_comparable,
        "artifact_byte_sizes": default_strategy_artifact_byte_sizes(input.strategy_dir)?,
        "patch_bytes": input.final_patch.text.len() as u64,
        "changed_files": input.final_patch.stats.files.clone(),
        "changed_file_count": input.final_patch.stats.files.len(),
        "changed_line_count": input.final_patch.stats.changed_line_count,
        "final_status": outcome.final_status,
        "final_verdict": outcome.final_verdict.clone(),
        "final_codex_action": outcome.final_verdict,
        "terminal_reject": false,
        "needs_worker_revision": false,
        "notes": notes
    }))
}

/// Build final metrics outcome when supervisor direct finish may be present.
pub(crate) fn default_strategy_outcome_with_direct_finish(
    final_decision: Option<&SupervisorFeedbackTurn>,
    direct_finish: Option<&SupervisorDirectTurn>,
    stop_after_first_worker: bool,
    stop_after_first_review: bool,
    stop_after_worker_turns: Option<u64>,
    completed_worker_turns: u64,
) -> DefaultStrategyOutcome {
    if let Some(direct_finish) = direct_finish {
        let final_status = if direct_finish.action == "approve" {
            "approved_by_supervisor_direct"
        } else {
            "stopped_by_supervisor_direct"
        };
        return DefaultStrategyOutcome {
            final_verdict: direct_finish.action.clone(),
            final_worker_mode: "supervisor_direct".to_string(),
            final_status,
        };
    }

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
            Some(SupervisorVerdict::TakeOver) => "needs_supervisor_direct",
            _ => "needs_review",
        }
    };
    DefaultStrategyOutcome {
        final_verdict,
        final_worker_mode,
        final_status,
    }
}

/// Return the stable phase labels for default-strategy metrics.
pub(crate) fn default_strategy_phase_labels(has_supervisor_direct_finish: bool) -> Value {
    if has_supervisor_direct_finish {
        json!([
            "codex_worker_brief",
            "codex_worker_decision_loop",
            "codex_supervisor_direct_finish"
        ])
    } else {
        json!(["codex_worker_brief", "codex_worker_decision_loop"])
    }
}

/// Return the serialized direct-finish record for metrics.
pub(crate) fn default_strategy_direct_finish_record(
    supervisor_direct_finish: Option<&SupervisorDirectTurn>,
) -> Value {
    supervisor_direct_finish
        .map(|turn| turn.record.clone())
        .unwrap_or(Value::Null)
}

/// Return byte sizes for default-strategy top-level worker artifacts.
pub(crate) fn default_strategy_artifact_byte_sizes(dir: &Path) -> Result<Value> {
    let mut map = serde_json::Map::new();
    for &name in WORKER_RUN_ARTIFACTS {
        let path = dir.join(name);
        if path.exists() {
            map.insert(name.to_string(), json!(file_len(&path)?));
        }
    }
    Ok(Value::Object(map))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_finish_outcome_records_supervisor_direct_approval() {
        let direct = SupervisorDirectTurn {
            record: json!({}),
            action: "approve".to_string(),
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
        };

        let outcome =
            default_strategy_outcome_with_direct_finish(None, Some(&direct), false, false, None, 2);

        assert_eq!(outcome.final_verdict, "approve");
        assert_eq!(outcome.final_worker_mode, "supervisor_direct");
        assert_eq!(outcome.final_status, "approved_by_supervisor_direct");
    }
}
