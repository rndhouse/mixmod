use crate::*;

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
        let worker_guidance = config.worker_supervisor_guidance();
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
        let worker_brief = run_supervisor_brief_turn(
            &work_dir,
            &default_dir,
            &task_file,
            &supervisor,
            &worker_guidance,
            supervisor_init,
        )?;
        write_pretty_json(
            &default_dir.join(WORKER_BRIEF_JSON),
            &worker_brief.brief,
            "worker brief",
        )?;
        append_jsonl(&feedback_path, &worker_brief.record)?;

        let worker_task = write_worker_brief_task(&task_file, &worker_brief.brief, &default_dir)?;
        let proposal_out = state_layout(&work_dir).runs().join("default-proposal");
        let proposal_receipt = run_mixmod_task_with_session_and_recovery(
            &work_dir,
            DelegationMode::Patch,
            &worker_task,
            &proposal_out,
            runner.as_ref(),
            options.require_local,
            None,
            !options.stop_after_first_worker,
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
                    let mut artifact_paths = RUN_COMPACT_ARTIFACTS
                        .iter()
                        .map(|name| final_out.join(name))
                        .collect::<Vec<_>>();
                    let supervisor_control_path = final_out.join(SUPERVISOR_CONTROL_LOG);
                    if supervisor_control_path.exists() {
                        artifact_paths.push(supervisor_control_path);
                    }
                    append_patch_checkpoint_artifacts(&final_out, &mut artifact_paths)?;
                    let decision = run_supervisor_feedback_turn(
                        &work_dir,
                        &default_dir,
                        &label,
                        &artifact_paths,
                        "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not solve by directly editing files.",
                        &supervisor,
                        &worker_guidance,
                    )?;
                    supervisor_samples.push(decision.usage_sample());
                    decision
                };
                append_jsonl(&feedback_path, &decision.feedback)?;

                match decision.verdict.as_str() {
                    "approve" | "stop" => break decision,
                    _ => {
                        worker_modes.push(decision.worker_mode.clone());
                        let resume_session_id = if decision.worker_mode == "continue" {
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
                            &task_file,
                            &default_dir,
                            name,
                            &decision,
                            decision_index,
                        )?;
                        let revision_out_name = if decision_index == 1 {
                            "default-revision".to_string()
                        } else {
                            format!("default-revision-{decision_index}")
                        };
                        let previous_out = final_out.clone();
                        final_out = state_layout(&work_dir).runs().join(revision_out_name);
                        let revision_receipt = run_mixmod_task_with_session(
                            &work_dir,
                            DelegationMode::Patch,
                            &revision_task,
                            &final_out,
                            runner.as_ref(),
                            options.require_local,
                            resume_session_id,
                        )?;
                        ensure_local_run_verified(
                            root,
                            &default_dir,
                            &revision_receipt,
                            &final_out,
                            options.require_local,
                        )?;
                        write_patch_checkpoint_comparison(&previous_out, &final_out, &decision)?;
                        opencode_calls += 1;
                        worker_run_dirs.push(final_out.clone());
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
        let supervisor_usage = aggregate_supervisor_usage(&supervisor_samples);
        let local_worker_stdout = worker_metrics
            .iter()
            .map(|metrics| get_u64(metrics, "stdout_bytes").unwrap_or(0))
            .sum::<u64>();
        let local_worker_stderr = worker_metrics
            .iter()
            .map(|metrics| get_u64(metrics, "stderr_bytes").unwrap_or(0))
            .sum::<u64>();
        let opencode_session_ids = worker_metrics
            .iter()
            .filter_map(|metrics| get_str(metrics, "opencode_session_id").map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        let opencode_session_labels = worker_metrics
            .iter()
            .filter_map(|metrics| get_str(metrics, "opencode_session_label").map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        let worker_session_reuse_count = worker_metrics
            .iter()
            .filter(|metrics| get_bool(metrics, "worker_session_reused").unwrap_or(false))
            .count() as u64;
        let supervisor_control_count = worker_metrics
            .iter()
            .map(|metrics| {
                metrics
                    .get("supervisor_control_events")
                    .and_then(Value::as_array)
                    .map(|items| items.len() as u64)
                    .unwrap_or(0)
            })
            .sum::<u64>();
        let supervisor_control_actions = worker_metrics
            .iter()
            .filter_map(|metrics| {
                get_str(metrics, "supervisor_control_action").map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        let supervisor_control_interrupts = worker_metrics
            .iter()
            .filter(|metrics| get_bool(metrics, "interrupted_by_supervisor").unwrap_or(false))
            .count() as u64;
        let local_inference_verified = !worker_metrics.is_empty()
            && worker_metrics
                .iter()
                .all(|metrics| get_bool(metrics, "local_inference_verified").unwrap_or(false));
        let gpu_activity_observed = worker_metrics
            .iter()
            .any(|metrics| get_bool(metrics, "gpu_activity_observed").unwrap_or(false));
        let backend_activity_observed = worker_metrics
            .iter()
            .any(|metrics| get_bool(metrics, "backend_activity_observed").unwrap_or(false));
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
        } else if approved {
            "approved_by_codex"
        } else if stopped_by_codex {
            "stopped_by_codex"
        } else {
            "needs_review"
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
            "codex_backend": "app-server-per-turn",
            "codex_app_server_thread_ids": supervisor_usage.thread_ids.clone(),
            "codex_app_server_turn_ids": supervisor_usage.turn_ids.clone(),
            "codex_app_server_thread_count": supervisor_usage.thread_count(),
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
            "opencode_session_ids": opencode_session_ids,
            "opencode_session_labels": opencode_session_labels,
            "worker_session_reuse_count": worker_session_reuse_count,
            "worker_session_reused": get_bool(&final_metrics, "worker_session_reused").unwrap_or(false),
            "supervisor_control_count": supervisor_control_count,
            "supervisor_control_actions": supervisor_control_actions,
            "supervisor_control_interrupts": supervisor_control_interrupts,
            "interrupted_by_supervisor": get_bool(&final_metrics, "interrupted_by_supervisor").unwrap_or(false),
            "supervisor_control_action": get_str(&final_metrics, "supervisor_control_action"),
            "opencode_timed_out": get_bool(&final_metrics, "opencode_timed_out").unwrap_or(false),
            "opencode_idle_timed_out": get_bool(&final_metrics, "opencode_idle_timed_out").unwrap_or(false),
            "heartbeat_count": get_u64(&final_metrics, "heartbeat_count").unwrap_or(0),
            "opencode_provider": get_str(&final_metrics, "opencode_provider").unwrap_or("unknown"),
            "opencode_model": get_str(&final_metrics, "opencode_model").unwrap_or("unknown"),
            "opencode_model_arg": get_str(&final_metrics, "opencode_model_arg").unwrap_or("unknown"),
            "require_local": options.require_local,
            "local_inference_verified": local_inference_verified,
            "gpu_activity_observed": gpu_activity_observed,
            "backend_activity_observed": backend_activity_observed,
            "local_worker_stdout_bytes": local_worker_stdout,
            "local_worker_stderr_bytes": local_worker_stderr,
            "local_worker_text_bytes": local_worker_stdout + local_worker_stderr,
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
                "Default strategy used a fresh supervisor app-server thread for each worker handoff and review turn.",
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
            "local Qwen 3.6 inference could not be verified under --require-local",
        )?;
        bail!("local Qwen 3.6 inference could not be verified under --require-local");
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
