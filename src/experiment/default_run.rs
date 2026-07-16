use crate::*;

use super::util::{copy_budgeted_artifacts, experiment_dir, validate_experiment_name};
use crate::strategy::default::engine::{
    DefaultStrategyEngineOptions, DefaultStrategyEngineOutput, DefaultStrategyStopOptions,
    run_default_strategy_engine,
};

#[derive(Debug, Clone)]
pub struct DefaultRunOptions {
    pub require_local: bool,
    pub model_overrides: ModelOverrides,
    pub supervisor_init: Option<SupervisorInitMode>,
    pub strategy: Option<DefaultStrategyMode>,
    pub stop_after_first_worker: bool,
    pub stop_after_first_review: bool,
    pub stop_after_worker_turns: Option<u64>,
    pub worker_target_patch_lines: Option<u64>,
    pub worker_max_patch_lines: Option<u64>,
}

pub fn experiment_run_default(root: &Path, name: &str, options: DefaultRunOptions) -> Result<()> {
    DefaultExperimentRun {
        root,
        name,
        options,
    }
    .execute()
}

struct DefaultExperimentRun<'a> {
    root: &'a Path,
    name: &'a str,
    options: DefaultRunOptions,
}

impl DefaultExperimentRun<'_> {
    fn execute(self) -> Result<()> {
        let Self {
            root,
            name,
            options,
        } = self;
        validate_experiment_name(name)?;
        let exp_dir = experiment_dir(root, name);
        let default_work_dir = exp_dir.join("work/default");
        let legacy_work_dir = exp_dir.join("work/budgeted");
        let work_dir = if default_work_dir.exists() {
            default_work_dir
        } else {
            legacy_work_dir
        };
        if !work_dir.exists() {
            bail!(
                "default strategy work directory is missing: {}. Run `mixmod experiment init {name} --fixture <fixture>` first.",
                display_path(root, &work_dir)
            );
        }
        ensure_project_state(&work_dir, false)?;
        let original_patch_base = git_rev_parse(&work_dir, "HEAD")?;

        let mut config = load_config(&work_dir)?;
        options.model_overrides.apply_to_config(&mut config)?;
        let supervisor = config.supervisor.clone();
        let supervisor_init = options
            .supervisor_init
            .unwrap_or(config.strategy.supervisor_init);
        let strategy = options.strategy.unwrap_or(config.strategy.mode);
        let live_supervision = config.strategy.live_supervision.clone();
        let worker_guidance = config
            .worker_supervisor_guidance()
            .with_patch_line_overrides(
                options.worker_target_patch_lines,
                options.worker_max_patch_lines,
            );
        let worker_self_review = env_bool("MIXMOD_WORKER_SELF_REVIEW")
            .unwrap_or_else(|| worker_guidance.worker_self_review_enabled())
            && worker_guidance.worker_self_review_enabled();
        let worker_auto_followups = worker_guidance.auto_followups_enabled();
        let worker_forced_context_focus = worker_guidance.forced_context_focus_enabled();
        let default_dir = exp_dir.join("default");
        let logs_dir = default_dir.join("logs");
        fs::create_dir_all(&logs_dir).with_context(|| {
            format!(
                "failed to create default-run logs dir {}",
                logs_dir.display()
            )
        })?;

        let run_start = Utc::now();
        let start = Instant::now();
        let task_file = work_dir.join(TASK_JSON);
        let canonical_task = exp_dir.join(TASK_JSON);
        if canonical_task.exists() {
            write_agent_visible_task_file(&canonical_task, &task_file)?;
        } else {
            ensure_agent_visible_task_file(&task_file)?;
        }
        let _ = read_task_json(&task_file)?;
        let runner = worker_harness_for_config(config);
        let runs_dir = state_layout(&work_dir).runs();
        let DefaultStrategyEngineOutput {
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
        } = run_default_strategy_engine(DefaultStrategyEngineOptions {
            root: &work_dir,
            strategy_dir: &default_dir,
            task_file: &task_file,
            runner: runner.as_ref(),
            supervisor: &supervisor,
            supervisor_init,
            strategy,
            worker_guidance: worker_guidance.clone(),
            live_supervision,
            proposal_resume_session: None,
            require_local: options.require_local,
            worker_self_review,
            worker_auto_followups,
            worker_forced_context_focus,
            stop: DefaultStrategyStopOptions {
                stop_after_first_worker: options.stop_after_first_worker,
                stop_after_first_review: options.stop_after_first_review,
                stop_after_worker_turns: options.stop_after_worker_turns,
            },
            proposal_out: runs_dir.join("default-proposal"),
            revision_task_label: name,
            revision_out_path: Box::new({
                let runs_dir = runs_dir.clone();
                move |decision_index| {
                    if decision_index == 1 {
                        runs_dir.join("default-revision")
                    } else {
                        runs_dir.join(format!("default-revision-{decision_index}"))
                    }
                }
            }),
            verify_worker_run: Box::new(|receipt, run_dir| {
                ensure_local_run_verified(
                    root,
                    &default_dir,
                    receipt,
                    run_dir,
                    options.require_local,
                )
            }),
        })?;

        let final_patch = if internal_patch_baselines > 0 {
            git_diff_from_base_with_untracked(&work_dir, &original_patch_base)?
        } else {
            git_diff_with_untracked(&work_dir)?
        };
        if internal_patch_baselines > 0 {
            restore_final_patch_to_base(&work_dir, &original_patch_base, &final_patch)?;
        }
        atomic_write(&default_dir.join(FINAL_PATCH), final_patch.as_bytes())?;
        let stats = patch_stats(&final_patch);
        copy_budgeted_artifacts(root, &default_dir, &final_out)?;

        let worker_metrics = worker_run_dirs
            .iter()
            .map(|dir| read_json_file(&dir.join(METRICS_JSON)))
            .collect::<Result<Vec<_>>>()?;
        let patch_checkpoint_metrics = patch_checkpoint_metrics(&worker_run_dirs)?;
        let final_metrics = worker_metrics.last().cloned().unwrap_or_else(|| json!({}));
        let supervisor_usage = aggregate_supervisor_usage(&supervisor_samples);
        let worker_summary = WorkerMetricsSummary::from_metrics(&worker_metrics);
        let outcome = default_strategy_outcome_with_direct_finish(
            final_decision.as_ref(),
            supervisor_direct_finish.as_ref(),
            options.stop_after_first_worker,
            options.stop_after_first_review,
            options.stop_after_worker_turns,
            opencode_calls,
        );
        let supervisor_token_usage =
            supervisor_token_usage_labels(supervisor_usage.token_usage_comparable);
        let strategy_phases = default_strategy_phase_labels(supervisor_direct_finish.is_some());
        let strategy_note = default_strategy_note(strategy);
        let supervisor_direct_finish_record =
            default_strategy_direct_finish_record(supervisor_direct_finish.as_ref());
        let metrics = json!({
            "kind": "mixmod-default-strategy",
            "strategy": strategy.as_str(),
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "supervisor_model": supervisor.model,
            "supervisor_init": supervisor_init.as_str(),
            "supervisor_input_tokens": supervisor_usage.input_tokens,
            "supervisor_reasoning_effort": supervisor.reasoning_effort,
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
            "supervisor_compaction_count": supervisor_compactions.len() as u64,
            "supervisor_compactions": supervisor_compactions,
            "did_codex_read_full_mixmod_session": false,
            "did_codex_read_raw_logs": false,
            "artifact_files_read_by_codex": CODEX_REVIEW_ARTIFACTS,
            "strategy_phases": strategy_phases,
            "codex_loop_exit": outcome.final_verdict.clone(),
            "supervisor_takeover": supervisor_takeover_decision.is_some(),
            "supervisor_takeover_decision": supervisor_takeover_decision,
            "supervisor_direct_finish": supervisor_direct_finish_record,
            "final_worker_mode": outcome.final_worker_mode,
            "worker_modes": worker_modes,
            "patch_checkpoints": patch_checkpoint_metrics,
            "internal_patch_baseline_count": internal_patch_baselines,
            "original_patch_base": original_patch_base,
            "revision_attempts": opencode_calls.saturating_sub(1),
            "stop_after_first_worker": options.stop_after_first_worker,
            "stop_after_first_review": options.stop_after_first_review,
            "stop_after_worker_turns": options.stop_after_worker_turns,
            "worker_self_review": worker_self_review,
            "worker_auto_followups": worker_auto_followups,
            "worker_forced_context_focus_after_overflow": worker_forced_context_focus,
            "worker_target_patch_lines": worker_guidance.target_patch_lines,
            "worker_max_patch_lines": worker_guidance.max_patch_lines,
            "worker_brief": WORKER_BRIEF_JSON,
            "worker_task": display_path(root, &worker_task),
            "worker_brief_output_tokens": worker_brief.output_tokens,
            "mixmod_delegations": opencode_calls,
            "opencode_calls": opencode_calls,
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
            "require_local": options.require_local,
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
            "artifact_byte_sizes": default_strategy_artifact_byte_sizes(&default_dir)?,
            "patch_bytes": final_patch.len() as u64,
            "changed_files": stats.files,
            "changed_file_count": stats.files.len(),
            "changed_line_count": stats.changed_line_count,
            "final_status": outcome.final_status,
            "final_verdict": outcome.final_verdict.clone(),
            "final_codex_action": outcome.final_verdict,
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Default strategy reused one supervisor app-server thread across worker handoff, review, repair, compaction, and live-control turns.",
                strategy_note,
                "The worker backend was selected through the Mixmod worker settings.",
                "If the worker times out, run `mixmod experiment recover <name> --require-local` to restart from worker-task.json."
            ]
        });
        write_pretty_json(
            &default_dir.join(METRICS_JSON),
            &metrics,
            "default experiment metrics",
        )?;
        atomic_write(
            &default_dir.join(REPORT_MD),
            budgeted_report(name, &metrics).as_bytes(),
        )?;
        println!(
            "default strategy experiment wrote {}",
            display_path(root, &default_dir.join(REPORT_MD))
        );
        Ok(())
    }
}

