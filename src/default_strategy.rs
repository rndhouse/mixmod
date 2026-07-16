use crate::experiment::{write_revision_task, write_worker_brief_task};
use crate::*;
use std::sync::{Arc, Mutex};

/// Options for running the public Mixmod default strategy.
pub(crate) struct DefaultStrategyOptions {
    /// Optional worker session to resume for the first worker turn.
    pub(crate) resume_session: Option<String>,
    /// Per-run model choices supplied by CLI flags.
    pub(crate) model_overrides: ModelOverrides,
    /// Optional override for the first supervisor handoff mode.
    pub(crate) supervisor_init: Option<SupervisorInitMode>,
    /// Optional override for the default-strategy orchestration mode.
    pub(crate) strategy: Option<DefaultStrategyMode>,
    /// Stop after the proposal worker run and leave artifacts for inspection.
    pub(crate) stop_after_first_worker: bool,
    /// Stop after the first supervisor review and leave artifacts for inspection.
    pub(crate) stop_after_first_review: bool,
    /// Stop after this many completed worker turns, before the next review.
    pub(crate) stop_after_worker_turns: Option<u64>,
    /// Optional worker changed-line target for one turn.
    pub(crate) worker_target_patch_lines: Option<u64>,
    /// Optional worker changed-line ceiling for one turn.
    pub(crate) worker_max_patch_lines: Option<u64>,
    /// Disable local-inference verification for this run.
    pub(crate) no_require_local: bool,
}

/// Run the supervisor-directed default strategy used by Mixmod benchmarks.
pub(crate) fn run_default_strategy(
    root: &Path,
    task_arg: &Path,
    out_dir: &Path,
    options: DefaultStrategyOptions,
) -> Result<()> {
    DefaultStrategyRun {
        root,
        task_arg,
        out_dir,
        options,
    }
    .execute()
}

struct DefaultStrategyRun<'a> {
    root: &'a Path,
    task_arg: &'a Path,
    out_dir: &'a Path,
    options: DefaultStrategyOptions,
}

