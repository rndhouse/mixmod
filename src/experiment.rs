use crate::*;

pub fn experiment_init(root: &Path, name: &str, fixture: Option<&Path>) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = root.join(".mixmod/experiments").join(name);
    let codex_only_dir = exp_dir.join("codex-only");
    let mixmod_dir = exp_dir.join("mixmod");
    let work_dir = exp_dir.join("work");
    for dir in [
        &codex_only_dir,
        &codex_only_dir.join("logs"),
        &mixmod_dir,
        &mixmod_dir.join("runs"),
        &work_dir,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }

    write_if_missing(&exp_dir.join("task.md"), task_md_template(name).as_bytes())?;
    write_pretty_json_if_missing(
        &exp_dir.join("task.json"),
        &task_json_template(name),
        "experiment task template",
    )?;
    write_if_missing(
        &exp_dir.join("README.md"),
        experiment_readme(name).as_bytes(),
    )?;
    write_if_missing(
        &codex_only_dir.join("notes.md"),
        codex_only_notes_template(name).as_bytes(),
    )?;
    write_if_missing(&codex_only_dir.join("final.patch"), b"")?;
    write_pretty_json_if_missing(
        &codex_only_dir.join("metrics.json"),
        &placeholder_experiment_metrics("codex-only"),
        "codex-only placeholder metrics",
    )?;
    write_if_missing(
        &mixmod_dir.join("notes.md"),
        mixmod_notes_template(name).as_bytes(),
    )?;
    write_if_missing(&mixmod_dir.join("final.patch"), b"")?;
    write_pretty_json_if_missing(
        &mixmod_dir.join("metrics.json"),
        &placeholder_experiment_metrics("codex-plus-mixmod"),
        "mixmod placeholder metrics",
    )?;
    if let Some(fixture) = fixture {
        let fixture_path = absolutize(root, fixture);
        seed_experiment_task_from_fixture(&fixture_path, &exp_dir, root)?;
        copy_fixture_workdir(
            &fixture_path,
            &work_dir.join("codex-only"),
            "codex-only",
            root,
        )?;
        copy_fixture_workdir(&fixture_path, &work_dir.join("mixmod"), "mixmod", root)?;
        copy_fixture_workdir(&fixture_path, &work_dir.join("default"), "default", root)?;
    }

    println!(
        "created experiment scaffold at {}",
        display_path(root, &exp_dir)
    );
    println!("task templates:");
    println!("  {}", display_path(root, &exp_dir.join("task.md")));
    println!("  {}", display_path(root, &exp_dir.join("task.json")));
    if fixture.is_some() {
        println!("isolated work dirs:");
        println!("  {}", display_path(root, &work_dir.join("codex-only")));
        println!("  {}", display_path(root, &work_dir.join("mixmod")));
        println!("  {}", display_path(root, &work_dir.join("default")));
    }
    Ok(())
}

pub fn experiment_record_codex_only(root: &Path, name: &str, task: &Path) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = root.join(".mixmod/experiments").join(name);
    if !exp_dir.exists() {
        experiment_init(root, name, None)?;
    }
    let task_path = absolutize(root, task);
    let task_bytes = file_len(&task_path).unwrap_or(0);
    let target = exp_dir.join("codex-only");
    fs::create_dir_all(&target)
        .with_context(|| format!("failed to create codex-only dir {}", target.display()))?;
    let work_dir = exp_dir.join("work/codex-only");
    if work_dir.exists() {
        write_agent_visible_task_file(&task_path, &work_dir.join("task.json"))?;
        return run_codex_only_baseline(root, name, &task_path, &target, &work_dir);
    }

    let patch = git_diff_with_untracked(root).unwrap_or_default();
    atomic_write(&target.join("final.patch"), patch.as_bytes())?;
    let stats = patch_stats(&patch);
    write_if_missing(
        &target.join("notes.md"),
        codex_only_notes_template(name).as_bytes(),
    )?;

    let metrics = json!({
        "kind": "codex-only",
        "recorded_at": Utc::now().to_rfc3339(),
        "task_file": display_path(root, &task_path),
        "task_bytes": task_bytes,
        "codex_token_usage": null,
        "codex_turns": null,
        "mixmod_delegations": 0,
        "artifact_files_read_by_codex": [],
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": task_bytes,
        "approximate_codex_output_bytes": patch.len() as u64,
        "local_worker_text_bytes": 0,
        "patch_bytes": patch.len() as u64,
        "changed_file_count": stats.files.len(),
        "changed_line_count": stats.changed_line_count,
        "tests_run": [],
        "test_status": "unknown",
        "final_status": "manual",
        "notes": [
            "Manual-assisted record. Fill notes.md with Codex-only turns, tests, and final outcome.",
            "Exact Codex token telemetry is unavailable unless added manually."
        ]
    });
    write_pretty_json(&target.join("metrics.json"), &metrics, "codex-only metrics")?;

    println!(
        "recorded Codex-only slot at {}",
        display_path(root, &target)
    );
    println!(
        "manual notes: {}",
        display_path(root, &target.join("notes.md"))
    );
    println!(
        "patch snapshot: {}",
        display_path(root, &target.join("final.patch"))
    );
    Ok(())
}

