#![recursion_limit = "512"]

use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde_json::{Value, json};

mod artifacts;
mod checkpoint;
mod cli;
mod config;
mod default_strategy;
mod diff;
mod experiment;
mod fs_util;
mod harness;
mod install;
mod interventions;
mod live;
mod loop_summary;
mod report;
mod run;
mod state;
mod strategy_metrics;
mod supervisor;
mod task;
#[cfg(test)]
mod tests;
mod worker;
mod worker_telemetry;

pub(crate) use artifacts::{
    BLOCKED_RECEIPT_JSON, CHANGES_PATCH, CODEX_REVIEW_ARTIFACTS, FINAL_PATCH,
    LOCAL_VERIFICATION_JSON, METRICS_JSON, OPENCODE_INSTRUCTIONS_MD, PARTIAL_PATCH,
    PATCH_COMPARISON, PATCH_ROLLBACK_JSON, PREVIOUS_WORKTREE_PATCH, REASONING_TRACE_JSONL,
    RECEIPT_JSON, REPORT_MD, ROLLBACK_CURRENT_PATCH, ROLLBACK_RESTORED_PATCH,
    RUN_COMPACT_ARTIFACTS, SESSION_JSONL, SUPERVISION_LOOP_SUMMARY_JSON, SUPERVISOR_CONTROL_LOG,
    SUPERVISOR_FEEDBACK_JSONL, TASK_JSON, TASK_MD, TOOL_EVENTS_JSONL, WORKER_BRIEF_JSON,
    WORKER_RUN_ARTIFACTS, WORKER_TASK_JSON, WORKTREE_PATCH, is_static_mixmod_artifact_name,
    supervisor_review_artifact_paths,
};
pub use artifacts::{
    CodexOnlyMetrics, DefaultStrategyMetrics, ExperimentReportInputs, INTERVENTIONS_JSONL,
    PatchStats, Receipt, RunMetrics, SupervisorControlCommand, SupervisorControlEvent,
    SupervisorFeedback, WorkerBrief,
};
#[cfg(test)]
pub(crate) use checkpoint::write_patch_checkpoint_comparison;
pub(crate) use checkpoint::{
    append_patch_checkpoint_artifacts, patch_checkpoint_metrics, restore_previous_patch_checkpoint,
    write_patch_checkpoint_comparison_from_patch,
};
pub use cli::{Cli, Commands, ControlCommand, DelegationMode, ExperimentCommand};
pub(crate) use config::is_cloud_opencode_provider;
pub use config::{
    LiveSupervisionConfig, LocalVerificationConfig, MixmodConfig, ModelOverrides, OpenCodeConfig,
    StrategyConfig, SupervisorConfig, SupervisorInitMode, WorkerBackend, WorkerConfig,
};
pub(crate) use default_strategy::{DefaultStrategyOptions, run_default_strategy};
pub use diff::patch_stats;
pub use experiment::{
    DefaultRunOptions, experiment_init, experiment_record_codex_only, experiment_record_mixmod,
    experiment_recover, experiment_run_default,
};
pub use harness::codex::ShellCodexRunner;
pub use harness::opencode::ShellOpenCodeRunner;
pub use harness::{
    AgentBackend, AgentHarness, AgentOutput, AgentRequest, AgentRole, AgentSessionMode,
    LiveWorkerSnapshot, OpenCodeOutput, OpenCodeRequest, OpenCodeRunner, SupervisorAdvisor,
    worker_harness_for_config,
};
pub use install::{doctor_project, init_project, status_project};
pub use interventions::{
    InterventionEvent, InterventionKind, InterventionLog, InterventionPhase,
    InterventionSessionPolicy, InterventionTarget,
};
pub use report::experiment_report;
pub use run::{run_mixmod_task, run_mixmod_task_with_options};

