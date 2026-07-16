pub(crate) mod engine;
pub(crate) mod finalize;
pub(crate) mod policy;
pub(crate) mod support;

use engine::{
    DefaultStrategyEngineOptions, DefaultStrategyStopOptions, run_default_strategy_engine,
};
use finalize::{
    DefaultStrategyMetricsInput, DefaultStrategyRequireLocal, build_default_strategy_metrics,
    write_default_strategy_final_patch,
};

use crate::*;

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

        let worker_runs_dir = out_dir.join("worker-runs");
        let engine = run_default_strategy_engine(DefaultStrategyEngineOptions {
            root,
            strategy_dir: &out_dir,
            task_file: &task_file,
            runner: runner.as_ref(),
            supervisor: &supervisor,
            supervisor_init,
            strategy,
            worker_guidance: worker_guidance.clone(),
            live_supervision,
            proposal_resume_session: options.resume_session.clone(),
            require_local: false,
            worker_self_review,
            worker_auto_followups,
            worker_forced_context_focus,
            stop: DefaultStrategyStopOptions {
                stop_after_first_worker: options.stop_after_first_worker,
                stop_after_first_review: options.stop_after_first_review,
                stop_after_worker_turns: options.stop_after_worker_turns,
            },
            proposal_out: worker_runs_dir.join("proposal"),
            revision_task_label: "exec",
            revision_out_path: Box::new({
                let worker_runs_dir = worker_runs_dir.clone();
                move |decision_index| {
                    if decision_index == 1 {
                        worker_runs_dir.join("revision")
                    } else {
                        worker_runs_dir.join(format!("revision-{decision_index}"))
                    }
                }
            }),
            verify_worker_run: Box::new(|receipt, run_dir| {
                ensure_worker_run_verified(&out_dir, receipt, run_dir)
            }),
        })?;

        let final_patch = write_default_strategy_final_patch(
            root,
            &out_dir,
            &original_patch_base,
            engine.internal_patch_baselines,
        )?;
        let metrics = build_default_strategy_metrics(DefaultStrategyMetricsInput {
            display_root: root,
            strategy_dir: &out_dir,
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
            require_local: DefaultStrategyRequireLocal::FromFinalWorkerMetrics,
            extra_notes: &[],
        })?;
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
        println!(
            "status: {}",
            get_str(&metrics, "final_status").unwrap_or("unknown")
        );
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