fn run_codex_only_baseline(
    root: &Path,
    name: &str,
    task_path: &Path,
    target: &Path,
    work_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(target.join("logs")).with_context(|| {
        format!(
            "failed to create codex-only logs dir {}",
            target.join("logs").display()
        )
    })?;
    if find_on_path("codex").is_none() {
        let mut extra = serde_json::Map::new();
        extra.insert("frontier_input_tokens".to_string(), Value::Null);
        extra.insert("frontier_output_tokens".to_string(), Value::Null);
        extra.insert("frontier_total_tokens".to_string(), Value::Null);
        let metrics = CodexOnlyMetrics {
            kind: "codex-only".to_string(),
            recorded_at: Utc::now().to_rfc3339(),
            final_status: "blocked".to_string(),
            test_status: "not_run".to_string(),
            blocker: Some("codex was not found on PATH".to_string()),
            extra,
        };
        write_pretty_json(&target.join("metrics.json"), &metrics, "codex-only metrics")?;
        bail!("codex was not found on PATH");
    }

    let (task_value, task_spec) = read_task_json(task_path)?;
    let prompt = codex_only_prompt(work_dir, &task_value)?;
    let config = load_config(root)?;
    let start = Instant::now();
    let run_start = Utc::now();
    let result = run_codex_app_server_turn(
        work_dir,
        target,
        "codex-only",
        &prompt,
        &config.frontier,
        CodexSandbox::WorkspaceWrite,
    )?;
    let patch = git_diff_with_untracked(work_dir).unwrap_or_default();
    atomic_write(&target.join("final.patch"), patch.as_bytes())?;
    let stats = patch_stats(&patch);
    let logs_dir = target.join("logs");
    let (test_status, test_results, tests_json) =
        run_task_tests(work_dir, &logs_dir, &task_spec.tests)?;
    write_pretty_json(
        &target.join("tests.json"),
        &tests_json,
        "codex-only test results",
    )?;
    write_if_missing(
        &target.join("notes.md"),
        codex_only_notes_template(name).as_bytes(),
    )?;
    let final_status =
        if result.exit_status == Some(0) && test_status == "passed" && !patch.trim().is_empty() {
            "success"
        } else {
            "needs_review"
        };
    let metrics = json!({
        "kind": "codex-only",
        "recorded_at": Utc::now().to_rfc3339(),
        "start_timestamp": run_start.to_rfc3339(),
        "end_timestamp": Utc::now().to_rfc3339(),
        "wall_clock_ms": start.elapsed().as_millis(),
        "task_file": display_path(root, task_path),
        "work_dir": display_path(root, work_dir),
        "codex_exit_status": result.exit_status,
        "frontier_model": result.model.clone(),
        "frontier_reasoning_effort": result.reasoning_effort.clone(),
        "frontier_input_tokens": result.usage.input_tokens,
        "frontier_output_tokens": result.usage.output_tokens,
        "frontier_reasoning_tokens": result.usage.reasoning_tokens,
        "frontier_total_tokens": result.usage.total_tokens,
        "frontier_cached_input_tokens": result.usage.cached_input_tokens,
        "frontier_input_bytes_fallback": result.input_bytes,
        "frontier_output_bytes_fallback": result.output_bytes,
        "codex_visible_bytes": result.input_bytes,
        "codex_token_usage": result.usage.total_tokens,
        "codex_turns": 1,
        "codex_calls": 1,
        "codex_backend": "app-server",
        "codex_app_server_thread_id": result.thread_id.clone(),
        "codex_app_server_turn_id": result.turn_id.clone(),
        "mixmod_delegations": 0,
        "artifact_files_read_by_codex": [],
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": result.input_bytes,
        "approximate_codex_output_bytes": result.output_bytes,
        "local_worker_text_bytes": 0,
        "patch_bytes": patch.len() as u64,
        "changed_files": stats.files,
        "changed_file_count": stats.files.len(),
        "changed_line_count": stats.changed_line_count,
        "tests_run": task_spec.tests,
        "test_results": test_results,
        "test_status": test_status,
        "final_status": final_status,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "stdout_bytes": result.stdout.len() as u64,
        "stderr_bytes": result.stderr.len() as u64,
        "notes": [
            "Automated Codex-only baseline ran in an isolated fixture work directory.",
            "Codex was invoked through app-server with an experiment-local CODEX_HOME."
        ]
    });
    write_pretty_json(&target.join("metrics.json"), &metrics, "codex-only metrics")?;
    println!(
        "recorded automated Codex-only baseline at {}",
        display_path(root, target)
    );
    Ok(())
}

