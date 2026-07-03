use crate::experiment::{write_revision_task, write_worker_brief_task};
use crate::*;

/// Options for running the public Mixmod default strategy.
pub(crate) struct DefaultStrategyOptions {
    /// Optional worker session to resume for the first worker turn.
    pub(crate) resume_session: Option<String>,
    /// Per-run model choices supplied by CLI flags.
    pub(crate) model_overrides: ModelOverrides,
}

/// Run the Codex-supervised default strategy used by Mixmod benchmarks.
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
        let logs_dir = out_dir.join("logs");
        fs::create_dir_all(&logs_dir).with_context(|| {
            format!(
                "failed to create default strategy logs dir {}",
                logs_dir.display()
            )
        })?;

        let mut config = load_config(root)?;
        options.model_overrides.apply_to_config(&mut config)?;
        let frontier = config.frontier.clone();
        let worker_guidance = config.worker_supervisor_guidance();
        let runner = worker_harness_for_config(config);

        let task_file = out_dir.join("task.json");
        write_agent_visible_task_file(&absolutize(root, task_arg), &task_file)?;
        let (_, task_spec) = read_task_json(&task_file)?;

        let feedback_path = out_dir.join("frontier-feedback.jsonl");
        let worker_brief =
            run_frontier_brief_turn(root, &out_dir, &task_file, &frontier, &worker_guidance)?;
        write_pretty_json(
            &out_dir.join("worker-brief.json"),
            &worker_brief.brief,
            "worker brief",
        )?;
        append_jsonl(&feedback_path, &worker_brief.record)?;

        let worker_task = write_worker_brief_task(&task_file, &worker_brief.brief, &out_dir)?;
        let worker_runs_dir = out_dir.join("worker-runs");
        let proposal_out = worker_runs_dir.join("proposal");
        let proposal_receipt = run_mixmod_task_with_session(
            root,
            DelegationMode::Patch,
            &worker_task,
            &proposal_out,
            runner.as_ref(),
            false,
            options.resume_session,
        )?;
        ensure_worker_run_verified(&out_dir, &proposal_receipt, &proposal_out)?;

        let mut opencode_calls = 1_u64;
        let mut worker_run_dirs = vec![proposal_out.clone()];
        let mut worker_modes = Vec::new();
        let mut active_opencode_session_id = read_opencode_session_id_from_metrics(&proposal_out)?;
        let mut pending_supervisor_control =
            supervisor_control_decision_from_metrics(&proposal_out)?;
        let mut final_out = proposal_out;
        let mut frontier_samples = vec![worker_brief.usage_sample()];
        let final_decision = loop {
            let decision_index = opencode_calls;
            let decision = if let Some(decision) = pending_supervisor_control.take() {
                decision
            } else {
                let label = if decision_index == 1 {
                    "critique".to_string()
                } else {
                    format!("critique-{decision_index}")
                };
                let mut artifact_paths = vec![
                    final_out.join("receipt.json"),
                    final_out.join("report.md"),
                    final_out.join("worktree.patch"),
                    final_out.join("changes.patch"),
                    final_out.join("tests.json"),
                    final_out.join("metrics.json"),
                ];
                let supervisor_control_path = final_out.join(SUPERVISOR_CONTROL_LOG);
                if supervisor_control_path.exists() {
                    artifact_paths.push(supervisor_control_path);
                }
                append_patch_checkpoint_artifacts(&final_out, &mut artifact_paths)?;
                let decision = run_frontier_feedback_turn(
                    root,
                    &out_dir,
                    &label,
                    &artifact_paths,
                    "Decide the next worker-loop action. Use approve only when the worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive worker result when no useful worker path remains; do not solve by directly editing files.",
                    &frontier,
                    &worker_guidance,
                )?;
                frontier_samples.push(decision.usage_sample());
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
                                "Codex requested worker_mode=continue, but Mixmod could not resolve the previous worker session id from {}",
                                final_out.join("metrics.json").display()
                            )
                        })?)
                    } else {
                        None
                    };
                    let revision_task = write_revision_task(
                        &task_file,
                        &out_dir,
                        "exec",
                        &decision,
                        decision_index,
                    )?;
                    let revision_out_name = if decision_index == 1 {
                        "revision".to_string()
                    } else {
                        format!("revision-{decision_index}")
                    };
                    let previous_out = final_out.clone();
                    final_out = worker_runs_dir.join(revision_out_name);
                    let revision_receipt = run_mixmod_task_with_session(
                        root,
                        DelegationMode::Patch,
                        &revision_task,
                        &final_out,
                        runner.as_ref(),
                        false,
                        resume_session_id,
                    )?;
                    ensure_worker_run_verified(&out_dir, &revision_receipt, &final_out)?;
                    write_patch_checkpoint_comparison(&previous_out, &final_out, &decision)?;
                    opencode_calls += 1;
                    worker_run_dirs.push(final_out.clone());
                    active_opencode_session_id = read_opencode_session_id_from_metrics(&final_out)?;
                    pending_supervisor_control =
                        supervisor_control_decision_from_metrics(&final_out)?;
                }
            }
        };

        let final_patch = git_diff_with_untracked(root)?;
        atomic_write(&out_dir.join("final.patch"), final_patch.as_bytes())?;
        let stats = patch_stats(&final_patch);
        let test_status = run_final_tests(root, &out_dir, &task_spec.tests)?;

        let worker_metrics = worker_run_dirs
            .iter()
            .map(|dir| read_json_file(&dir.join("metrics.json")))
            .collect::<Result<Vec<_>>>()?;
        let patch_checkpoint_metrics = patch_checkpoint_metrics(&worker_run_dirs)?;
        let final_metrics = worker_metrics.last().cloned().unwrap_or_else(|| json!({}));
        let frontier_usage = aggregate_frontier_usage(&frontier_samples);
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
        let approval_action = final_decision.verdict.clone();
        let approved = approval_action == "approve";
        let stopped_by_codex = approval_action == "stop";
        let final_status = if approved {
            "approved_by_codex"
        } else if stopped_by_codex {
            "stopped_by_codex"
        } else {
            "needs_review"
        };
        let worker_run_dirs = worker_run_dirs
            .iter()
            .map(|dir| display_path(root, dir))
            .collect::<Vec<_>>();
        let metrics = json!({
            "kind": "mixmod-default-strategy",
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "frontier_model": frontier.model,
            "frontier_reasoning_effort": frontier.reasoning_effort,
            "frontier_input_tokens": frontier_usage.input_tokens,
            "frontier_output_tokens": frontier_usage.output_tokens,
            "frontier_reasoning_tokens": frontier_usage.reasoning_tokens,
            "frontier_total_tokens": frontier_usage.total_tokens,
            "frontier_cached_input_tokens": frontier_usage.cached_input_tokens,
            "frontier_input_bytes_fallback": frontier_usage.input_bytes,
            "frontier_output_bytes_fallback": frontier_usage.output_bytes,
            "codex_visible_bytes": frontier_usage.input_bytes,
            "supervision_turn_count": frontier_usage.turn_count,
            "codex_calls": frontier_usage.turn_count,
            "codex_backend": "app-server-per-turn",
            "codex_app_server_thread_ids": frontier_usage.thread_ids.clone(),
            "codex_app_server_turn_ids": frontier_usage.turn_ids.clone(),
            "codex_app_server_thread_count": frontier_usage.thread_count(),
            "supervisor_session_reused": frontier_usage.session_reused(),
            "supervisor_resume_count": frontier_usage.thread_reuse_count(),
            "did_codex_read_full_mixmod_session": false,
            "did_codex_read_raw_logs": false,
            "artifact_files_read_by_codex": ["receipt.json", "report.md", "worktree.patch", "changes.patch", "tests.json", "metrics.json", "patch-comparison.json", "previous-worktree.patch"],
            "strategy_phases": ["codex_worker_brief", "codex_worker_decision_loop"],
            "codex_loop_exit": approval_action,
            "final_worker_mode": final_decision.worker_mode,
            "worker_modes": worker_modes,
            "patch_checkpoints": patch_checkpoint_metrics,
            "revision_attempts": opencode_calls.saturating_sub(1),
            "worker_brief": "worker-brief.json",
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
            "worker_run_dirs": worker_run_dirs,
            "final_worker_run_dir": display_path(root, &final_out),
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
            "require_local": get_bool(&final_metrics, "require_local").unwrap_or(false),
            "local_inference_verified": local_inference_verified,
            "gpu_activity_observed": gpu_activity_observed,
            "backend_activity_observed": backend_activity_observed,
            "local_worker_stdout_bytes": local_worker_stdout,
            "local_worker_stderr_bytes": local_worker_stderr,
            "local_worker_text_bytes": local_worker_stdout + local_worker_stderr,
            "artifact_byte_sizes": artifact_byte_sizes(&out_dir)?,
            "patch_bytes": final_patch.len() as u64,
            "changed_files": stats.files,
            "changed_file_count": stats.files.len(),
            "changed_line_count": stats.changed_line_count,
            "test_commands": task_spec.tests,
            "test_status": test_status,
            "final_status": final_status,
            "final_verdict": approval_action.clone(),
            "final_codex_action": approval_action,
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Default strategy used a fresh Codex app-server supervisor thread for each worker handoff and review turn.",
                "Codex controls the worker loop with approve, revise, or blocked/inconclusive stop decisions; direct supervisor editing is not part of this strategy.",
                "The worker backend was selected through the Mixmod worker settings."
            ]
        });
        write_pretty_json(
            &out_dir.join("metrics.json"),
            &metrics,
            "default strategy metrics",
        )?;
        atomic_write(
            &out_dir.join("report.md"),
            budgeted_report("exec", &metrics).as_bytes(),
        )?;

        println!(
            "Mixmod exec wrote artifacts to {}",
            display_path(root, &out_dir)
        );
        println!("status: {}", final_status);
        println!("report: {}", display_path(root, &out_dir.join("report.md")));
        println!(
            "patch: {}",
            display_path(root, &out_dir.join("final.patch"))
        );
        Ok(())
    }
}

