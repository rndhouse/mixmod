use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mixmod=warn".into()),
        )
        .without_time()
        .init();

    let cli = mixmod::Cli::parse();
    match std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|cwd| mixmod::run_cli(cli, &cwd))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
