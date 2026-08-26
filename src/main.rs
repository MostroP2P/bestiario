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
            // Written straight to stderr rather than through `tracing`,
            // because the subscriber can be configured to drop it: `RUST_LOG=off`,
            // or a dependency-only filter such as `RUST_LOG=sqlx=debug`, would
            // leave a failed command exiting 1 with no explanation at all.
            //
            // `{:#}` renders the whole anyhow chain on one line, so the context
            // added at each layer is visible without a backtrace.
            eprintln!("bestiario: {error:#}");

            // A copy goes to the log as well, so that a run being captured for
            // diagnostics has the failure in the same stream as everything
            // leading up to it.
            tracing::debug!("{error:?}");

            std::process::ExitCode::FAILURE
        }
    }
}
