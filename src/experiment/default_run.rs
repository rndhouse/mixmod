use crate::*;
use std::sync::{Arc, Mutex};

use super::tasks::{write_revision_task, write_worker_brief_task};
use super::util::{
    artifact_byte_sizes, copy_budgeted_artifacts, experiment_dir, validate_experiment_name,
};

#[derive(Debug, Clone)]
pub struct DefaultRunOptions {
    pub require_local: bool,
    pub model_overrides: ModelOverrides,
    pub supervisor_init: Option<SupervisorInitMode>,
    pub stop_after_first_worker: bool,
    pub stop_after_first_review: bool,
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

        let mut config = load_config(&work_dir)?;
        options.model_overrides.apply_to_config(&mut config)?;
        let supervisor = config.supervisor.clone();
        let supervisor_init = options
            .supervisor_init
            .unwrap_or(config.strategy.supervisor_init);
        let live_supervision = config.strategy.live_supervision.clone();
        let worker_guidance = config
            .worker_supervisor_guidance()
            .with_patch_line_overrides(
                options.worker_target_patch_lines,
                options.worker_max_patch_lines,
            );
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
        let feedback_path = default_dir.join(SUPERVISOR_FEEDBACK_JSONL);
        let supervisor_session = Arc::new(Mutex::new(SupervisorCodexSession::start(
            &work_dir,
            &supervisor,
        )?));
        let live_supervisor = live_supervision.enabled.then(|| {
            Arc::new(LiveSupervisorAdvisor::new(
                &work_dir,
                &default_dir,
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
                &work_dir,
                &default_dir,
                &task_file,
                &worker_guidance,
                supervisor_init,
            )?
        };
        write_pretty_json(
            &default_dir.join(WORKER_BRIEF_JSON),
            &worker_brief.brief,
            "worker brief",
        )?;
        append_jsonl(&feedback_path, &worker_brief.record)?;

        let worker_task =
            write_worker_brief_task(&work_dir, &task_file, &worker_brief.brief, &default_dir)?;
        let proposal_out = state_layout(&work_dir).runs().join("default-proposal");
        let proposal_receipt = run_mixmod_task_with_worker_options(
            &work_dir,
            DelegationMode::Patch,
            &worker_task,
            &proposal_out,
            runner.as_ref(),
            options.require_local,
            WorkerRunOptions {
                resume_session_id: None,
                allow_auto_followups: !(options.stop_after_first_worker
                    || options.stop_after_first_review),
                supervisor_advisor: live_supervisor_advisor(&live_supervisor),
            },
        )?;
        ensure_local_run_verified(
            root,
            &default_dir,
            &proposal_receipt,
            &proposal_out,
            options.require_local,
        )?;

        let mut opencode_calls = 1_u64;
        let mut worker_run_dirs = vec![proposal_out.clone()];
        write_supervision_loop_summary(&default_dir, &worker_run_dirs)?;
        let mut worker_modes = Vec::new();
        let mut active_opencode_session_id = read_opencode_session_id_from_metrics(&proposal_out)?;
        let mut pending_supervisor_control =
            supervisor_control_decision_from_metrics(&proposal_out)?;
        let mut final_out = proposal_out;
        let mut supervisor_samples = vec![worker_brief.usage_sample()];
        let final_decision = if options.stop_after_first_worker {
            None
        } else {
            Some(loop {
                let decision_index = opencode_calls;
                let decision = if let Some(decision) = pending_supervisor_control.take() {
                    decision
                } else {
                    let label = if decision_index == 1 {
                        "critique".to_string()
                    } else {
                        format!("critique-{decision_index}")
                    };
                    let mut artifact_paths =
                        supervisor_review_artifact_paths(&default_dir, &final_out);
                    let supervisor_control_path = final_out.join(SUPERVISOR_CONTROL_LOG);
                    if supervisor_control_path.exists() {
                        artifact_paths.push(supervisor_control_path);
                    }
                    append_patch_checkpoint_artifacts(&final_out, &mut artifact_paths)?;
                    let decision = {
                        let mut supervisor_session = supervisor_session
                            .lock()
                            .map_err(|_| anyhow!("supervisor Codex session lock was poisoned"))?;
                        run_supervisor_feedback_turn(
                            &mut supervisor_session,
                            &work_dir,
                            &default_dir,
                            &label,
                            &artifact_paths,
                            "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not author task-solving source changes.",
                            &worker_guidance,
                        )?
                    };
                    supervisor_samples.push(decision.usage_sample());
                    decision
                };
                append_jsonl(&feedback_path, &decision.feedback)?;

                if options.stop_after_first_review && decision_index == 1 {
                    break decision;
                }

                match decision.verdict.as_str() {
                    "approve" | "stop" => break decision,
                    _ => {
                        let mut worker_decision = decision.clone();
                        let previous_patch_source =
                            if worker_decision.patch_decision == "revise_previous" {
                                restore_previous_patch_checkpoint(&work_dir, &final_out)?;
                                worker_decision.worker_mode = "context_focus".to_string();
                                final_out.join(PREVIOUS_WORKTREE_PATCH)
                            } else {
                                final_out.join(WORKTREE_PATCH)
                            };
                        worker_modes.push(worker_decision.worker_mode.clone());
                        let resume_session_id = if worker_decision.worker_mode == "continue" {
                            Some(active_opencode_session_id.clone().ok_or_else(|| {
                                anyhow!(
                                    "The supervisor requested worker_mode=continue, but Mixmod could not resolve the previous worker session id from {}",
                                    final_out.join(METRICS_JSON).display()
                                )
                            })?)
                        } else {
                            None
                        };
                        let revision_task = write_revision_task(
                            &work_dir,
                            &task_file,
                            &default_dir,
                            name,
                            &worker_decision,
                            decision_index,
                        )?;
                        let revision_out_name = if decision_index == 1 {
                            "default-revision".to_string()
                        } else {
                            format!("default-revision-{decision_index}")
                        };
                        final_out = state_layout(&work_dir).runs().join(revision_out_name);
                        let revision_receipt = run_mixmod_task_with_worker_options(
                            &work_dir,
                            DelegationMode::Patch,
                            &revision_task,
                            &final_out,
                            runner.as_ref(),
                            options.require_local,
                            WorkerRunOptions {
                                resume_session_id,
                                allow_auto_followups: true,
                                supervisor_advisor: live_supervisor_advisor(&live_supervisor),
                            },
                        )?;
                        ensure_local_run_verified(
                            root,
                            &default_dir,
                            &revision_receipt,
                            &final_out,
                            options.require_local,
                        )?;
                        write_patch_checkpoint_comparison_from_patch(
                            &previous_patch_source,
                            &final_out,
                            &worker_decision,
                        )?;
                        opencode_calls += 1;
                        worker_run_dirs.push(final_out.clone());
                        write_supervision_loop_summary(&default_dir, &worker_run_dirs)?;
                        active_opencode_session_id =
                            read_opencode_session_id_from_metrics(&final_out)?;
                        pending_supervisor_control =
                            supervisor_control_decision_from_metrics(&final_out)?;
                    }
                }
            })
        };

        let final_patch = git_diff_with_untracked(&work_dir)?;
        atomic_write(&default_dir.join(FINAL_PATCH), final_patch.as_bytes())?;
        let stats = patch_stats(&final_patch);
        copy_budgeted_artifacts(root, &default_dir, &final_out)?;

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
        let approval_action = final_decision
            .as_ref()
            .map(|decision| decision.verdict.clone())
            .unwrap_or_else(|| "not_requested".to_string());
        let final_worker_mode = final_decision
            .as_ref()
            .map(|decision| decision.worker_mode.clone())
            .unwrap_or_else(|| "not_requested".to_string());
        let approved = approval_action == "approve";
        let stopped_by_codex = approval_action == "stop";
        let final_status = if options.stop_after_first_worker {
            "stopped_after_first_worker"
        } else if options.stop_after_first_review {
            "stopped_after_first_review"
        } else if approved {
            "approved_by_codex"
        } else if stopped_by_codex {
            "stopped_by_codex"
        } else {
            "needs_review"
        };
        let supervisor_token_usage_source = if supervisor_usage.token_usage_comparable {
            "codex_app_server_total_token_usage"
        } else {
            "incomplete_or_noncomparable"
        };
        let supervisor_token_usage_scope = if supervisor_usage.token_usage_comparable {
            "cumulative"
        } else {
            "incomplete"
        };
        let metrics = json!({
            "kind": "mixmod-default-strategy",
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
            "supervisor_token_usage_source": supervisor_token_usage_source,
            "supervisor_token_usage_scope": supervisor_token_usage_scope,
            "supervisor_token_usage_comparable": supervisor_usage.token_usage_comparable,
            "supervisor_session_reused": supervisor_usage.session_reused(),
            "supervisor_resume_count": supervisor_usage.thread_reuse_count(),
            "did_codex_read_full_mixmod_session": false,
            "did_codex_read_raw_logs": false,
            "artifact_files_read_by_codex": CODEX_REVIEW_ARTIFACTS,
            "strategy_phases": ["codex_worker_brief", "codex_worker_decision_loop"],
            "codex_loop_exit": approval_action,
            "final_worker_mode": final_worker_mode,
            "worker_modes": worker_modes,
            "patch_checkpoints": patch_checkpoint_metrics,
            "revision_attempts": opencode_calls.saturating_sub(1),
            "stop_after_first_worker": options.stop_after_first_worker,
            "stop_after_first_review": options.stop_after_first_review,
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
            "artifact_byte_sizes": artifact_byte_sizes(&default_dir)?,
            "patch_bytes": final_patch.len() as u64,
            "changed_files": stats.files,
            "changed_file_count": stats.files.len(),
            "changed_line_count": stats.changed_line_count,
            "final_status": final_status,
            "final_verdict": approval_action.clone(),
            "final_codex_action": approval_action,
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Default strategy reused one supervisor app-server thread across worker handoff, review, repair, and live-control turns.",
                "The supervisor controls the worker loop with approve, revise, or blocked/inconclusive stop decisions; direct supervisor editing is not part of this strategy.",
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

fn live_supervisor_advisor(
    advisor: &Option<Arc<LiveSupervisorAdvisor>>,
) -> Option<Arc<dyn SupervisorAdvisor>> {
    advisor
        .as_ref()
        .map(|advisor| Arc::clone(advisor) as Arc<dyn SupervisorAdvisor>)
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
