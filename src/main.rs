//! Thin binary entry point. The work lives in the `bestiario` library.

use clap::Parser;

use bestiario::cli::Cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    bestiario::logging::init(cli.verbose);

    match bestiario::commands::run(&cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` renders the whole anyhow chain on one line, so the
            // context added at each layer is visible without a backtrace.
            tracing::error!("{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
