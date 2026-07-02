#![recursion_limit = "256"]

use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

mod artifacts;
mod cli;
mod config;
mod diff;
mod experiment;
mod frontier;
mod fs_util;
mod install;
mod live;
mod opencode;
mod report;
mod run;
mod task;
#[cfg(test)]
mod tests;

pub use artifacts::{
    CodexOnlyMetrics, DefaultStrategyMetrics, ExperimentReportInputs, FrontierFeedback, PatchStats,
    Receipt, RunMetrics, SupervisorControlCommand, SupervisorControlEvent, TestArtifact,
    TestCommandResult, WorkerBrief,
};
pub use cli::{Cli, Commands, ControlCommand, DelegationMode, ExperimentCommand};
pub use config::{
    FrontierConfig, LocalVerificationConfig, MixmodConfig, ModelOverrides, OpenCodeConfig,
};
pub use diff::patch_stats;
pub use experiment::{
    DefaultRunOptions, experiment_init, experiment_record_codex_only, experiment_record_mixmod,
    experiment_recover, experiment_run_default,
};
pub use install::{doctor_project, init_project, status_project};
pub use opencode::{OpenCodeOutput, OpenCodeRequest, OpenCodeRunner, ShellOpenCodeRunner};
pub use report::experiment_report;
pub use run::{run_mixmod_task, run_mixmod_task_with_options};

use diff::{diff_without_unchanged_blocks, git_diff_with_untracked};
pub(crate) use experiment::{placeholder_experiment_metrics, validate_experiment_name};
#[cfg(test)]
pub(crate) use experiment::{write_revision_task, write_worker_brief_task};
pub(crate) use frontier::{
    CodexSandbox, FrontierFeedbackTurn, aggregate_frontier_usage, codex_only_prompt,
    normalize_worker_mode, run_codex_exec_turn, run_frontier_brief_turn,
    run_frontier_feedback_turn,
};
#[cfg(test)]
pub(crate) use frontier::{
    codex_exec_turn_args, codex_home_for_work_dir, frontier_feedback_prompt,
    frontier_worker_brief_prompt, normalize_feedback_value,
};
pub(crate) use fs_util::*;
#[cfg(test)]
pub(crate) use install::is_managed_file;
pub(crate) use install::{ensure_project_state, find_on_path, load_config, yes_no};
#[cfg(test)]
pub(crate) use live::supervise_run_args;
pub(crate) use live::{
    ensure_debug_command_enabled, live_control, live_status, supervise_mixmod_task,
};
#[cfg(test)]
pub(crate) use opencode::{
    OpenCodeModelSelection, opencode_config_path, prepare_opencode_args,
    prepare_opencode_control_args, run_with_local_verification,
};
pub(crate) use opencode::{
    normalize_supervisor_control_action, normalize_supervisor_control_worker_mode, tail_text,
};
pub(crate) use report::budgeted_report;
#[cfg(test)]
pub(crate) use run::{build_opencode_instruction, build_run_summary, opencode_exit_status_label};
pub(crate) use run::{run_mixmod_task_with_session, run_task_tests, shell_command};

use task::{
    TaskSpec, agent_visible_task_value, ensure_agent_visible_task_file, read_task_json,
    task_markdown_from_json, write_agent_visible_task_file,
};

const MANAGED_MARKER: &str = "MIXMOD MANAGED";
const MIXMOD_CONFIG: &str = ".mixmod/config.toml";
const MIXMOD_CODEX_HOME: &str = ".mixmod/codex-home";
const OPENCODE_CONFIG: &str = ".mixmod/opencode.json";
#[cfg(test)]
const LEGACY_OPENCODE_CONFIG: &str = "opencode.json";
#[cfg(test)]
const CODEX_INSTRUCTIONS: &str = ".codex/mixmod-instructions.md";
const LIVE_STATUS_FILE: &str = "live-status.json";
const SUPERVISOR_CONTROL_FILE: &str = "control.json";
const SUPERVISOR_CONTROL_LOG: &str = "supervisor-control.jsonl";
const DEFAULT_OPENCODE_PROVIDER: &str = "mixmod-local-ollama";
const DEFAULT_OPENCODE_MODEL: &str = "qwen-3.6-27b";
const DEFAULT_OPENCODE_OLLAMA_MODEL: &str = "qwen3.6:27b";
const DEFAULT_FRONTIER_MODEL: &str = "gpt-5.5";
const DEFAULT_FRONTIER_REASONING_EFFORT: &str = "high";
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
            out,
            resume_session,
            supervisor_model,
            worker_model,
        } => {
            ensure_project_state(&root, false)?;
            let model_overrides = ModelOverrides::new(supervisor_model, worker_model);
            supervise_mixmod_task(
                &root,
                DelegationMode::Patch,
                &task,
                &out,
                false,
                resume_session,
                model_overrides,
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
        } => {
            ensure_debug_command_enabled("mixmod run-worker")?;
            ensure_project_state(&root, false)?;
            let mut config = load_config(&root)?;
            ModelOverrides::new(supervisor_model, worker_model).apply_to_config(&mut config)?;
            let runner = ShellOpenCodeRunner::new(config);
            run_mixmod_task_with_session(
                &root,
                mode,
                &task,
                &out,
                &runner,
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
        } => {
            ensure_debug_command_enabled("mixmod run-supervisor")?;
            ensure_project_state(&root, false)?;
            let model_overrides = ModelOverrides::new(supervisor_model, worker_model);
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
                } => {
                    ensure_project_state(&root, false)?;
                    experiment_run_default(&root, &name, DefaultRunOptions { require_local })
                }
                ExperimentCommand::RunBudgeted {
                    name,
                    require_local,
                } => {
                    ensure_project_state(&root, false)?;
                    experiment_run_default(&root, &name, DefaultRunOptions { require_local })
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
