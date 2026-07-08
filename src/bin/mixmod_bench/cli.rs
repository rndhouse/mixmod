use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Internal benchmark harness commands.
#[derive(Debug, Parser)]
#[command(name = "mixmod-bench")]
#[command(about = "Internal Mixmod benchmark harness helpers")]
#[command(version)]
pub(crate) struct Cli {
    /// Benchmark helper namespace to run.
    #[command(subcommand)]
    pub(crate) command: Commands,
}

/// Top-level benchmark helper namespaces.
#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Manage worker services used by benchmark runs.
    Worker {
        /// Worker command to run.
        #[command(subcommand)]
        command: WorkerCommand,
    },
    /// DeepSWE benchmark helpers.
    #[command(name = "deepswe")]
    DeepSwe {
        /// DeepSWE command to run.
        #[command(subcommand)]
        command: DeepSweCommand,
    },
}

/// Worker service commands.
#[derive(Debug, Subcommand)]
pub(crate) enum WorkerCommand {
    /// Start or reuse the llama.cpp OpenAI-compatible worker.
    #[command(name = "setup-llama")]
    Setup,
    /// Stop a llama.cpp worker that Mixmod started.
    #[command(name = "teardown-llama")]
    Teardown,
    /// Run a command with the llama.cpp worker lifecycle wrapped around it.
    #[command(name = "run-with-llama")]
    RunWith {
        /// Benchmark command and arguments.
        #[arg(value_name = "COMMAND", num_args = 1.., trailing_var_arg = true)]
        command: Vec<String>,
    },
}

/// DeepSWE benchmark commands.
#[derive(Debug, Subcommand)]
pub(crate) enum DeepSweCommand {
    /// Copy Pier trial artifacts into stable per-task bundles.
    NormalizeArtifacts {
        /// Per-job output pool directory.
        #[arg(long)]
        pool: PathBuf,
        /// Pier job directory to normalize.
        #[arg(long)]
        job_dir: PathBuf,
        /// JSON file containing result records, or '-' to read stdin.
        #[arg(long)]
        records_json: Option<String>,
    },
}
