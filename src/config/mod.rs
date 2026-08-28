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

use nostr_sdk::prelude::{FromBech32, PublicKey};
use serde::Deserialize;

use crate::network::Network;

mod secret;
#[cfg(test)]
mod tests;

pub use secret::{Secret, SecretRef, Unresolved};

/// Environment prefix and separator: `BESTIARIO__DATABASE__URL` overrides
/// `[database].url`.
const ENV_PREFIX: &str = "BESTIARIO";
const ENV_SEPARATOR: &str = "__";

/// Separator between the human-readable part and the data of a bech32
/// string, as in `npub1…`.
const BECH32_SEPARATOR: char = '1';

/// Anything that can go wrong between "a file on disk" and "a usable
/// [`Settings`]".
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read, or is not valid TOML, or does not match
    /// the expected shape.
    ///
    /// The message does not interpolate the source: it is already declared as
    /// one, and anyhow renders the whole chain, so naming it here would print
    /// it twice.
    #[error("could not load configuration")]
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

    #[error("[indexer].instances contains `{pubkey}`: not a valid npub ({reason})")]
    PubkeyNotNpub { pubkey: String, reason: String },

    /// The same instance appears more than once in
    /// `[assumptions.dev_fee_percentage]`, spelled differently (hex and
    /// npub). Neither spelling has precedence, so the file is ambiguous.
    #[error(
        "[assumptions.dev_fee_percentage] names `{pubkey}` more than once ({}): \
         keep a single entry per instance",
        spellings.join(", ")
    )]
    DuplicateDevFeeOverride {
        pubkey: String,
        spellings: Vec<String>,
    },

    #[error(
        "[indexer].instances is empty and accept_unknown_instances is false: \
         nothing would ever be indexed"
    )]
    NothingToIndex,

    #[error("[indexer].networks is empty: at least one network is required")]
    NoNetworks,

    #[error("[indexer].backfill_from is {value}: expected a unix timestamp, or 0 for everything")]
    NegativeBackfillFrom { value: i64 },

    #[error("{setting} is {value}: expected a fraction greater than 0 and at most 1")]
    DevFeePercentageOutOfRange { setting: String, value: f64 },

    #[error("[database].url is `{url}`: expected a `sqlite:` URL")]
    DatabaseNotSqlite { url: String },

    #[error("[report].reference_currency is `{code}`: expected a three-letter currency code")]
    ReferenceCurrencyNotIso { code: String },

    #[error(
        "[publish].relays contains `{url}`: expected a websocket URL starting with `wss://` \
         (or `ws://` for a local relay)"
    )]
    PublishRelayNotWebsocket { url: String },

    #[error(
        "[publish].max_content_bytes is 0: a ceiling of zero would refuse every document, \
         including the index"
    )]
    PublishCeilingIsZero,
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
    #[serde(default)]
    pub publish: PublishSettings,
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
    /// Instance pubkeys to follow, lowercase hex. The file may spell them
    /// as hex or as NIP-19 `npub1…`; both are folded to hex on load.
    #[serde(default)]
    pub instances: Vec<String>,
    /// Index any pubkey publishing events tagged `y = ["mostro", ...]`,
    /// rather than only those listed above.
    #[serde(default)]
    pub accept_unknown_instances: bool,
    /// A misspelling is rejected by deserialization, which names the accepted
    /// values, rather than reaching a query that matches nothing.
    #[serde(default = "default_networks")]
    pub networks: Vec<Network>,
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
    /// Per-instance overrides, keyed by pubkey (lowercase hex; `npub1…` is
    /// accepted in the file and folded like `[indexer].instances`).
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

/// Where and how the snapshot of `docs/NOSTR-PUBLICATION.md` is published.
///
/// Separate from `[nostr]` on purpose: reading a relay and writing to it
/// are different trust decisions, and an operator who indexes from a dozen
/// relays does not thereby agree to sign events onto all twelve.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishSettings {
    /// Relays to publish to. Empty means `[nostr].relays`, which is the
    /// useful default and is filled in on load, so every reader after
    /// validation sees the real list rather than a rule.
    #[serde(default)]
    pub relays: Vec<String>,
    /// The publisher's own ceiling on a document's `content` (§9.1). A
    /// relay that advertises a smaller `limitation.max_content_length`
    /// lowers it; none raises it.
    #[serde(default = "default_max_content_bytes")]
    pub max_content_bytes: usize,
    /// Where the signing key of §12 lives: the *name* of an environment
    /// variable (`nsec = "env:BESTIARIO_PUBLISH_NSEC"`) or the path of a
    /// file holding it (`nsec = "file:/run/secrets/bestiario-nsec"`).
    ///
    /// Never the key itself — see [`SecretRef`]. It is resolved when a run
    /// actually signs, so a `stats` invocation on a machine that publishes
    /// nothing neither needs it nor fails without it.
    #[serde(default)]
    pub nsec: Option<SecretRef>,
}

fn default_max_content_bytes() -> usize {
    bestiario_stats::publish::size::DEFAULT_MAX_CONTENT_BYTES
}

