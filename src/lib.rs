#![recursion_limit = "256"]

use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
pub use cli::{Cli, Commands, DelegationMode, ExperimentCommand, LiveCommand};
pub use config::{FrontierConfig, LocalVerificationConfig, MixmodConfig, OpenCodeConfig};
pub use diff::patch_stats;
pub use experiment::{
    DefaultRunOptions, experiment_init, experiment_record_codex_only, experiment_record_mixmod,
    experiment_recover, experiment_run_default,
};
pub use install::{
    doctor_project, hook_entrypoint, init_project, status_project, uninstall_project,
};
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
    codex_exec_turn_args, frontier_feedback_prompt, frontier_worker_brief_prompt,
    normalize_feedback_value,
};
pub(crate) use fs_util::*;
#[cfg(test)]
pub(crate) use install::is_managed_file;
pub(crate) use install::{find_on_path, load_config, yes_no};
#[cfg(test)]
pub(crate) use live::supervise_run_args;
pub(crate) use live::{
    ensure_debug_command_enabled, live_control, live_status, supervise_mixmod_task,
};
#[cfg(test)]
pub(crate) use opencode::{
    OpenCodeModelSelection, prepare_opencode_args, prepare_opencode_control_args,
    run_with_local_verification,
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
const OPENCODE_CONFIG: &str = "opencode.json";
const CODEX_INSTRUCTIONS: &str = ".codex/mixmod-instructions.md";
const CODEX_CONFIG: &str = ".codex/config.toml";
const CODEX_HOOKS_CONFIG: &str = ".codex/hooks.json";
const CODEX_HOOK: &str = ".codex/hooks/mixmod-hook.sh";
const BACKUP_MANIFEST: &str = ".mixmod/backups/manifest.json";
const LIVE_STATUS_FILE: &str = "live-status.json";
const SUPERVISOR_CONTROL_FILE: &str = "control.json";
const SUPERVISOR_CONTROL_LOG: &str = "supervisor-control.jsonl";
const DEFAULT_OPENCODE_MODEL: &str = "qwen-3.6-27b";
const DEFAULT_OPENCODE_OLLAMA_MODEL: &str = "qwen3.6:27b";
const DEFAULT_FRONTIER_MODEL: &str = "gpt-5.5";
const DEFAULT_FRONTIER_REASONING_EFFORT: &str = "high";
const DEBUG_COMMANDS_ENV: &str = "MIXMOD_DEBUG_COMMANDS";

const INSTRUCTION_TEXT: &str = r#"You are the frontier supervisor.

This repo has Mixmod available for local delegation.

Use Mixmod for bounded local work. Optimize for lower frontier output tokens,
not for less frontier thinking. Prefer this low-bandwidth pattern:
1. Inspect the repo and task yourself.
2. Emit one compact executable worker handoff instead of writing a verbose patch.
   Default to `{"handoff":"guided",...}`. For guided, keep the handoff terse:
   one command-style message, likely files, and at most one or two checks.
   Omit risk/avoid fields unless they prevent a likely wrong patch. Target
   under about 120 output tokens for a normal guided brief.
   Assume the local worker is capable but prone to setup rabbit holes, broad
   exploration, and delayed edits.
   Use only `{"handoff":"as_given"}` when the original task already names the
   relevant files, desired behavior, and checks clearly enough for the worker.
3. Let Mixmod/OpenCode do implementation-heavy work locally.
4. Review compact artifacts and decide approve or request revision.

Use this command:
- `mixmod delegate --task <task.json> --out <run-dir> --require-local`

Mixmod invokes OpenCode locally and writes artifacts under `.mixmod/runs/`.
Repo-local hooks may log Codex lifecycle events under `.mixmod/hooks.jsonl`, but
Codex remains deliberate about delegation.

Read compact artifacts first:
1. receipt.json
2. report.md
3. changes.patch
4. tests.json
5. metrics.json

Read session and raw logs only when needed.

For live supervision, do not run long Mixmod worker commands in the foreground.
Use `mixmod delegate --task <task.json> --out <run-dir> --require-local`.
This starts the local worker in the background and returns immediately. Then poll
`mixmod live status --run <run-dir>` while OpenCode is active and use
`mixmod live control --run <run-dir> --action interrupt_continue ...` or
`interrupt_context_focus` when the worker needs steering.

During Mixmod experiments, do not solve by directly editing source or test files
as a fallback. Do not ask the user for approval to switch strategies. Keep
steering OpenCode with concise `interrupt_continue` messages, use
`interrupt_context_focus` when the worker context is polluted, or start another
`mixmod delegate` attempt when useful. If no useful local-worker path remains,
record a blocked or inconclusive result instead of making the patch yourself.

Always inspect worker outputs before accepting them. If `changes.patch` exists,
inspect it before accepting it. A successful delegation does not need to produce
a patch; it may produce analysis, ideas, blockers, test results, or a patch.
Final authority remains with Codex.
Prefer compact executable handoffs, compact critiques, and artifact paths over
pasting long logs or generating large patches directly.
"#;

pub fn run_cli(cli: Cli, cwd: &Path) -> Result<()> {
    let root = cwd
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", cwd.display()))?;
    match cli.command {
        Commands::Init => init_project(&root),
        Commands::Uninstall => uninstall_project(&root),
        Commands::Status => status_project(&root),
        Commands::Doctor => doctor_project(&root),
        Commands::Delegate {
            task,
            out,
            require_local,
            resume_session,
        } => supervise_mixmod_task(
            &root,
            DelegationMode::Patch,
            &task,
            &out,
            require_local,
            resume_session,
        ),
        Commands::Run {
            mode,
            task,
            out,
            require_local,
            resume_session,
        } => {
            ensure_debug_command_enabled("mixmod run")?;
            let config = load_config(&root)?;
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
        Commands::Supervise {
            mode,
            task,
            out,
            require_local,
            resume_session,
        } => {
            ensure_debug_command_enabled("mixmod supervise")?;
            supervise_mixmod_task(&root, mode, &task, &out, require_local, resume_session)
        }
        Commands::Hook { args } => hook_entrypoint(&root, args),
        Commands::Live { command } => match command {
            LiveCommand::Status { run, json } => live_status(&root, &run, json),
            LiveCommand::Control {
                run,
                action,
                message,
                focus_files,
                required_checks,
                risk,
            } => live_control(
                &root,
                &run,
                &action,
                message.as_deref(),
                &focus_files,
                &required_checks,
                risk.as_deref(),
            ),
        },
        Commands::Experiment { command } => match command {
            ExperimentCommand::Init { name, fixture } => {
                experiment_init(&root, &name, fixture.as_deref())
            }
            ExperimentCommand::RecordCodexOnly { name, task } => {
                experiment_record_codex_only(&root, &name, &task)
            }
            ExperimentCommand::RecordMixmod { name, task } => {
                experiment_record_mixmod(&root, &name, &task)
            }
            ExperimentCommand::RunDefault {
                name,
                require_local,
            } => experiment_run_default(&root, &name, DefaultRunOptions { require_local }),
            ExperimentCommand::RunBudgeted {
                name,
                require_local,
            } => experiment_run_default(&root, &name, DefaultRunOptions { require_local }),
            ExperimentCommand::Recover {
                name,
                require_local,
            } => experiment_recover(&root, &name, require_local),
            ExperimentCommand::Report { name } => experiment_report(&root, &name).map(|_| ()),
        },
    }
}
