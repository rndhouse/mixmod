use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "mixmod")]
#[command(about = "Reduce frontier LLM cost with supervised worker models")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create project-local Mixmod state and OpenCode configuration.
    #[command(hide = true)]
    Init,
    /// Show local Mixmod and tool status.
    #[command(hide = true)]
    Status,
    /// Validate the local environment and print actionable diagnostics.
    #[command(hide = true)]
    Doctor,
    /// Run a Mixmod task non-interactively.
    Exec {
        #[arg(long)]
        task: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        require_local: bool,
        #[arg(long)]
        resume_session: Option<String>,
    },
    /// Debug-only low-level foreground worker command.
    #[command(hide = true)]
    Run {
        #[arg(value_enum)]
        mode: DelegationMode,
        #[arg(long)]
        task: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        require_local: bool,
        #[arg(long)]
        resume_session: Option<String>,
    },
    /// Debug-only explicit-mode background worker launcher.
    #[command(hide = true)]
    Supervise {
        #[arg(value_enum)]
        mode: DelegationMode,
        #[arg(long)]
        task: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        require_local: bool,
        #[arg(long)]
        resume_session: Option<String>,
    },
    /// Entry point for legacy repo-local Codex hooks.
    Hook {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Inspect or steer one running Mixmod/OpenCode worker run.
    Live {
        #[command(subcommand)]
        command: LiveCommand,
    },
    /// Manage repeatable Mixmod experiments.
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum LiveCommand {
    /// Print compact live status for a worker run directory.
    Status {
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Send a supervisor control action for a running worker.
    Control {
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        action: String,
        #[arg(long)]
        message: Option<String>,
        #[arg(long = "focus-file")]
        focus_files: Vec<String>,
        #[arg(long = "check")]
        required_checks: Vec<String>,
        #[arg(long)]
        risk: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum DelegationMode {
    Explore,
    Patch,
    Review,
    ExplainFailure,
}

impl std::fmt::Display for DelegationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Explore => "explore",
            Self::Patch => "patch",
            Self::Review => "review",
            Self::ExplainFailure => "explain-failure",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Subcommand)]
pub enum ExperimentCommand {
    /// Create an experiment directory with task templates and run slots.
    Init {
        name: String,
        #[arg(long)]
        fixture: Option<PathBuf>,
    },
    /// Create or update the Codex-only result slot.
    RecordCodexOnly {
        name: String,
        #[arg(long)]
        task: PathBuf,
    },
    /// Legacy one-shot Mixmod trial. Prefer `run-default`.
    #[command(hide = true)]
    RecordMixmod {
        name: String,
        #[arg(long)]
        task: PathBuf,
    },
    /// Run the default Mixmod strategy over a local OpenCode worker.
    RunDefault {
        name: String,
        #[arg(long)]
        require_local: bool,
    },
    /// Deprecated alias for `run-default`.
    #[command(hide = true)]
    RunBudgeted {
        name: String,
        #[arg(long)]
        require_local: bool,
    },
    /// Recover a default-strategy run by restarting OpenCode from the saved worker task.
    Recover {
        name: String,
        #[arg(long)]
        require_local: bool,
    },
    /// Summarize Codex-only vs Mixmod default results.
    Report { name: String },
}