pub fn experiment_record_mixmod(root: &Path, name: &str, task: &Path) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = root.join(".mixmod/experiments").join(name);
    if !exp_dir.exists() {
        experiment_init(root, name, None)?;
    }
    let task_path = absolutize(root, task);
    let mixmod_dir = exp_dir.join("mixmod");
    fs::create_dir_all(mixmod_dir.join("runs")).with_context(|| {
        format!(
            "failed to create Mixmod experiment runs dir {}",
            mixmod_dir.join("runs").display()
        )
    })?;

    let task_json = if task_path.extension() == Some(OsStr::new("json")) {
        let bytes = fs::read(&task_path)
            .with_context(|| format!("failed to read {}", task_path.display()))?;
        serde_json::from_slice::<Value>(&bytes)
            .with_context(|| format!("failed to parse {}", task_path.display()))?
    } else {
        let body = fs::read_to_string(&task_path)
            .with_context(|| format!("failed to read {}", task_path.display()))?;
        json!({
            "title": name,
            "instructions": body,
            "files": [],
            "tests": [],
            "constraints": ["Keep the patch focused and report tests clearly."],
            "acceptance": []
        })
    };
    let prepared_task = mixmod_dir.join("task.json");
    write_pretty_json(
        &prepared_task,
        &task_json,
        "prepared Mixmod experiment task",
    )?;

    let out_dir = mixmod_dir.join("runs").join(make_run_id("mixmod"));
    let config = load_config(root)?;
    let runner = ShellOpenCodeRunner::new(config);
    let receipt = run_mixmod_task(
        root,
        DelegationMode::Patch,
        &prepared_task,
        &out_dir,
        &runner,
    )?;

    let final_patch = mixmod_dir.join("final.patch");
    fs::copy(out_dir.join("changes.patch"), &final_patch)
        .with_context(|| format!("failed to copy {}", final_patch.display()))?;

    let run_metrics_path = out_dir.join("metrics.json");
    let run_metrics_value = read_json_file(&run_metrics_path)?;
    let compact_artifact_bytes = [
        "receipt.json",
        "report.md",
        "changes.patch",
        "tests.json",
        "metrics.json",
    ]
    .iter()
    .map(|name| file_len(&out_dir.join(name)).unwrap_or(0))
    .sum::<u64>();
    let local_worker_text_bytes = get_u64(&run_metrics_value, "stdout_bytes").unwrap_or(0)
        + get_u64(&run_metrics_value, "stderr_bytes").unwrap_or(0)
        + get_u64(&run_metrics_value, "session_bytes").unwrap_or(0);
    let exp_metrics = json!({
        "kind": "codex-plus-mixmod",
        "recorded_at": Utc::now().to_rfc3339(),
        "task_file": display_path(root, &task_path),
        "prepared_task": display_path(root, &prepared_task),
        "run_dir": display_path(root, &out_dir),
        "run_receipt": receipt,
        "run_metrics": run_metrics_value,
        "codex_token_usage": null,
        "codex_turns": null,
        "mixmod_delegations": 1,
        "artifact_files_read_by_codex": ["receipt.json", "report.md", "changes.patch", "tests.json", "metrics.json"],
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": compact_artifact_bytes,
        "approximate_codex_output_bytes": null,
        "local_worker_text_bytes": local_worker_text_bytes,
        "patch_bytes": get_u64(&run_metrics_value, "patch_bytes").unwrap_or(0),
        "changed_file_count": get_u64(&run_metrics_value, "changed_file_count").unwrap_or(0),
        "changed_line_count": get_u64(&run_metrics_value, "changed_line_count").unwrap_or(0),
        "tests_run": [],
        "test_status": get_str(&run_metrics_value, "test_status").unwrap_or("unknown").to_string(),
        "final_status": get_str(&json!(receipt), "status").unwrap_or("unknown").to_string(),
        "notes": [
            "This prototype assumes Codex reviews compact Mixmod artifacts first.",
            "Exact Codex token telemetry is unavailable unless added manually."
        ]
    });
    write_pretty_json(
        &mixmod_dir.join("metrics.json"),
        &exp_metrics,
        "Mixmod experiment metrics",
    )?;
    write_if_missing(
        &mixmod_dir.join("notes.md"),
        mixmod_notes_template(name).as_bytes(),
    )?;

    println!(
        "recorded Codex+Mixmod slot at {}",
        display_path(root, &mixmod_dir)
    );
    println!("run artifacts: {}", display_path(root, &out_dir));
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct DefaultRunOptions {
    pub require_local: bool,
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
        let exp_dir = root.join(".mixmod/experiments").join(name);
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

        let config = load_config(&work_dir)?;
        let frontier = config.frontier.clone();
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
        let task_file = work_dir.join("task.json");
        let canonical_task = exp_dir.join("task.json");
        if canonical_task.exists() {
            write_agent_visible_task_file(&canonical_task, &task_file)?;
        } else {
            ensure_agent_visible_task_file(&task_file)?;
        }
        let (_, task_spec) = read_task_json(&task_file)?;
        let runner = ShellOpenCodeRunner::new(config);
        let feedback_path = default_dir.join("frontier-feedback.jsonl");
        let worker_brief = run_frontier_brief_turn(&work_dir, &default_dir, &task_file, &frontier)?;
        write_pretty_json(
            &default_dir.join("worker-brief.json"),
            &worker_brief.brief,
            "worker brief",
        )?;
        append_jsonl(&feedback_path, &worker_brief.record)?;

        let worker_task = write_worker_brief_task(&task_file, &worker_brief.brief, &default_dir)?;
        let proposal_out = work_dir.join(".mixmod/runs/default-proposal");
        let proposal_receipt = run_mixmod_task_with_options(
            &work_dir,
            DelegationMode::Patch,
            &worker_task,
            &proposal_out,
            &runner,
            options.require_local,
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
                    final_out.join("changes.patch"),
                    final_out.join("tests.json"),
                    final_out.join("metrics.json"),
                ];
                let supervisor_control_path = final_out.join(SUPERVISOR_CONTROL_LOG);
                if supervisor_control_path.exists() {
                    artifact_paths.push(supervisor_control_path);
                }
                let decision = run_frontier_feedback_turn(
                    &work_dir,
                    &default_dir,
                    &label,
                    &artifact_paths,
                    "Decide the next Codex/OpenCode loop action. Use approve only when the local-worker result is acceptable. Prefer revise after failed or empty worker attempts, with a concrete next instruction. Use stop only to record a blocked or inconclusive local-worker result when no useful OpenCode path remains; do not solve by directly editing files.",
                    &frontier,
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
                            "Codex requested worker_mode=continue, but Mixmod could not resolve the previous OpenCode session id from {}",
                            final_out.join("metrics.json").display()
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
                    final_out = work_dir.join(".mixmod/runs").join(revision_out_name);
                    let revision_receipt = run_mixmod_task_with_session(
                        &work_dir,
                        DelegationMode::Patch,
                        &revision_task,
                        &final_out,
                        &runner,
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
                    opencode_calls += 1;
                    worker_run_dirs.push(final_out.clone());
                    active_opencode_session_id = read_opencode_session_id_from_metrics(&final_out)?;
                    pending_supervisor_control =
                        supervisor_control_decision_from_metrics(&final_out)?;
                }
            }
        };

        let final_patch = git_diff_with_untracked(&work_dir)?;
        atomic_write(&default_dir.join("final.patch"), final_patch.as_bytes())?;
        let stats = patch_stats(&final_patch);
        let test_status =
            run_tests_for_experiment(&work_dir, &default_dir, "final", &task_spec.tests)?;
        copy_budgeted_artifacts(root, &default_dir, &final_out)?;

        let worker_metrics = worker_run_dirs
            .iter()
            .map(|dir| read_json_file(&dir.join("metrics.json")))
            .collect::<Result<Vec<_>>>()?;
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
        let metrics = json!({
            "kind": "mixmod-default-strategy",
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "frontier_model": frontier.model,
            "frontier_input_tokens": frontier_usage.input_tokens,
            "frontier_reasoning_effort": frontier.reasoning_effort,
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
            "artifact_files_read_by_codex": ["receipt.json", "report.md", "changes.patch", "tests.json", "metrics.json"],
            "strategy_phases": ["codex_worker_brief", "codex_open_code_decision_loop"],
            "codex_loop_exit": approval_action,
            "final_worker_mode": final_decision.worker_mode,
            "worker_modes": worker_modes,
            "revision_attempts": opencode_calls.saturating_sub(1),
            "worker_brief": "worker-brief.json",
            "worker_task": display_path(root, &worker_task),
            "worker_brief_output_tokens": worker_brief.output_tokens,
            "mixmod_delegations": opencode_calls,
            "opencode_calls": opencode_calls,
            "opencode_backend": get_str(&final_metrics, "opencode_backend").unwrap_or("unknown"),
            "opencode_server_url": get_str(&final_metrics, "opencode_server_url"),
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
            "test_commands": task_spec.tests,
            "test_status": test_status,
            "final_status": final_status,
            "final_verdict": approval_action.clone(),
            "final_codex_action": approval_action,
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Default strategy used a fresh Codex app-server supervisor thread for each worker handoff and review turn.",
                "Codex controls the OpenCode loop with approve, revise, or blocked/inconclusive stop decisions; direct Codex editing is not part of this strategy.",
                "OpenCode was required to use explicit local Qwen 3.6 model selection.",
                "If the worker times out, run `mixmod experiment recover <name> --require-local` to restart from worker-task.json."
            ]
        });
        write_pretty_json(
            &default_dir.join("metrics.json"),
            &metrics,
            "default experiment metrics",
        )?;
        atomic_write(
            &default_dir.join("report.md"),
            budgeted_report(name, &metrics).as_bytes(),
        )?;
        println!(
            "default strategy experiment wrote {}",
            display_path(root, &default_dir.join("report.md"))
        );
        Ok(())
    }
}