impl Default for PublishSettings {
    fn default() -> Self {
        Self {
            relays: Vec::new(),
            max_content_bytes: default_max_content_bytes(),
            nsec: None,
        }
    }
}

fn default_resume_overlap_secs() -> u64 {
    3600
}

fn default_networks() -> Vec<Network> {
    vec![Network::Mainnet]
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

        raw.normalized()?.validated()
    }

    /// Parses TOML from memory, without the environment layer.
    pub fn from_toml_str(toml: &str) -> Result<Self, ConfigError> {
        let raw = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()?
            .try_deserialize::<Settings>()?;

        raw.normalized()?.validated()
    }

    /// Returns a copy with case-insensitive values folded to their canonical
    /// case and pubkeys folded to hex, so that later comparisons are plain
    /// string equality.
    ///
    /// Fails only when a pubkey is spelled as an `npub1…` that does not
    /// decode: there is no canonical form to fold it to, and the checksum
    /// failure is the useful message.
    fn normalized(self) -> Result<Self, ValidationError> {
        let instances = self
            .indexer
            .instances
            .iter()
            .map(|p| canonical_pubkey(p))
            .collect::<Result<Vec<_>, _>>()?;
        let dev_fee_percentage = canonical_dev_fee_overrides(&self.assumptions.dev_fee_percentage)?;

        Ok(Self {
            indexer: IndexerSettings {
                instances,
                ..self.indexer
            },
            assumptions: AssumptionSettings {
                dev_fee_percentage,
                ..self.assumptions
            },
            report: ReportSettings {
                reference_currency: self.report.reference_currency.trim().to_uppercase(),
            },
            publish: PublishSettings {
                relays: if self.publish.relays.is_empty() {
                    self.nostr.relays.clone()
                } else {
                    self.publish.relays
                },
                ..self.publish
            },
            ..self
        })
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
        self.validate_report()?;
        self.validate_publish()
    }

    fn validate_publish(&self) -> Result<(), ValidationError> {
        for url in &self.publish.relays {
            if !url.starts_with("wss://") && !url.starts_with("ws://") {
                return Err(ValidationError::PublishRelayNotWebsocket { url: url.clone() });
            }
        }
        if self.publish.max_content_bytes == 0 {
            return Err(ValidationError::PublishCeilingIsZero);
        }
        Ok(())
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
}

impl AssumptionSettings {
    /// The dev fee share to assume for `pubkey`: its override if it has one,
    /// otherwise the global default.
    pub fn dev_fee_percentage_for(&self, pubkey: &str) -> f64 {
        self.dev_fee_percentage
            .get(&pubkey.to_lowercase())
            .copied()
            .unwrap_or(self.dev_fee_percentage_default)
    }
}

/// Trims and lowercases a pubkey as written in the file, and decodes it to
/// hex if it is a NIP-19 `npub1…`. Hex is returned as is — its shape is
/// checked later by [`validate_pubkey`], so that a malformed hex string still
/// gets the hex-specific error rather than a bech32 one.
///
/// Lowercasing before decoding is sound: bech32 is case-insensitive, and a
/// mixed-case string is invalid either way.
fn canonical_pubkey(raw: &str) -> Result<String, ValidationError> {
    let value = raw.trim().to_lowercase();
    if !looks_like_bech32(&value) {
        return Ok(value);
    }
    PublicKey::from_bech32(&value)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| ValidationError::PubkeyNotNpub {
            pubkey: value,
            reason: error.to_string(),
        })
}

/// Folds the keys of `[assumptions.dev_fee_percentage]` to hex, rejecting
/// two spellings of the same instance rather than letting one silently win.
///
/// TOML already refuses a literally repeated key, so a collision can only
/// come from an alias (hex plus npub of the same pubkey), which is easy to
/// leave behind while migrating a file from one form to the other.
fn canonical_dev_fee_overrides(
    raw: &BTreeMap<String, f64>,
) -> Result<BTreeMap<String, f64>, ValidationError> {
    let mut spellings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut folded = BTreeMap::new();
    for (key, value) in raw {
        let pubkey = canonical_pubkey(key)?;
        spellings
            .entry(pubkey.clone())
            .or_default()
            .push(key.trim().to_string());
        folded.insert(pubkey, *value);
    }
    if let Some((pubkey, spellings)) = spellings.into_iter().find(|(_, s)| s.len() > 1) {
        return Err(ValidationError::DuplicateDevFeeOverride { pubkey, spellings });
    }
    Ok(folded)
}

/// Whether `value` has the `<hrp>1<data>` shape of a bech32 string with a
/// prefix that cannot be hex — `npub`, but also `nsec` or `note`, so that a
/// wrong-kind NIP-19 string is reported as such rather than as bad hex.
fn looks_like_bech32(value: &str) -> bool {
    value.split_once(BECH32_SEPARATOR).is_some_and(|(hrp, _)| {
        !hrp.is_empty()
            && hrp.chars().all(|c| c.is_ascii_lowercase())
            && !hrp.chars().all(|c| c.is_ascii_hexdigit())
    })
}

/// Checks the shape of an already-canonical (lowercase hex) pubkey.
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