use diff::{diff_without_unchanged_blocks, git_diff_committed_range, git_diff_with_untracked};
pub(crate) use experiment::{placeholder_experiment_metrics, validate_experiment_name};
#[cfg(test)]
pub(crate) use experiment::{write_revision_task, write_worker_brief_task};
pub(crate) use fs_util::*;
#[cfg(test)]
pub(crate) use harness::codex::codex_home_for_work_dir;
pub(crate) use harness::codex::{CodexSandbox, run_codex_exec_turn};
#[cfg(test)]
pub(crate) use harness::opencode::{
    OpenCodeModelSelection, effective_backend_command_for_base_url, opencode_config_path,
    prepare_opencode_args, prepare_opencode_control_args, run_with_local_verification,
};
pub(crate) use harness::opencode::{
    normalize_supervisor_control_action, normalize_supervisor_control_worker_mode, tail_text,
};
#[cfg(test)]
pub(crate) use install::is_managed_file;
pub(crate) use install::{ensure_project_state, find_on_path, load_config, yes_no};
#[cfg(test)]
pub(crate) use live::supervise_run_args;
pub(crate) use live::{
    ensure_debug_command_enabled, live_control, live_status, supervise_mixmod_task,
};
pub(crate) use loop_summary::write_supervision_loop_summary;
pub(crate) use report::budgeted_report;
pub(crate) use run::{
    WorkerRunOptions, run_mixmod_task_with_session, run_mixmod_task_with_worker_options,
    shell_command, worker_session_token_peak,
};
#[cfg(test)]
pub(crate) use run::{
    build_opencode_instruction, build_run_summary, opencode_exit_status_label,
    worker_context_signals,
};
pub(crate) use state::state_layout;
pub(crate) use strategy_metrics::WorkerMetricsSummary;
pub(crate) use supervisor::{
    LiveSupervisorAdvisor, RevisionHandoff, SupervisorCodexSession, SupervisorFeedbackTurn,
    aggregate_supervisor_usage, codex_only_prompt, normalize_worker_mode,
    run_supervisor_brief_turn, run_supervisor_feedback_turn,
};
#[cfg(test)]
pub(crate) use supervisor::{
    normalize_feedback_value, supervisor_feedback_prompt, supervisor_feedback_repair_prompt,
    supervisor_worker_brief_prompt,
};
pub use worker::WorkerModelProfile;
pub(crate) use worker::{WorkerSupervisorGuidance, default_worker_model_profiles};
pub use worker_telemetry::{WorkerBackendSlotTelemetry, WorkerBackendTelemetry};

use task::{
    TaskSpec, agent_visible_task_value, ensure_agent_visible_task_file, read_task_json,
    task_markdown_from_json, write_agent_visible_task_file, write_prompt_task_file,
};

const MANAGED_MARKER: &str = "MIXMOD MANAGED";
#[cfg(test)]
const LEGACY_OPENCODE_CONFIG: &str = "opencode.json";
#[cfg(test)]
const CODEX_INSTRUCTIONS: &str = ".codex/mixmod-instructions.md";
const LIVE_STATUS_FILE: &str = "live-status.json";
const SUPERVISOR_CONTROL_FILE: &str = "control.json";
const DEFAULT_OPENCODE_PROVIDER: &str = "llama.cpp";
const MIXMOD_OPENCODE_AGENT: &str = "mixmod-worker";
const DEFAULT_OPENCODE_MODEL: &str = "qwen-3.6-27b";
const DEFAULT_OPENCODE_LOCAL_MODEL: &str = "qwen/qwen3.6-27b";
const DEFAULT_SUPERVISOR_MODEL: &str = "gpt-5.5";
const DEFAULT_SUPERVISOR_REASONING_EFFORT: &str = "high";
const DEBUG_COMMANDS_ENV: &str = "MIXMOD_DEBUG_COMMANDS";