pub fn experiment_recover(root: &Path, name: &str, require_local: bool) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = root.join(".mixmod/experiments").join(name);
    let default_work_dir = exp_dir.join("work/default");
    let legacy_work_dir = exp_dir.join("work/budgeted");
    let work_dir = if default_work_dir.exists() {
        default_work_dir
    } else {
        legacy_work_dir
    };
    if !work_dir.exists() {
        bail!(
            "default strategy work directory is missing: {}",
            display_path(root, &work_dir)
        );
    }
    ensure_project_state(&work_dir, false)?;

    let default_dir = exp_dir.join("default");
    let worker_task = default_dir.join("worker-task.json");
    if !worker_task.exists() {
        bail!(
            "cannot recover without {}; run `mixmod experiment run-default {name}` through the worker-brief phase first",
            display_path(root, &worker_task)
        );
    }

    let config = load_config(&work_dir)?;
    let runner = ShellOpenCodeRunner::new(config);
    let recovery_id = make_run_id("recovery");
    let out_dir = work_dir.join(".mixmod/runs").join(&recovery_id);
    let receipt = run_mixmod_task_with_options(
        &work_dir,
        DelegationMode::Patch,
        &worker_task,
        &out_dir,
        &runner,
        require_local,
    )?;

    let recovery_dir = default_dir.join("recoveries").join(&recovery_id);
    fs::create_dir_all(&recovery_dir)
        .with_context(|| format!("failed to create recovery dir {}", recovery_dir.display()))?;
    copy_budgeted_artifacts(root, &recovery_dir, &out_dir)?;
    for name in ["worker-brief.json", "worker-task.json"] {
        let source = default_dir.join(name);
        if source.exists() {
            fs::copy(&source, recovery_dir.join(name)).with_context(|| {
                format!(
                    "failed to copy recovery artifact {} to {}",
                    source.display(),
                    recovery_dir.join(name).display()
                )
            })?;
        }
    }
    let final_patch = git_diff_with_untracked(&work_dir).unwrap_or_default();
    atomic_write(&recovery_dir.join("final.patch"), final_patch.as_bytes())?;
    let run_metrics = read_json_file(&out_dir.join("metrics.json"))?;
    let recovery_summary = json!({
        "kind": "mixmod-default-recovery",
        "recorded_at": Utc::now().to_rfc3339(),
        "experiment": name,
        "recovery_id": recovery_id,
        "work_dir": display_path(root, &work_dir),
        "run_dir": display_path(root, &out_dir),
        "recovery_dir": display_path(root, &recovery_dir),
        "receipt": receipt,
        "run_metrics": run_metrics,
        "final_patch": display_path(root, &recovery_dir.join("final.patch")),
        "notes": [
            "Recovery restarts OpenCode from the saved worker-task.json.",
            "Codex review is not run automatically; inspect recovery artifacts before accepting."
        ]
    });
    write_pretty_json(
        &recovery_dir.join("recovery.json"),
        &recovery_summary,
        "recovery summary",
    )?;
    write_pretty_json(
        &default_dir.join("latest-recovery.json"),
        &recovery_summary,
        "latest recovery summary",
    )?;
    println!(
        "recovery wrote {}",
        display_path(root, &recovery_dir.join("recovery.json"))
    );
    println!("status: {}", receipt.status);
    Ok(())
}

