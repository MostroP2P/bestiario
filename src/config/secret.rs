//! Secrets, and the `env:NAME` references that stand in for them.
//!
//! # Why a reference and not the value
//!
//! `settings.toml` is a file operators paste into issues, copy between
//! machines and commit to a private repository. A signing key written in
//! it is a signing key in every one of those places, and no amount of
//! care at the point of use undoes that. So the file never holds the key:
//! it holds the *name of the environment variable* that does, and
//! [`EnvRef`] is the only shape `[publish].nsec` will deserialize into.
//!
//! That is a type-level guarantee rather than a rule in the
//! documentation. `Settings` cannot carry a secret, because the field
//! that would carry one cannot be built from a literal.

use std::fmt;

use serde::{Deserialize, Deserializer};

/// Prefix marking a configured string as an environment-variable
/// reference.
const ENV_PREFIX: &str = "env:";

/// Marker printed instead of a secret.
const REDACTED: &str = "[redacted]";

/// The name of an environment variable holding a secret, written in the
/// file as `"env:BESTIARIO_PUBLISH_NSEC"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRef(String);

impl EnvRef {
    /// The variable's name — safe to print, and the only useful thing to
    /// say when it turns out not to be set.
    pub fn name(&self) -> &str {
        &self.0
    }

    /// The value the variable holds, through a lookup of the caller's
    /// choosing.
    ///
    /// Taking the lookup rather than reading the process environment
    /// directly is what makes the resolution testable without a test
    /// mutating the environment of every other test running beside it.
    pub fn read<F>(&self, lookup: F) -> Option<Secret>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        lookup(&self.0).map(Secret)
    }

    /// The value the variable holds in this process.
    pub fn read_env(&self) -> Option<Secret> {
        self.read(|name| std::env::var(name).ok())
    }
}

/// Refuses anything that is not an `env:NAME` reference, so a key pasted
/// into the file is a configuration error and never a working setup that
/// happens to leak.
impl<'de> Deserialize<'de> for EnvRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let name = raw.strip_prefix(ENV_PREFIX).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "expected an environment-variable reference of the form \
                 `{ENV_PREFIX}NAME`, so that the secret itself is not written \
                 into the configuration file"
            ))
        })?;
        if name.trim().is_empty() {
            return Err(serde::de::Error::custom(format!(
                "`{ENV_PREFIX}` names no variable"
            )));
        }
        Ok(Self(name.trim().to_string()))
    }
}

/// A secret value, read from the environment.
///
/// `Debug` prints a marker: a secret that reaches a log line reaches
/// every place that log line is pasted. [`Secret::expose`] is the one way
/// to the value, so every exposure is greppable at its call site.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests;
