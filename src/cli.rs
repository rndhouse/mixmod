use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::config::{SupervisorInitMode, WorkerBackend};

#[derive(Debug, Parser)]
#[command(name = "mixmod")]
#[command(about = "Reduce supervisor-model cost with worker models")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create central Mixmod state and OpenCode configuration.
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
        /// Read task details from a structured JSON task file.
        #[arg(long, value_name = "TASK_JSON")]
        task: Option<PathBuf>,
        /// Resume a specific worker session.
        #[arg(long)]
        resume_session: Option<String>,
        /// Supervisor model, optionally suffixed with reasoning effort.
        #[arg(long, value_name = "MODEL[:EFFORT]")]
        supervisor_model: Option<String>,
        /// Worker model override, interpreted by the selected backend.
        #[arg(long, value_name = "MODEL")]
        worker_model: Option<String>,
        /// Worker backend used for repository-editing turns.
        #[arg(long, value_enum)]
        worker_backend: Option<WorkerBackend>,
        /// Initial supervisor briefing style.
        #[arg(long, value_enum)]
        supervisor_init: Option<SupervisorInitMode>,
        /// Stop after the first worker attempt and leave artifacts for inspection.
        #[arg(long)]
        stop_after_first_worker: bool,
        /// Stop after the first supervisor review and leave artifacts for inspection.
        #[arg(long, conflicts_with = "stop_after_first_worker")]
        stop_after_first_review: bool,
        /// Suggested worker changed-line target for one turn.
        #[arg(long)]
        worker_target_patch_lines: Option<u64>,
        /// Suggested worker changed-line ceiling for one turn.
        #[arg(long)]
        worker_max_patch_lines: Option<u64>,
        /// Do not require local worker inference verification for this run.
        #[arg(long)]
        no_require_local: bool,
        /// Natural-language task request.
        #[arg(value_name = "PROMPT", num_args = 0.., trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// Debug-only foreground worker command.
    #[command(hide = true)]
    #[command(name = "run-worker")]
    RunWorker {
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
        #[arg(long, value_name = "MODEL[:EFFORT]")]
        supervisor_model: Option<String>,
        #[arg(long, value_name = "MODEL")]
        worker_model: Option<String>,
        #[arg(long, value_enum)]
        worker_backend: Option<WorkerBackend>,
    },
    /// Debug-only background supervisor launcher.
    #[command(hide = true)]
    #[command(name = "run-supervisor")]
    RunSupervisor {
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
        #[arg(long, value_name = "MODEL[:EFFORT]")]
        supervisor_model: Option<String>,
        #[arg(long, value_name = "MODEL")]
        worker_model: Option<String>,
        #[arg(long, value_enum)]
        worker_backend: Option<WorkerBackend>,
    },
    /// Debug-only run inspection and steering commands.
    #[command(hide = true)]
    Control {
        #[command(subcommand)]
        command: ControlCommand,
    },
    /// Manage repeatable Mixmod experiments.
    #[command(hide = true)]
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ControlCommand {
    /// Print compact status for a worker run directory.
    Status {
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Send a supervisor control action for a running worker.
    Send {
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
    /// Legacy one-shot Mixmod trial. Prefer `run-default`.
    #[command(hide = true)]
    RecordMixmod {
        name: String,
        #[arg(long)]
        task: PathBuf,
    },
    /// Run the default Mixmod strategy over the configured worker.
    RunDefault {
        name: String,
        #[arg(long)]
        require_local: bool,
        #[arg(long, value_name = "MODEL[:EFFORT]")]
        supervisor_model: Option<String>,
        #[arg(long, value_name = "MODEL")]
        worker_model: Option<String>,
        #[arg(long, value_enum)]
        worker_backend: Option<WorkerBackend>,
        /// Initial supervisor briefing style.
        #[arg(long, value_enum)]
        supervisor_init: Option<SupervisorInitMode>,
        /// Stop after the first worker attempt and leave artifacts for inspection.
        #[arg(long)]
        stop_after_first_worker: bool,
        /// Stop after the first supervisor review and leave artifacts for inspection.
        #[arg(long, conflicts_with = "stop_after_first_worker")]
        stop_after_first_review: bool,
        /// Suggested worker changed-line target for one turn.
        #[arg(long)]
        worker_target_patch_lines: Option<u64>,
        /// Suggested worker changed-line ceiling for one turn.
        #[arg(long)]
        worker_max_patch_lines: Option<u64>,
    },
    /// Deprecated alias for `run-default`.
    #[command(hide = true)]
    RunBudgeted {
        name: String,
        #[arg(long)]
        require_local: bool,
        #[arg(long, value_name = "MODEL[:EFFORT]")]
        supervisor_model: Option<String>,
        #[arg(long, value_name = "MODEL")]
        worker_model: Option<String>,
        #[arg(long, value_enum)]
        worker_backend: Option<WorkerBackend>,
        /// Initial supervisor briefing style.
        #[arg(long, value_enum)]
        supervisor_init: Option<SupervisorInitMode>,
        /// Stop after the first worker attempt and leave artifacts for inspection.
        #[arg(long)]
        stop_after_first_worker: bool,
        /// Stop after the first supervisor review and leave artifacts for inspection.
        #[arg(long, conflicts_with = "stop_after_first_worker")]
        stop_after_first_review: bool,
        /// Suggested worker changed-line target for one turn.
        #[arg(long)]
        worker_target_patch_lines: Option<u64>,
        /// Suggested worker changed-line ceiling for one turn.
        #[arg(long)]
        worker_max_patch_lines: Option<u64>,
    },
    /// Recover a default-strategy run by restarting the configured worker.
    Recover {
        name: String,
        #[arg(long)]
        require_local: bool,
    },
    /// Summarize Codex-only vs Mixmod default results.
    Report { name: String },
}
