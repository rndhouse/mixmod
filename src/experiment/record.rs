use crate::*;

use super::init::experiment_init;
use super::util::{experiment_dir, mixmod_notes_template, validate_experiment_name};

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
