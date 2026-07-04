use crate::*;

use super::util::{copy_budgeted_artifacts, experiment_dir, validate_experiment_name};

pub fn experiment_recover(root: &Path, name: &str, require_local: bool) -> Result<()> {
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
            "default strategy work directory is missing: {}",
            display_path(root, &work_dir)
        );
    }
    ensure_project_state(&work_dir, false)?;

    let default_dir = exp_dir.join("default");
    let worker_task = default_dir.join(WORKER_TASK_JSON);
    if !worker_task.exists() {
        bail!(
            "cannot recover without {}; run `mixmod experiment run-default {name}` through the worker-brief phase first",
            display_path(root, &worker_task)
        );
    }

    let config = load_config(&work_dir)?;
    let runner = worker_harness_for_config(config);
    let recovery_id = make_run_id("recovery");
    let out_dir = state_layout(&work_dir).runs().join(&recovery_id);
    let receipt = run_mixmod_task_with_options(
        &work_dir,
        DelegationMode::Patch,
        &worker_task,
        &out_dir,
        runner.as_ref(),
        require_local,
    )?;

    let recovery_dir = default_dir.join("recoveries").join(&recovery_id);
    fs::create_dir_all(&recovery_dir)
        .with_context(|| format!("failed to create recovery dir {}", recovery_dir.display()))?;
    copy_budgeted_artifacts(root, &recovery_dir, &out_dir)?;
    for name in [WORKER_BRIEF_JSON, WORKER_TASK_JSON] {
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
    atomic_write(&recovery_dir.join(FINAL_PATCH), final_patch.as_bytes())?;
    let run_metrics = read_json_file(&out_dir.join(METRICS_JSON))?;
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
        "final_patch": display_path(root, &recovery_dir.join(FINAL_PATCH)),
        "notes": [
            "Recovery restarts the configured worker from the saved worker-task.json.",
            "Supervisor review is not run automatically; inspect recovery artifacts before accepting."
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
