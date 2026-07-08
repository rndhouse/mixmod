//! Internal benchmark harness utilities for Mixmod development.

mod cli;
mod deepswe;
mod process;
mod state;
mod worker;

pub(crate) use cli::Cli;

use anyhow::Result;
use cli::{Commands, DeepSweCommand, WorkerCommand};

/// Run the internal benchmark harness command line.
pub(crate) fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Worker { command } => match command {
            WorkerCommand::Setup => worker::llama::setup(),
            WorkerCommand::Teardown => worker::llama::teardown(),
            WorkerCommand::RunWith { command } => worker::llama::run_with_worker(command),
        },
        Commands::DeepSwe { command } => match command {
            DeepSweCommand::NormalizeArtifacts {
                pool,
                job_dir,
                records_json,
            } => deepswe::artifacts::normalize_cli(&pool, &job_dir, records_json.as_deref()),
        },
    }
}