impl DefaultStrategyRun<'_> {
    fn execute(self) -> Result<()> {
        let Self {
            root,
            task_arg,
            out_dir,
            options,
        } = self;
        let run_start = Utc::now();
        let start = Instant::now();
        let out_dir = absolutize(root, out_dir);
        let original_patch_base = git_rev_parse(root, "HEAD")?;
        let logs_dir = out_dir.join("logs");
        fs::create_dir_all(&logs_dir).with_context(|| {
            format!(
                "failed to create default strategy logs dir {}",
                logs_dir.display()
            )
        })?;

        let mut config = load_config(root)?;
        options.model_overrides.apply_to_config(&mut config)?;
        if options.no_require_local {
            config.opencode.require_local = false;
            config.opencode.local_verification.enabled = false;
        }
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
        let runner = worker_harness_for_config(config);

        let task_file = out_dir.join(TASK_JSON);
        write_agent_visible_task_file(&absolutize(root, task_arg), &task_file)?;
        let _ = read_task_json(&task_file)?;

        let feedback_path = out_dir.join(SUPERVISOR_FEEDBACK_JSONL);
        let supervisor_session = Arc::new(Mutex::new(SupervisorCodexSession::start(
            root,
            &supervisor,
        )?));
        let live_supervisor = live_supervision.enabled.then(|| {
            Arc::new(LiveSupervisorAdvisor::new(
                root,
                &out_dir,
                &feedback_path,
                Arc::clone(&supervisor_session),
                worker_guidance.clone(),
                live_supervision.clone(),
            ))
        });
        let worker_brief = {
            let mut supervisor_session = supervisor_session
                .lock()
                .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
            run_supervisor_brief_turn(
                &mut supervisor_session,
                root,
                &out_dir,
                &task_file,
                &worker_guidance,
                supervisor_init,
            )?
        };
        write_pretty_json(
            &out_dir.join(WORKER_BRIEF_JSON),
            &worker_brief.brief,
            "worker brief",
        )?;
        append_jsonl(&feedback_path, &worker_brief.record)?;

        let worker_task = write_worker_brief_task(root, &task_file, &worker_brief.brief, &out_dir)?;
        let worker_runs_dir = out_dir.join("worker-runs");
        let proposal_out = worker_runs_dir.join("proposal");
        let proposal_receipt = run_mixmod_task_with_worker_options(
            root,
            DelegationMode::Patch,
            &worker_task,
            &proposal_out,
            runner.as_ref(),
            false,
            WorkerRunOptions {
                resume_session_id: options.resume_session.clone(),
                allow_auto_followups: worker_auto_followups
                    && !(options.stop_after_first_worker
                        || options.stop_after_first_review
                        || options.stop_after_worker_turns.is_some()),
                worker_self_review,
                supervisor_advisor: live_supervisor_advisor(&live_supervisor),
            },
        )?;
        ensure_worker_run_verified(&out_dir, &proposal_receipt, &proposal_out)?;

        let mut opencode_calls = 1_u64;
        let mut worker_run_dirs = vec![proposal_out.clone()];
        write_supervision_loop_summary(&out_dir, &worker_run_dirs)?;
        let mut worker_modes = Vec::new();
        let mut active_opencode_session_id = read_opencode_session_id_from_metrics(&proposal_out)?;
        let mut pending_supervisor_control =
            supervisor_control_decision_from_metrics(&proposal_out)?;
        let mut final_out = proposal_out;
        let mut internal_patch_baselines = 0_u64;
        let mut supervisor_samples = vec![worker_brief.usage_sample()];
        let mut supervisor_context = SupervisorCompactionState::default();
        supervisor_context.record_brief(&worker_brief);
        let mut supervisor_compactions = Vec::new();
        let mut supervisor_takeover_decision = None;
        let mut supervisor_direct_finish = None;
        let mut final_decision = None;
        if !should_stop_before_next_review(&options, opencode_calls) {
            loop {
                if should_stop_before_next_review(&options, opencode_calls) {
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
                        &out_dir,
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
                    let artifact_paths = default_strategy_review_artifacts(&out_dir, &final_out)?;
                    let review = run_default_supervisor_review(
                        &supervisor_session,
                        root,
                        &out_dir,
                        &label,
                        &artifact_paths,
                        &worker_guidance,
                        &mut supervisor_context,
                        &mut supervisor_samples,
                        strategy,
                    )?;
                    compaction_request = review.compaction_request;
                    review.decision
                };
                if options.stop_after_first_review && decision_index == 1 {
                    append_jsonl(&feedback_path, &decision.feedback)?;
                    final_decision = Some(decision);
                    break;
                }

                if worker_forced_context_focus {
                    force_context_focus_after_worker_context_overflow(&mut decision, &final_out)?;
                }
                append_jsonl(&feedback_path, &decision.feedback)?;

                if decision.verdict_kind() == SupervisorVerdict::TakeOver
                    && strategy.allows_supervisor_takeover()
                {
                    let takeover_decision = decision.clone();
                    let takeover = run_default_supervisor_takeover(
                        &supervisor_session,
                        root,
                        &out_dir,
                        &feedback_path,
                        &final_out,
                        decision_index,
                        &takeover_decision,
                        &mut supervisor_context,
                        &mut supervisor_samples,
                        &mut supervisor_compactions,
                        strategy,
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
                            &out_dir,
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
                        prepare_default_revision_decision(root, &final_out, &decision)?;
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
                        root,
                        &task_file,
                        &out_dir,
                        "exec",
                        &worker_decision,
                        decision_index,
                    )?;
                    let revision_out_name = if decision_index == 1 {
                        "revision".to_string()
                    } else {
                        format!("revision-{decision_index}")
                    };
                    final_out = worker_runs_dir.join(revision_out_name);
                    let revision_receipt = run_mixmod_task_with_worker_options(
                        root,
                        DelegationMode::Patch,
                        &revision_task,
                        &final_out,
                        runner.as_ref(),
                        false,
                        WorkerRunOptions {
                            resume_session_id,
                            allow_auto_followups: worker_auto_followups
                                && options.stop_after_worker_turns.is_none(),
                            worker_self_review,
                            supervisor_advisor: live_supervisor_advisor(&live_supervisor),
                        },
                    )?;
                    ensure_worker_run_verified(&out_dir, &revision_receipt, &final_out)?;
                    write_patch_checkpoint_comparison_from_patch(
                        &previous_patch_source,
                        &final_out,
                        &worker_decision,
                    )?;
                    opencode_calls += 1;
                    worker_run_dirs.push(final_out.clone());
                    write_supervision_loop_summary(&out_dir, &worker_run_dirs)?;
                    active_opencode_session_id = read_opencode_session_id_from_metrics(&final_out)?;
                    pending_supervisor_control =
                        supervisor_control_decision_from_metrics(&final_out)?;
                }
            }
        }

        let final_patch = if internal_patch_baselines > 0 {
            git_diff_from_base_with_untracked(root, &original_patch_base)?
        } else {
            git_diff_with_untracked(root)?
        };
        if internal_patch_baselines > 0 {
            restore_final_patch_to_base(root, &original_patch_base, &final_patch)?;
        }
        atomic_write(&out_dir.join(FINAL_PATCH), final_patch.as_bytes())?;
        let stats = patch_stats(&final_patch);

        let worker_metrics = worker_run_dirs
            .iter()
            .map(|dir| read_json_file(&dir.join(METRICS_JSON)))
            .collect::<Result<Vec<_>>>()?;
        let patch_checkpoint_metrics = patch_checkpoint_metrics(&worker_run_dirs)?;
        let final_metrics = worker_metrics.last().cloned().unwrap_or_else(|| json!({}));
        if let Some(live_supervisor) = &live_supervisor {
            supervisor_samples.extend(live_supervisor.drain_usage_samples());
        }
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
        let worker_run_dirs = worker_run_dirs
            .iter()
            .map(|dir| display_path(root, dir))
            .collect::<Vec<_>>();
        let metrics = json!({
            "kind": "mixmod-default-strategy",
            "strategy": strategy.as_str(),
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "supervisor_model": supervisor.model,
            "supervisor_init": supervisor_init.as_str(),
            "supervisor_reasoning_effort": supervisor.reasoning_effort,
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
            "worker_run_dirs": worker_run_dirs,
            "final_worker_run_dir": display_path(root, &final_out),
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
            "require_local": get_bool(&final_metrics, "require_local").unwrap_or(false),
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
            "worker_token_usage_scope": "worker_run_step_sum",
            "worker_token_usage_comparable": worker_summary.worker_token_usage_comparable,
            "artifact_byte_sizes": default_strategy_artifact_byte_sizes(&out_dir)?,
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
                "The worker backend was selected through the Mixmod worker settings."
            ]
        });
        write_pretty_json(
            &out_dir.join(METRICS_JSON),
            &metrics,
            "default strategy metrics",
        )?;
        atomic_write(
            &out_dir.join(REPORT_MD),
            budgeted_report("exec", &metrics).as_bytes(),
        )?;

        println!(
            "Mixmod exec wrote artifacts to {}",
            display_path(root, &out_dir)
        );
        println!("status: {}", outcome.final_status);
        println!("report: {}", display_path(root, &out_dir.join(REPORT_MD)));
        println!("patch: {}", display_path(root, &out_dir.join(FINAL_PATCH)));
        Ok(())
    }
}

fn ensure_worker_run_verified(out_dir: &Path, receipt: &Receipt, run_dir: &Path) -> Result<()> {
    let metrics = read_json_file(&run_dir.join(METRICS_JSON))?;
    if !get_bool(&metrics, "require_local").unwrap_or(false)
        || get_bool(&metrics, "local_inference_verified").unwrap_or(false)
    {
        return Ok(());
    }

    write_pretty_json(
        &out_dir.join(BLOCKED_RECEIPT_JSON),
        receipt,
        "blocked worker receipt",
    )?;
    bail!(
        "local worker inference was required but could not be verified for {}",
        run_dir.display()
    )
}

fn should_stop_before_next_review(options: &DefaultStrategyOptions, worker_turns: u64) -> bool {
    options.stop_after_first_worker
        || options
            .stop_after_worker_turns
            .is_some_and(|limit| worker_turns >= limit)
}