fn copy_budgeted_artifacts(root: &Path, budgeted_dir: &Path, final_out: &Path) -> Result<()> {
    for name in [
        "worker-brief.json",
        "worker-task.json",
        "receipt.json",
        "task.json",
        "report.md",
        "session.jsonl",
        "changes.patch",
        "partial.patch",
        "tests.json",
        "metrics.json",
        "local-verification.json",
        SUPERVISOR_CONTROL_LOG,
    ] {
        let source = final_out.join(name);
        if source.exists() {
            fs::copy(&source, budgeted_dir.join(name)).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    display_path(root, &source),
                    display_path(root, &budgeted_dir.join(name))
                )
            })?;
        }
    }
    let logs_dir = budgeted_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create artifact logs dir {}", logs_dir.display()))?;
    for name in [
        "opencode.stdout.txt",
        "opencode.stderr.txt",
        "nvidia-smi-before.txt",
        "nvidia-smi-during.txt",
        "nvidia-smi-after.txt",
        "ollama-ps.txt",
        "heartbeat.jsonl",
    ] {
        let source = final_out.join("logs").join(name);
        if source.exists() {
            let target = logs_dir.join(name);
            fs::copy(&source, &target).with_context(|| {
                format!(
                    "failed to copy worker log {} to {}",
                    source.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn run_tests_for_experiment(
    work_dir: &Path,
    target_dir: &Path,
    label: &str,
    tests: &[String],
) -> Result<String> {
    let logs_dir = target_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create test logs dir {}", logs_dir.display()))?;
    let (status, _results, tests_json) = run_task_tests(work_dir, &logs_dir, tests)?;
    write_pretty_json(
        &target_dir.join(format!("{label}-tests.json")),
        &tests_json,
        "experiment test results",
    )?;
    write_pretty_json(&target_dir.join("tests.json"), &tests_json, "test results")?;
    Ok(status)
}

pub(crate) fn write_worker_brief_task(
    task_path: &Path,
    brief: &Value,
    default_dir: &Path,
) -> Result<PathBuf> {
    let original = read_json_file(task_path)?;
    let original = agent_visible_task_value(&original);
    let typed_brief = WorkerBrief::from_value(brief);
    let handoff = typed_brief.handoff.as_deref().unwrap_or_else(|| {
        if brief_has_legacy_guidance(brief) {
            "guided"
        } else {
            "as_given"
        }
    });
    let explicit_focus_files =
        first_non_empty_string_array(brief, &["files", "focus_files", "target_files"]);
    let target_files = non_empty_or(
        explicit_focus_files.clone(),
        get_string_array(&original, "files"),
    );
    let required_tests = non_empty_or(
        merged_string_arrays(brief, &["tests", "required_tests"]),
        get_string_array(&original, "tests"),
    );
    let checks = merged_string_arrays(
        brief,
        &[
            "checks",
            "must_check",
            "required_checks",
            "acceptance_checks",
        ],
    );
    let avoid = get_string_array(brief, "avoid");
    let mut constraints = get_string_array(&original, "constraints");
    constraints.extend(
        get_string_array(brief, "constraints")
            .into_iter()
            .map(|constraint| format!("Codex constraint: {constraint}")),
    );
    constraints.extend(avoid.iter().map(|item| format!("Avoid: {item}")));
    constraints.push(
        "Treat the original task JSON as primary; Codex handoff is supplemental.".to_string(),
    );
    constraints.push("Keep stdout compact.".to_string());
    constraints.sort();
    constraints.dedup();

    let original_instructions = get_str(&original, "instructions").unwrap_or("");
    let brief_json =
        serde_json::to_string_pretty(brief).context("failed to serialize Codex worker brief")?;
    let title = get_str(&original, "title").unwrap_or("Mixmod task");
    let codex_message = codex_message_to_worker(brief, handoff);
    let acceptance = non_empty_or(checks.clone(), get_string_array(&original, "acceptance"));
    let expect_patch = typed_brief.expect_patch.unwrap_or(handoff != "blocked");

    let worker_task = json!({
        "title": format!("Mixmod handoff: {title}"),
        "instructions": format!(
            "Original task instructions:\n{original_instructions}\n\nCodex message to OpenCode:\n{codex_message}\n\nCodex handoff JSON:\n{brief_json}"
        ),
        "expect_patch": expect_patch,
        "files": target_files,
        "tests": required_tests,
        "constraints": constraints,
        "acceptance": acceptance,
        "context": {
            "expect_patch": expect_patch,
            "worker_brief": brief
        }
    });
    let path = default_dir.join("worker-task.json");
    write_pretty_json(&path, &worker_task, "worker task")?;
    Ok(path)
}

fn codex_message_to_worker(brief: &Value, handoff: &str) -> String {
    if let Some(message) = get_str(brief, "message_to_worker")
        .or_else(|| get_str(brief, "message"))
        .filter(|message| !message.trim().is_empty())
    {
        return message.trim().to_string();
    }
    if handoff == "as_given" && !brief_has_legacy_guidance(brief) {
        return "Proceed from the original task.".to_string();
    }

    let mut lines = Vec::new();
    if let Some(supplement) = get_str(brief, "supplement")
        .or_else(|| get_str(brief, "objective"))
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(supplement.trim().to_string());
    }
    append_handoff_list(
        &mut lines,
        "Files",
        &first_non_empty_string_array(brief, &["files", "focus_files", "target_files"]),
    );
    append_handoff_list(
        &mut lines,
        "Checks",
        &merged_string_arrays(
            brief,
            &[
                "checks",
                "must_check",
                "required_checks",
                "acceptance_checks",
            ],
        ),
    );
    append_handoff_list(
        &mut lines,
        "Notes",
        &get_string_array(brief, "implementation_steps"),
    );
    append_handoff_list(&mut lines, "Avoid", &get_string_array(brief, "avoid"));
    if let Some(risk) = get_str(brief, "risk").filter(|value| !value.trim().is_empty()) {
        lines.push(format!("Risk: {}", risk.trim()));
    }
    if lines.is_empty() {
        "Proceed from the original task.".to_string()
    } else {
        lines.join("\n")
    }
}

pub(crate) fn write_revision_task(
    task_path: &Path,
    default_dir: &Path,
    experiment_name: &str,
    decision: &FrontierFeedbackTurn,
    revision_index: u64,
) -> Result<PathBuf> {
    let task_value = read_json_file(task_path)?;
    let task_value = agent_visible_task_value(&task_value);
    let work_dir = task_path.parent().unwrap_or_else(|| Path::new("."));
    let (repo_focus_files, artifact_focus_files) =
        split_worker_focus_files(work_dir, default_dir, &decision.focus_files);
    let focus_files = non_empty_or(
        repo_focus_files.clone(),
        get_string_array(&task_value, "files"),
    );
    let artifact_note = if artifact_focus_files.is_empty() {
        String::new()
    } else {
        format!(
            "\nMixmod artifact references from Codex, not repo source files: {:?}\nDo not read these from the repo root; use the current task text and compact artifacts instead.",
            artifact_focus_files
        )
    };
    let focus_note = format!(
        "Repo focus files: {:?}{artifact_note}",
        if repo_focus_files.is_empty() {
            focus_files.clone()
        } else {
            repo_focus_files.clone()
        }
    );
    let acceptance = non_empty_or(
        decision.required_checks.clone(),
        get_string_array(&task_value, "acceptance"),
    );
    let original_instructions = get_str(&task_value, "instructions").unwrap_or("Revise the patch.");
    let instructions = if decision.worker_mode == "context_focus" {
        format!(
            "Original task instructions:\n{original_instructions}\n\nCodex requested worker_mode=context_focus.\nThis starts a new OpenCode session on the current worktree.\nTreat this as a fresh focused worker attempt and ignore previous worker reasoning unless it is repeated here.\n\nCodex message to OpenCode:\n{}\n\n{focus_note}\nRequired checks: {:?}\nIf checks cannot run because of local environment problems, make the code/test edit first and report the blocker compactly.",
            decision.hint, decision.required_checks
        )
    } else {
        format!(
            "{original_instructions}\n\nCodex decision: revise\nWorker mode: continue\nSame OpenCode session should be reused when available.\nMessage to OpenCode: {}\n{focus_note}\nRequired checks: {:?}\nContinue work from the current working tree and return compact artifacts for Codex review.",
            decision.hint, decision.required_checks
        )
    };
    let revision = json!({
        "title": format!("Revision {}: {}", revision_index, get_str(&task_value, "title").unwrap_or(experiment_name)),
        "instructions": instructions,
        "expect_patch": true,
        "worker_mode": decision.worker_mode,
        "files": focus_files,
        "tests": get_string_array(&task_value, "tests"),
        "constraints": ["Keep the revision focused.", "Do not paste long logs."],
        "acceptance": acceptance,
        "context": {
            "expect_patch": true,
            "codex_focus_files": decision.focus_files,
            "repo_focus_files": repo_focus_files,
            "mixmod_artifact_refs": artifact_focus_files
        }
    });
    let path = if revision_index == 1 {
        default_dir.join("revision-task.json")
    } else {
        default_dir.join(format!("revision-task-{revision_index}.json"))
    };
    write_pretty_json(&path, &revision, "revision task")?;
    if revision_index != 1 {
        write_pretty_json(
            &default_dir.join("revision-task.json"),
            &revision,
            "latest revision task",
        )?;
    }
    Ok(path)
}

fn split_worker_focus_files(
    work_dir: &Path,
    default_dir: &Path,
    requested: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut repo_files = Vec::new();
    let mut artifact_refs = Vec::new();
    for raw in requested {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        match classify_worker_focus_file(work_dir, default_dir, trimmed) {
            WorkerFocusFile::Repo(path) => push_unique(&mut repo_files, path),
            WorkerFocusFile::Artifact(path) => push_unique(&mut artifact_refs, path),
        }
    }
    (repo_files, artifact_refs)
}

enum WorkerFocusFile {
    Repo(String),
    Artifact(String),
}

fn classify_worker_focus_file(work_dir: &Path, default_dir: &Path, raw: &str) -> WorkerFocusFile {
    let normalized = raw.trim_start_matches("./").replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(work_dir) {
            let relative = path_to_repo_string(relative);
            if is_artifact_focus_ref(&relative) {
                WorkerFocusFile::Artifact(normalized)
            } else {
                WorkerFocusFile::Repo(relative)
            }
        } else {
            let _ = path.strip_prefix(default_dir);
            WorkerFocusFile::Artifact(normalized)
        }
    } else if normalized.starts_with("../")
        || normalized.contains("/../")
        || normalized.starts_with(".mixmod/")
        || normalized.starts_with(".codex/")
        || is_artifact_focus_ref(&normalized)
    {
        WorkerFocusFile::Artifact(normalized)
    } else {
        WorkerFocusFile::Repo(normalized)
    }
}

fn is_artifact_focus_ref(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(path);
    file_name == "worker-task.json"
        || file_name == "worker-brief.json"
        || file_name == "frontier-feedback.jsonl"
        || file_name == "receipt.json"
        || file_name == "metrics.json"
        || file_name == "changes.patch"
        || file_name == "tests.json"
        || file_name == "report.md"
        || file_name == "session.jsonl"
        || file_name == "revision-task.json"
        || (file_name.starts_with("revision-task-") && file_name.ends_with(".json"))
}

fn path_to_repo_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn push_unique(items: &mut Vec<String>, item: String) {
    if !items.contains(&item) {
        items.push(item);
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
    let metrics = read_json_file(&run_dir.join("metrics.json"))?;
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

fn brief_has_legacy_guidance(brief: &Value) -> bool {
    get_str(brief, "message_to_worker").is_some()
        || get_str(brief, "message").is_some()
        || get_str(brief, "supplement").is_some()
        || get_str(brief, "objective").is_some()
        || !get_string_array(brief, "files").is_empty()
        || !get_string_array(brief, "checks").is_empty()
        || !get_string_array(brief, "focus_files").is_empty()
        || !get_string_array(brief, "target_files").is_empty()
        || !get_string_array(brief, "implementation_steps").is_empty()
        || !get_string_array(brief, "acceptance_checks").is_empty()
        || !get_string_array(brief, "required_checks").is_empty()
        || !get_string_array(brief, "required_tests").is_empty()
        || !get_string_array(brief, "tests").is_empty()
        || !get_string_array(brief, "constraints").is_empty()
        || get_str(brief, "risk").is_some()
}

fn append_handoff_list(lines: &mut Vec<String>, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(items.iter().map(|item| format!("- {item}")));
}

fn non_empty_or<T>(value: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if value.is_empty() { fallback } else { value }
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
        &budgeted_dir.join("metrics.json"),
        &metrics,
        "default blocker metrics",
    )?;
    atomic_write(
        &budgeted_dir.join("report.md"),
        format!("# Mixmod Default Strategy Blocked\n\n{blocker}\n").as_bytes(),
    )?;
    println!(
        "default strategy blocked: {}",
        display_path(root, &budgeted_dir.join("report.md"))
    );
    Ok(())
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
        "changes.patch",
        "partial.patch",
        "tests.json",
        "metrics.json",
        "frontier-feedback.jsonl",
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

fn experiment_readme(name: &str) -> String {
    format!(
        r#"# Mixmod Experiment: {name}

This directory compares one small code-change task in two modes:

1. Codex-only: Codex performs the change directly.
2. Mixmod default: Codex emits a compact executable worker handoff, OpenCode implements locally from the original task plus that handoff, and Codex reviews compact artifacts.

Suggested workflow:

```sh
mixmod experiment record-codex-only {name} --task .mixmod/experiments/{name}/task.md
mixmod experiment run-default {name} --require-local
mixmod experiment report {name}
```

Fill `codex-only/notes.md` with manual baseline notes when exact telemetry is unavailable. Mixmod default metrics are written under `default/`.
"#
    )
}

fn task_md_template(name: &str) -> String {
    format!(
        r#"# {name}

## Task

Describe one bounded code-change task.

## Relevant files

- TBD

## Acceptance

- TBD

## Tests

- TBD
"#
    )
}

fn task_json_template(name: &str) -> Value {
    json!({
        "title": name,
        "instructions": "Describe one bounded code-change task.",
        "files": [],
        "tests": [],
        "constraints": [
            "Keep the patch focused.",
            "Report tests clearly.",
            "Do not paste long logs."
        ],
        "acceptance": []
    })
}

fn codex_only_notes_template(name: &str) -> String {
    format!(
        r#"# Codex-only Notes: {name}

Record:
- Codex turns:
- Codex token usage, if available:
- Tests run:
- Final status:
- Notes:
"#
    )
}

fn mixmod_notes_template(name: &str) -> String {
    format!(
        r#"# Mixmod Default Notes: {name}

Record:
- Codex turns:
- Mixmod delegations:
- Artifacts read by Codex:
- Whether Codex read `session.jsonl` or raw logs:
- Codex token usage, if available:
- Tests run:
- Final status:
- Notes:
"#
    )
}

pub(crate) fn placeholder_experiment_metrics(kind: &str) -> Value {
    json!({
        "kind": kind,
        "recorded_at": null,
        "codex_token_usage": null,
        "codex_turns": null,
        "mixmod_delegations": if kind == "codex-plus-mixmod" { 1 } else { 0 },
        "artifact_files_read_by_codex": [],
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": null,
        "approximate_codex_output_bytes": null,
        "local_worker_text_bytes": null,
        "patch_bytes": 0,
        "changed_file_count": 0,
        "changed_line_count": 0,
        "tests_run": [],
        "test_status": "unknown",
        "final_status": "unknown",
        "notes": ["Telemetry unavailable until this slot is recorded."]
    })
}

fn copy_fixture_workdir(fixture: &Path, target: &Path, label: &str, root: &Path) -> Result<()> {
    if !fixture.is_dir() {
        bail!("fixture path is not a directory: {}", fixture.display());
    }
    if target.exists()
        && fs::read_dir(target)
            .with_context(|| format!("failed to inspect work dir {}", target.display()))?
            .next()
            .is_some()
    {
        println!(
            "unchanged {} work dir {}",
            label,
            display_path(root, target)
        );
        return Ok(());
    }
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create {label} work dir {}", target.display()))?;
    copy_dir_contents(fixture, target)?;
    init_fixture_git_repo(target)?;
    println!("created {} work dir {}", label, display_path(root, target));
    Ok(())
}

fn seed_experiment_task_from_fixture(fixture: &Path, exp_dir: &Path, root: &Path) -> Result<()> {
    let fixture_task = fixture.join("task.json");
    if !fixture_task.exists() {
        return Ok(());
    }
    let value = read_json_file(&fixture_task)?;
    write_pretty_json(&exp_dir.join("task.json"), &value, "fixture task")?;
    atomic_write(
        &exp_dir.join("task.md"),
        task_markdown_from_json(&value).as_bytes(),
    )?;
    println!(
        "seeded experiment task from {}",
        display_path(root, &fixture_task)
    );
    Ok(())
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read directory {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source.display()))?;
        let source_path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".mixmod" || name == ".codex" {
            continue;
        }
        let target_path = target.join(name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("failed to create directory {}", target_path.display()))?;
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn init_fixture_git_repo(target: &Path) -> Result<()> {
    run_git(target, &["init"])?;
    run_git(target, &["config", "user.email", "mixmod@example.invalid"])?;
    run_git(target, &["config", "user.name", "Mixmod Fixture"])?;
    run_git(target, &["add", "."])?;
    run_git(target, &["commit", "-m", "fixture baseline"])?;
    Ok(())
}

fn run_git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run git {} in {}", args.join(" "), root.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

pub(crate) fn validate_experiment_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        bail!(
            "invalid experiment name `{name}`; use ASCII letters, numbers, dot, underscore, or hyphen"
        );
    }
    Ok(())
}