pub fn run_cli(cli: Cli, cwd: &Path) -> Result<()> {
    let root = cwd
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", cwd.display()))?;
    match cli.command {
        Commands::Init => {
            ensure_debug_command_enabled("mixmod init")?;
            init_project(&root)
        }
        Commands::Status => {
            ensure_debug_command_enabled("mixmod status")?;
            status_project(&root)
        }
        Commands::Doctor => {
            ensure_debug_command_enabled("mixmod doctor")?;
            doctor_project(&root)
        }
        Commands::Exec {
            task,
            resume_session,
            supervisor_model,
            worker_model,
            worker_backend,
            supervisor_init,
            stop_after_first_worker,
            no_require_local,
            prompt,
        } => {
            ensure_project_state(&root, false)?;
            let task = resolve_exec_task(&root, task, prompt)?;
            let out = state_layout(&root).runs().join(make_run_id("run"));
            let model_overrides = ModelOverrides::new(supervisor_model, worker_model)
                .with_worker_backend(worker_backend);
            run_default_strategy(
                &root,
                &task,
                &out,
                DefaultStrategyOptions {
                    resume_session,
                    model_overrides,
                    supervisor_init,
                    stop_after_first_worker,
                    no_require_local,
                },
            )
        }
        Commands::RunWorker {
            mode,
            task,
            out,
            require_local,
            resume_session,
            supervisor_model,
            worker_model,
            worker_backend,
        } => {
            ensure_debug_command_enabled("mixmod run-worker")?;
            ensure_project_state(&root, false)?;
            let mut config = load_config(&root)?;
            ModelOverrides::new(supervisor_model, worker_model)
                .with_worker_backend(worker_backend)
                .apply_to_config(&mut config)?;
            let runner = worker_harness_for_config(config);
            run_mixmod_task_with_session(
                &root,
                mode,
                &task,
                &out,
                runner.as_ref(),
                require_local,
                resume_session,
            )
            .map(|_| ())
        }
        Commands::RunSupervisor {
            mode,
            task,
            out,
            require_local,
            resume_session,
            supervisor_model,
            worker_model,
            worker_backend,
        } => {
            ensure_debug_command_enabled("mixmod run-supervisor")?;
            ensure_project_state(&root, false)?;
            let model_overrides = ModelOverrides::new(supervisor_model, worker_model)
                .with_worker_backend(worker_backend);
            supervise_mixmod_task(
                &root,
                mode,
                &task,
                &out,
                require_local,
                resume_session,
                model_overrides,
            )
        }
        Commands::Control { command } => {
            ensure_debug_command_enabled("mixmod control")?;
            match command {
                ControlCommand::Status { run, json } => live_status(&root, &run, json),
                ControlCommand::Send {
                    run,
                    action,
                    message,
                    focus_files,
                    required_checks,
                    risk,
                } => {
                    ensure_project_state(&root, false)?;
                    live_control(
                        &root,
                        &run,
                        &action,
                        message.as_deref(),
                        &focus_files,
                        &required_checks,
                        risk.as_deref(),
                    )
                }
            }
        }
        Commands::Experiment { command } => {
            ensure_debug_command_enabled("mixmod experiment")?;
            match command {
                ExperimentCommand::Init { name, fixture } => {
                    ensure_project_state(&root, false)?;
                    experiment_init(&root, &name, fixture.as_deref())
                }
                ExperimentCommand::RecordCodexOnly { name, task } => {
                    ensure_project_state(&root, false)?;
                    experiment_record_codex_only(&root, &name, &task)
                }
                ExperimentCommand::RecordMixmod { name, task } => {
                    ensure_project_state(&root, false)?;
                    experiment_record_mixmod(&root, &name, &task)
                }
                ExperimentCommand::RunDefault {
                    name,
                    require_local,
                    supervisor_model,
                    worker_model,
                    worker_backend,
                    supervisor_init,
                    stop_after_first_worker,
                } => {
                    ensure_project_state(&root, false)?;
                    experiment_run_default(
                        &root,
                        &name,
                        DefaultRunOptions {
                            require_local,
                            model_overrides: ModelOverrides::new(supervisor_model, worker_model)
                                .with_worker_backend(worker_backend),
                            supervisor_init,
                            stop_after_first_worker,
                        },
                    )
                }
                ExperimentCommand::RunBudgeted {
                    name,
                    require_local,
                    supervisor_model,
                    worker_model,
                    worker_backend,
                    supervisor_init,
                    stop_after_first_worker,
                } => {
                    ensure_project_state(&root, false)?;
                    experiment_run_default(
                        &root,
                        &name,
                        DefaultRunOptions {
                            require_local,
                            model_overrides: ModelOverrides::new(supervisor_model, worker_model)
                                .with_worker_backend(worker_backend),
                            supervisor_init,
                            stop_after_first_worker,
                        },
                    )
                }
                ExperimentCommand::Recover {
                    name,
                    require_local,
                } => {
                    ensure_project_state(&root, false)?;
                    experiment_recover(&root, &name, require_local)
                }
                ExperimentCommand::Report { name } => experiment_report(&root, &name).map(|_| ()),
            }
        }
    }
}

fn resolve_exec_task(
    root: &Path,
    task: Option<PathBuf>,
    prompt_parts: Vec<String>,
) -> Result<PathBuf> {
    let prompt = prompt_parts.join(" ");
    let has_prompt = !prompt.trim().is_empty();
    match (task, has_prompt) {
        (Some(_), true) => {
            bail!("provide either a prompt or --task <task.json>, not both")
        }
        (Some(task), false) => Ok(task),
        (None, true) => write_prompt_task_file(root, &prompt),
        (None, false) => bail!("provide a prompt or --task <task.json>"),
    }
}
