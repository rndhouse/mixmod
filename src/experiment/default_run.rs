use crate::*;

use super::util::{copy_budgeted_artifacts, experiment_dir, validate_experiment_name};
use crate::strategy::engine::{
    DefaultStrategyEngineOptions, DefaultStrategyStopOptions, run_default_strategy_engine,
};
use crate::strategy::finalize::{
    DefaultStrategyMetricsInput, DefaultStrategyRequireLocal, build_default_strategy_metrics,
    write_default_strategy_final_patch,
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
        let engine = run_default_strategy_engine(DefaultStrategyEngineOptions {
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

        let final_patch = write_default_strategy_final_patch(
            &work_dir,
            &default_dir,
            &original_patch_base,
            engine.internal_patch_baselines,
        )?;
        copy_budgeted_artifacts(root, &default_dir, &engine.final_out)?;
        let metrics = build_default_strategy_metrics(DefaultStrategyMetricsInput {
            display_root: root,
            strategy_dir: &default_dir,
            run_start,
            wall_clock_ms: start.elapsed().as_millis(),
            supervisor: &supervisor,
            supervisor_init,
            strategy,
            worker_guidance: &worker_guidance,
            worker_self_review,
            worker_auto_followups,
            worker_forced_context_focus,
            stop_after_first_worker: options.stop_after_first_worker,
            stop_after_first_review: options.stop_after_first_review,
            stop_after_worker_turns: options.stop_after_worker_turns,
            original_patch_base: &original_patch_base,
            final_patch: &final_patch,
            engine: &engine,
            require_local: DefaultStrategyRequireLocal::Fixed(options.require_local),
            extra_notes: &[
                "If the worker times out, run `mixmod experiment recover <name> --require-local` to restart from worker-task.json.",
            ],
        })?;
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