fn ensure_worker_run_verified(out_dir: &Path, receipt: &Receipt, run_dir: &Path) -> Result<()> {
    let metrics = read_json_file(&run_dir.join("metrics.json"))?;
    if !get_bool(&metrics, "require_local").unwrap_or(false)
        || get_bool(&metrics, "local_inference_verified").unwrap_or(false)
    {
        return Ok(());
    }

    write_pretty_json(
        &out_dir.join("blocked-receipt.json"),
        receipt,
        "blocked worker receipt",
    )?;
    bail!(
        "local worker inference was required but could not be verified for {}",
        run_dir.display()
    )
}

fn run_final_tests(root: &Path, out_dir: &Path, tests: &[String]) -> Result<String> {
    let logs_dir = out_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create test logs dir {}", logs_dir.display()))?;
    let (status, _results, tests_json) = run_task_tests(root, &logs_dir, tests)?;
    write_pretty_json(
        &out_dir.join("final-tests.json"),
        &tests_json,
        "final test results",
    )?;
    write_pretty_json(&out_dir.join("tests.json"), &tests_json, "test results")?;
    Ok(status)
}

fn artifact_byte_sizes(dir: &Path) -> Result<Value> {
    let mut map = serde_json::Map::new();
    for name in [
        "worker-brief.json",
        "worker-task.json",
        "receipt.json",
        "task.json",
        "report.md",
        "session.jsonl",
        "worktree.patch",
        "changes.patch",
        PATCH_COMPARISON,
        PREVIOUS_WORKTREE_PATCH,
        "partial.patch",
        "tests.json",
        "metrics.json",
        "frontier-feedback.jsonl",
        "final.patch",
        "final-tests.json",
        "local-verification.json",
        SUPERVISOR_CONTROL_LOG,
    ] {
        let path = dir.join(name);
        if path.exists() {
            map.insert(name.to_string(), json!(file_len(&path)?));
        }
    }
    Ok(Value::Object(map))
}