fn ensure_local_run_verified(
    root: &Path,
    default_dir: &Path,
    receipt: &Receipt,
    run_dir: &Path,
    require_local: bool,
) -> Result<()> {
    if !require_local {
        return Ok(());
    }
    let metrics = read_json_file(&run_dir.join(METRICS_JSON))?;
    if !get_bool(&metrics, "local_inference_verified").unwrap_or(false) {
        write_budgeted_blocker(
            root,
            default_dir,
            receipt,
            "local worker inference could not be verified under --require-local",
        )?;
        bail!("local worker inference could not be verified under --require-local");
    }
    Ok(())
}

fn write_budgeted_blocker(
    root: &Path,
    budgeted_dir: &Path,
    receipt: &Receipt,
    blocker: &str,
) -> Result<()> {
    fs::create_dir_all(budgeted_dir)
        .with_context(|| format!("failed to create {}", budgeted_dir.display()))?;
    let metrics = DefaultStrategyMetrics {
        kind: "mixmod-default-strategy".to_string(),
        recorded_at: Some(Utc::now().to_rfc3339()),
        final_status: "blocked".to_string(),
        blocker: Some(blocker.to_string()),
        run_receipt: Some(receipt.clone()),
        extra: serde_json::Map::new(),
    };
    write_pretty_json(
        &budgeted_dir.join(METRICS_JSON),
        &metrics,
        "default blocker metrics",
    )?;
    atomic_write(
        &budgeted_dir.join(REPORT_MD),
        format!("# Mixmod Default Strategy Blocked\n\n{blocker}\n").as_bytes(),
    )?;
    println!(
        "default strategy blocked: {}",
        display_path(root, &budgeted_dir.join(REPORT_MD))
    );
    Ok(())
}
