//! Loading and validation of `settings.toml`.
//!
//! Responsibility: turn a configuration file plus `BESTIARIO__*` environment
//! overrides into a validated [`Settings`] value, and reject anything
//! malformed at startup rather than at the point of use. See `docs/SPEC.md`
//! §9 for the file format.
//!
//! # Why validate eagerly
//!
//! Most of the settings here select *what to index*. A typo in `networks` or
//! in an instance pubkey does not crash anything — it silently produces an
//! empty database and a report full of zeros, which is the most expensive
//! kind of failure this project can have. So every rule that can be checked
//! without touching the network is checked once, at startup, with an error
//! that names the offending value.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[cfg(test)]
mod tests;

/// Networks a `network` tag may name. Validated eagerly so that a typo such
/// as `mainet` fails at startup instead of silently filtering out every
/// event.
const KNOWN_NETWORKS: [&str; 4] = ["mainnet", "testnet", "signet", "regtest"];

/// Environment prefix and separator: `BESTIARIO__DATABASE__URL` overrides
/// `[database].url`.
const ENV_PREFIX: &str = "BESTIARIO";
const ENV_SEPARATOR: &str = "__";

/// Anything that can go wrong between "a file on disk" and "a usable
/// [`Settings`]".
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read, or is not valid TOML, or does not match
    /// the expected shape.
    #[error("could not load configuration: {0}")]
    Load(#[from] config::ConfigError),

    /// The file parsed, but a value does not make sense.
    #[error(transparent)]
    Invalid(#[from] ValidationError),
}

/// A setting that parsed but cannot be acted on.
///
/// One variant per rule, each naming the offending value, so that the
/// operator does not have to guess which of twelve relays is malformed.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("[nostr].relays is empty: at least one relay is required")]
    NoRelays,

    #[error(
        "[nostr].relays contains `{url}`: expected a websocket URL starting with `wss://` \
         (or `ws://` for a local relay)"
    )]
    RelayNotWebsocket { url: String },

    #[error(
        "[indexer].instances contains `{pubkey}`: expected 64 hexadecimal characters, got {len}"
    )]
    PubkeyLength { pubkey: String, len: usize },

    #[error("[indexer].instances contains `{pubkey}`: `{found}` is not a hexadecimal character")]
    PubkeyNotHex { pubkey: String, found: char },

    #[error(
        "[indexer].instances is empty and accept_unknown_instances is false: \
         nothing would ever be indexed"
    )]
    NothingToIndex,

    #[error("[indexer].networks is empty: at least one network is required")]
    NoNetworks,

    #[error(
        "[indexer].networks contains `{network}`: expected one of {}",
        KNOWN_NETWORKS.join(", ")
    )]
    UnknownNetwork { network: String },

    #[error("[indexer].backfill_from is {value}: expected a unix timestamp, or 0 for everything")]
    NegativeBackfillFrom { value: i64 },

    #[error("{setting} is {value}: expected a fraction greater than 0 and at most 1")]
    DevFeePercentageOutOfRange { setting: String, value: f64 },

    #[error("[database].url is `{url}`: expected a `sqlite:` URL")]
    DatabaseNotSqlite { url: String },

    #[error("[report].reference_currency is `{code}`: expected a three-letter currency code")]
    ReferenceCurrencyNotIso { code: String },
}

