use crate::*;

use super::init::experiment_init;
use super::util::{
    codex_only_notes_template, experiment_dir, mixmod_notes_template, validate_experiment_name,
};

pub fn experiment_record_codex_only(root: &Path, name: &str, task: &Path) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = experiment_dir(root, name);
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
        write_agent_visible_task_file(&task_path, &work_dir.join(TASK_JSON))?;
        return run_codex_only_baseline(root, name, &task_path, &target, &work_dir);
    }

    let patch = git_diff_with_untracked(root).unwrap_or_default();
    atomic_write(&target.join(FINAL_PATCH), patch.as_bytes())?;
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
        "final_status": "manual",
        "notes": [
            "Manual-assisted record. Fill notes.md with Codex-only turns and final outcome.",
            "Exact Codex token telemetry is unavailable unless added manually."
        ]
    });
    write_pretty_json(&target.join(METRICS_JSON), &metrics, "codex-only metrics")?;

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
        display_path(root, &target.join(FINAL_PATCH))
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
        extra.insert("supervisor_input_tokens".to_string(), Value::Null);
        extra.insert("supervisor_output_tokens".to_string(), Value::Null);
        extra.insert("supervisor_total_tokens".to_string(), Value::Null);
        let metrics = CodexOnlyMetrics {
            kind: "codex-only".to_string(),
            recorded_at: Utc::now().to_rfc3339(),
            final_status: "blocked".to_string(),
            blocker: Some("codex was not found on PATH".to_string()),
            extra,
        };
        write_pretty_json(&target.join(METRICS_JSON), &metrics, "codex-only metrics")?;
        bail!("codex was not found on PATH");
    }

    let (task_value, _) = read_task_json(task_path)?;
    let prompt = codex_only_prompt(work_dir, &task_value)?;
    let config = load_config(root)?;
    let start = Instant::now();
    let run_start = Utc::now();
    let sandbox = codex_only_sandbox_from_env()?;
    let result = run_codex_app_server_turn(
        work_dir,
        target,
        "codex-only",
        &prompt,
        &config.supervisor,
        sandbox,
    )?;
    let patch = git_diff_with_untracked(work_dir).unwrap_or_default();
    atomic_write(&target.join(FINAL_PATCH), patch.as_bytes())?;
    let stats = patch_stats(&patch);
    write_if_missing(
        &target.join("notes.md"),
        codex_only_notes_template(name).as_bytes(),
    )?;
    let final_status = if result.exit_status == Some(0) && !patch.trim().is_empty() {
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
        "supervisor_model": result.model.clone(),
        "supervisor_reasoning_effort": result.reasoning_effort.clone(),
        "supervisor_input_tokens": result.usage.input_tokens,
        "supervisor_output_tokens": result.usage.output_tokens,
        "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
        "supervisor_total_tokens": result.usage.total_tokens,
        "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
        "supervisor_input_bytes_fallback": result.input_bytes,
        "supervisor_output_bytes_fallback": result.output_bytes,
        "codex_visible_bytes": result.input_bytes,
        "codex_token_usage": result.usage.total_tokens,
        "codex_turns": 1,
        "codex_calls": 1,
        "codex_backend": "app-server",
        "codex_sandbox": sandbox.as_thread_arg(),
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
        "final_status": final_status,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "stdout_bytes": result.stdout.len() as u64,
        "stderr_bytes": result.stderr.len() as u64,
        "notes": [
            "Automated Codex-only baseline ran in an isolated fixture work directory.",
            "Codex was invoked through app-server with an experiment-local CODEX_HOME.",
            "Mixmod did not execute project tests in this arm."
        ]
    });
    write_pretty_json(&target.join(METRICS_JSON), &metrics, "codex-only metrics")?;
    println!(
        "recorded automated Codex-only baseline at {}",
        display_path(root, target)
    );
    Ok(())
}

fn codex_only_sandbox_from_env() -> Result<CodexSandbox> {
    match env::var("MIXMOD_CODEX_ONLY_SANDBOX") {
        Ok(value) if value == "danger-full-access" => Ok(CodexSandbox::DangerFullAccess),
        Ok(value) if value == "workspace-write" => Ok(CodexSandbox::WorkspaceWrite),
        Ok(value) => bail!(
            "unsupported MIXMOD_CODEX_ONLY_SANDBOX value `{value}`; expected workspace-write or danger-full-access"
        ),
        Err(env::VarError::NotPresent) => Ok(CodexSandbox::WorkspaceWrite),
        Err(error) => Err(error).context("failed to read MIXMOD_CODEX_ONLY_SANDBOX"),
    }
}

pub fn experiment_record_mixmod(root: &Path, name: &str, task: &Path) -> Result<()> {
    validate_experiment_name(name)?;
    let exp_dir = experiment_dir(root, name);
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
    let prepared_task = mixmod_dir.join(TASK_JSON);
    write_pretty_json(
        &prepared_task,
        &task_json,
        "prepared Mixmod experiment task",
    )?;

    let out_dir = mixmod_dir.join("runs").join(make_run_id("mixmod"));
    let config = load_config(root)?;
    let runner = worker_harness_for_config(config);
    let receipt = run_mixmod_task(
        root,
        DelegationMode::Patch,
        &prepared_task,
        &out_dir,
        runner.as_ref(),
    )?;

    let final_patch = mixmod_dir.join(FINAL_PATCH);
    let source_patch = {
        let worktree_patch = out_dir.join(WORKTREE_PATCH);
        if worktree_patch.exists() {
            worktree_patch
        } else {
            out_dir.join(CHANGES_PATCH)
        }
    };
    fs::copy(&source_patch, &final_patch)
        .with_context(|| format!("failed to copy {}", final_patch.display()))?;

    let run_metrics_path = out_dir.join(METRICS_JSON);
    let run_metrics_value = read_json_file(&run_metrics_path)?;
    let compact_artifact_bytes = CODEX_REVIEW_ARTIFACTS
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
        "artifact_files_read_by_codex": RUN_COMPACT_ARTIFACTS,
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": compact_artifact_bytes,
        "approximate_codex_output_bytes": null,
        "local_worker_text_bytes": local_worker_text_bytes,
        "patch_bytes": get_u64(&run_metrics_value, "patch_bytes").unwrap_or(0),
        "changed_file_count": get_u64(&run_metrics_value, "changed_file_count").unwrap_or(0),
        "changed_line_count": get_u64(&run_metrics_value, "changed_line_count").unwrap_or(0),
        "final_status": get_str(&json!(receipt), "status").unwrap_or("unknown").to_string(),
        "notes": [
            "This prototype assumes the supervisor reviews compact Mixmod artifacts first.",
            "Exact Codex token telemetry is unavailable unless added manually."
        ]
    });
    write_pretty_json(
        &mixmod_dir.join(METRICS_JSON),
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
