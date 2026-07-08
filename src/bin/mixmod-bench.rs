use std::process::ExitCode;

use clap::Parser;

#[path = "mixmod_bench/mod.rs"]
mod mixmod_bench;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mixmod_bench=warn".into()),
        )
        .without_time()
        .init();

    let cli = mixmod_bench::Cli::parse();
    match mixmod_bench::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
