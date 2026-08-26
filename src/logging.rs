//! Log initialisation.
//!
//! Verbosity comes from `-v` flags, and `RUST_LOG` overrides them entirely —
//! the flag is the convenient path, the environment variable is the precise
//! one.

use tracing_subscriber::EnvFilter;

/// Installs the global subscriber. Called once, from `main`.
///
/// `-v` raises this crate's level without also turning on every dependency's
/// logs, which is what a bare `RUST_LOG=debug` would do and is rarely what
/// anyone wants.
pub fn init(verbose: u8) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter(verbose)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn default_filter(verbose: u8) -> String {
    match verbose {
        0 => "bestiario=info,bestiario_stats=info,warn".to_string(),
        1 => "bestiario=debug,bestiario_stats=debug,info".to_string(),
        _ => "bestiario=trace,bestiario_stats=trace,debug".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_raises_this_crate_before_it_raises_dependencies() {
        // A bare RUST_LOG=debug drowns the output in relay and sqlx chatter.
        // -v should make bestiario talkative first.
        assert!(default_filter(0).starts_with("bestiario=info"));
        assert!(default_filter(1).starts_with("bestiario=debug"));
        assert!(default_filter(9).starts_with("bestiario=trace"));
    }

    #[test]
    fn dependencies_stay_one_level_quieter_than_this_crate() {
        assert!(default_filter(0).ends_with("warn"));
        assert!(default_filter(1).ends_with("info"));
        assert!(default_filter(2).ends_with("debug"));
    }
}