/// The whole of `settings.toml`, validated.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub nostr: NostrSettings,
    pub indexer: IndexerSettings,
    #[serde(default)]
    pub assumptions: AssumptionSettings,
    pub database: DatabaseSettings,
    #[serde(default)]
    pub report: ReportSettings,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrSettings {
    /// Relays to connect to. Additional ones may be discovered via NIP-65
    /// when `discover_relays` is set.
    pub relays: Vec<String>,
    #[serde(default)]
    pub discover_relays: bool,
    /// How far back to rewind the cursor when resuming a live subscription,
    /// to tolerate clock skew between relays. Duplicates are absorbed by the
    /// dedup step of the pipeline.
    #[serde(default = "default_resume_overlap_secs")]
    pub resume_overlap_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexerSettings {
    /// Instance pubkeys to follow, lowercase hex.
    #[serde(default)]
    pub instances: Vec<String>,
    /// Index any pubkey publishing events tagged `y = ["mostro", ...]`,
    /// rather than only those listed above.
    #[serde(default)]
    pub accept_unknown_instances: bool,
    #[serde(default = "default_networks")]
    pub networks: Vec<String>,
    /// Unix timestamp to backfill down to; `0` means everything the relays
    /// still hold.
    #[serde(default)]
    pub backfill_from: i64,
}

/// Values that are not published on Nostr and therefore have to be assumed.
/// Every metric derived from these is reported as inferred (`docs/SPEC.md`
/// §5).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssumptionSettings {
    /// Share of its fee an instance forwards as the dev fee. The daemon
    /// default is 0.30; instances do not publish the real value.
    #[serde(default = "default_dev_fee_percentage")]
    pub dev_fee_percentage_default: f64,
    /// Per-instance overrides, keyed by pubkey.
    #[serde(default)]
    pub dev_fee_percentage: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSettings {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSettings {
    /// Currency that inferred volume is converted into.
    #[serde(default = "default_reference_currency")]
    pub reference_currency: String,
}

fn default_resume_overlap_secs() -> u64 {
    3600
}

fn default_networks() -> Vec<String> {
    vec!["mainnet".to_string()]
}

fn default_dev_fee_percentage() -> f64 {
    0.30
}

fn default_reference_currency() -> String {
    "USD".to_string()
}

impl Default for AssumptionSettings {
    fn default() -> Self {
        Self {
            dev_fee_percentage_default: default_dev_fee_percentage(),
            dev_fee_percentage: BTreeMap::new(),
        }
    }
}

impl Default for ReportSettings {
    fn default() -> Self {
        Self {
            reference_currency: default_reference_currency(),
        }
    }
}

impl Settings {
    /// Loads `path`, layers `BESTIARIO__*` environment overrides on top,
    /// normalizes and validates.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = config::Config::builder()
            .add_source(config::File::from(path).required(true))
            .add_source(
                config::Environment::with_prefix(ENV_PREFIX)
                    .separator(ENV_SEPARATOR)
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize::<Settings>()?;

        raw.normalized().validated()
    }

    /// Parses TOML from memory, without the environment layer.
    pub fn from_toml_str(toml: &str) -> Result<Self, ConfigError> {
        let raw = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()?
            .try_deserialize::<Settings>()?;

        raw.normalized().validated()
    }

    /// Returns a copy with case-insensitive values folded to their canonical
    /// case, so that later comparisons are plain string equality.
    fn normalized(self) -> Self {
        Self {
            indexer: IndexerSettings {
                instances: self
                    .indexer
                    .instances
                    .iter()
                    .map(|p| p.trim().to_lowercase())
                    .collect(),
                networks: self
                    .indexer
                    .networks
                    .iter()
                    .map(|n| n.trim().to_lowercase())
                    .collect(),
                ..self.indexer
            },
            assumptions: AssumptionSettings {
                dev_fee_percentage: self
                    .assumptions
                    .dev_fee_percentage
                    .iter()
                    .map(|(k, v)| (k.trim().to_lowercase(), *v))
                    .collect(),
                ..self.assumptions
            },
            report: ReportSettings {
                reference_currency: self.report.reference_currency.trim().to_uppercase(),
            },
            ..self
        }
    }

    fn validated(self) -> Result<Self, ConfigError> {
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_relays()?;
        self.validate_indexer()?;
        self.validate_assumptions()?;
        self.validate_database()?;
        self.validate_report()
    }

    fn validate_relays(&self) -> Result<(), ValidationError> {
        if self.nostr.relays.is_empty() {
            return Err(ValidationError::NoRelays);
        }
        for url in &self.nostr.relays {
            // `ws://` is accepted on purpose: the E2E suite of docs/SPEC.md
            // §12 runs against a local relay, which is not served over TLS.
            if !url.starts_with("wss://") && !url.starts_with("ws://") {
                return Err(ValidationError::RelayNotWebsocket { url: url.clone() });
            }
        }
        Ok(())
    }

    fn validate_indexer(&self) -> Result<(), ValidationError> {
        for pubkey in &self.indexer.instances {
            validate_pubkey(pubkey)?;
        }

        if self.indexer.instances.is_empty() && !self.indexer.accept_unknown_instances {
            return Err(ValidationError::NothingToIndex);
        }

        if self.indexer.networks.is_empty() {
            return Err(ValidationError::NoNetworks);
        }
        for network in &self.indexer.networks {
            if !KNOWN_NETWORKS.contains(&network.as_str()) {
                return Err(ValidationError::UnknownNetwork {
                    network: network.clone(),
                });
            }
        }

        if self.indexer.backfill_from < 0 {
            return Err(ValidationError::NegativeBackfillFrom {
                value: self.indexer.backfill_from,
            });
        }

        Ok(())
    }

    fn validate_assumptions(&self) -> Result<(), ValidationError> {
        validate_fraction(
            "[assumptions].dev_fee_percentage_default",
            self.assumptions.dev_fee_percentage_default,
        )?;

        for (pubkey, value) in &self.assumptions.dev_fee_percentage {
            validate_pubkey(pubkey)?;
            validate_fraction(
                &format!("[assumptions.dev_fee_percentage].\"{pubkey}\""),
                *value,
            )?;
        }

        Ok(())
    }

    fn validate_database(&self) -> Result<(), ValidationError> {
        if !self.database.url.starts_with("sqlite:") {
            return Err(ValidationError::DatabaseNotSqlite {
                url: self.database.url.clone(),
            });
        }
        Ok(())
    }

    fn validate_report(&self) -> Result<(), ValidationError> {
        let code = &self.report.reference_currency;
        if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(ValidationError::ReferenceCurrencyNotIso { code: code.clone() });
        }
        Ok(())
    }

    /// The dev fee share to assume for `pubkey`: its override if it has one,
    /// otherwise the global default.
    pub fn dev_fee_percentage_for(&self, pubkey: &str) -> f64 {
        self.assumptions
            .dev_fee_percentage
            .get(&pubkey.to_lowercase())
            .copied()
            .unwrap_or(self.assumptions.dev_fee_percentage_default)
    }
}

fn validate_pubkey(pubkey: &str) -> Result<(), ValidationError> {
    if pubkey.len() != 64 {
        return Err(ValidationError::PubkeyLength {
            pubkey: pubkey.to_string(),
            len: pubkey.len(),
        });
    }
    if let Some(found) = pubkey.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(ValidationError::PubkeyNotHex {
            pubkey: pubkey.to_string(),
            found,
        });
    }
    Ok(())
}

fn validate_fraction(setting: &str, value: f64) -> Result<(), ValidationError> {
    if !(value > 0.0 && value <= 1.0) {
        return Err(ValidationError::DevFeePercentageOutOfRange {
            setting: setting.to_string(),
            value,
        });
    }
    Ok(())
}
