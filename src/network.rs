//! The networks a Mostro instance can publish on.
//!
//! One type, used by both the configuration file and the `--network` flag, so
//! that the accepted vocabulary is defined once. A misspelling has to fail at
//! whichever boundary it arrives through: an unrecognised network does not
//! crash anything, it silently matches no events and reports zeros, which
//! reads exactly like a real answer.

use clap::ValueEnum;
use serde::Deserialize;

/// Values the `network` tag of kinds 38383 and 8383 can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

impl Network {
    /// The wire form, as it appears in the `network` tag.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Signet => "signet",
            Self::Regtest => "regtest",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_round_trips_through_its_wire_form() {
        for network in [
            Network::Mainnet,
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ] {
            let wire = network.as_str();
            let parsed = Network::from_str(wire, true).expect("wire form should parse");
            assert_eq!(parsed, network, "{wire}");
            assert_eq!(network.to_string(), wire);
        }
    }

    #[test]
    fn a_misspelling_is_rejected_with_the_accepted_values() {
        let error = Network::from_str("mainet", true).expect_err("misspelling");

        assert!(error.contains("mainet"), "{error}");
    }

    #[test]
    fn the_wire_forms_are_lowercase() {
        // The tag values mostrod publishes are lowercase; a mismatch here
        // would filter out every event.
        for network in [Network::Mainnet, Network::Regtest] {
            assert_eq!(network.as_str(), network.as_str().to_lowercase());
        }
    }
}
